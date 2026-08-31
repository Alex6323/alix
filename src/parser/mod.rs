use std::{collections::HashSet, ops::Range, path::PathBuf, sync::Arc};

use thiserror::Error;

use crate::{
    answer::Input,
    card::{Badge, Card, CardImage, Direction, Note},
    choice,
    depth::Reveal,
    token,
};

mod canonical;
pub(crate) mod checklist;
mod cloze;
mod frontmatter;
mod mathspan;
mod normalize;
pub mod region;
mod sidecar;
mod stream;

pub use canonical::{canonical_content, content_fingerprint, mix_fingerprint};
pub use cloze::{BLANK, HIDDEN};
use cloze::{Seg, scan_markers, seg_display};
pub(crate) use frontmatter::is_frontmatter_fence;
pub use frontmatter::{
    Frontmatter, Mapping, PERSONAL_PARENT_KEY, Reorder, parse_sampling, reorder_frontmatter,
    yaml_quote,
};
use frontmatter::{MappableBlock, bad_value, parse_frontmatter, parse_reveal};
pub use normalize::normalize;
pub use sidecar::{SidecarNote, notes, without_notes};

// Deliberately not Unicode whitespace; anything outside this set is content.
pub(crate) const WHITESPACE: [char; 6] = ['\t', '\n', '\x0B', '\x0C', '\r', ' '];

const ESCAPABLE: [&str; 6] = ["#", ">", "---", "<!--", "```", "~~~"];

pub type LineSpan = (usize, usize);

#[derive(Debug)]
pub struct ParsedDeck {
    pub deck_token: Option<String>,
    pub title: Option<String>,
    pub frontmatter: Frontmatter,
    pub cards: Vec<Card>,
    /// Deck-wide link-definition labels, in document order, unfolded:
    /// reference-link matching folds at lookup, not at collection.
    pub definitions: Vec<String>,
    pub lints: Vec<Lint>,
    pub frontmatter_span: Option<LineSpan>,
    pub tables: Vec<TableStamping>,
}

/// A card table's identity surface, so `stamp` can mint what is missing
/// without re-deriving the table grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableStamping {
    /// 1-based header line: the block boundary a preceding card ends at.
    pub line: usize,
    pub rows: Vec<TableRowStamping>,
    pub token: Option<String>,
    /// 1-based last line of the block (rows and trailing directive
    /// comments): the container id line splices after it.
    pub end_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRowStamping {
    pub line: usize,
    pub stamp: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReference {
    pub source: String,
    pub destination: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    pub line: usize,
    pub kind: LintKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintKind {
    UnknownKey {
        key: String,
    },
    BadValue {
        key: String,
        value: String,
    },
    EmptyValue {
        key: String,
    },
    IndentedH2,
    UnclosedComment,
    UnclosedFence,
    ImageMalformed,
    ChoiceAnswerMixed,
    ChoiceNeedsBothSides,
    DuplicateChoiceOption,
    ChoiceMultiCorrectUnsupported,
    ChoiceNoteNamesPosition,
    /// A comment that is not alix machinery. A deck's `<!-- -->` is alix
    /// vocabulary, so anything else in one is ignored, which is silent
    /// exactly when the author meant it to do something.
    UnrecognizedComment,
    /// A blockquote opening with a badge-shaped first line that is not one
    /// of the five: it stays a quote, which is a silent meaning shift.
    BadgeShape {
        text: String,
    },
    /// A badged note with no body lines: it renders nothing.
    EmptyNote,
    UntypableSpan {
        answer: String,
    },
    /// A block note that spells out one blank's answer, which the block's
    /// other cards also show. `blank` is 1-based, in author order.
    NoteContainsBlankAnswer {
        blank: usize,
        answer: String,
    },
    NoteNamesNoGroup {
        name: String,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("line {0}: frontmatter never closes (missing the terminating `---`)")]
    UnclosedFrontmatter(usize),
    #[error("line {line}: frontmatter is not valid yaml: {message}")]
    FrontmatterSyntax { line: usize, message: String },
    #[error("line {line}: `id:` must be a quoted string (`id: \"deck-<token>\"`), got {found}")]
    NonStringId { line: usize, found: &'static str },
    #[error("line {line}: `id:` must hold a `deck-<token>` id, got `{value}`")]
    InvalidDeckId { line: usize, value: String },
    #[error("line {line}: a card `<!-- id: -->` must hold a base `card-<token>` id, got `{value}`")]
    InvalidCardId { line: usize, value: String },
    #[error(
        "line {line}: deck format version {version} is not supported; this deck was written by a newer alix, so upgrade alix rather than editing the deck"
    )]
    UnsupportedDeckVersion { line: usize, version: i64 },
    #[error("line {line}: `format-version:` must be an integer, got {found}")]
    NonIntegerVersion { line: usize, found: &'static str },
    #[error("line {line}: control character {found} outside the whitespace set")]
    ControlChar { line: usize, found: String },
    #[error("line {0}: `⍰` and `⬚` are the mask markers and cannot appear in authored text")]
    ReservedMarker(usize),
    #[error("line {line}: `title:` must be a non-empty single line, found {value:?}")]
    InvalidTitle { line: usize, value: String },
    #[error("line {0}: card front is empty; write the question on the heading line")]
    EmptyFront(usize),
    #[error(
        "line {0}: card front without an answer; a card heading asks a question, so give it at least one answer line below"
    )]
    FrontWithoutAnswer(usize),
    #[error(
        "line {0}: this sub-card has no open parent one level above it; add that parent heading first, or remove `#`s until the depth one above is open"
    )]
    OrphanSubCard(usize),
    #[error(
        "line {0}: a deck body starts with a heading; open a `# ` section or `## ` card above this line"
    )]
    ProseBeforeFirstHeading(usize),
    #[error(
        "line {0}: a bare `#` sits attached inside a card, where a context reset has no meaning; leave a blank line above it to reset the section, or write the section title after `# `"
    )]
    ContextResetInCard(usize),
    #[error("line {0}: a section heading takes no directives and no card id")]
    SectionDirective(usize),
    #[error("line {0}: this card id belongs to no card")]
    OrphanCardId(usize),
    #[error("line {0}: only a `## ` heading can title a card table")]
    SubCardTableTitle(usize),
    #[error(
        "line {line}: `<!-- {word} -->` sits above the block it maps; comment machinery trails: move it to the line directly below the block"
    )]
    LeadingInvocation { line: usize, word: String },
    #[error(
        "line {line}: `<!-- {key}: ... -->` sits above card content; comment machinery trails: move it below the card's last content line"
    )]
    LeadingDirective { line: usize, key: String },
    #[error(
        "line {line}: the note opened here trails the card, and answer content follows it; comment machinery trails: move the content above the note"
    )]
    ContentAfterNote { line: usize },
    #[error(
        "line {line}: a card heading line takes no `<!-- {key}: ... -->`; comment machinery trails: move it below the card's content"
    )]
    FrontDirective { line: usize, key: String },
    #[error(
        "line {line}: `at:` is not a named-field locator (`at: <src>:<lines> fingerprint: xxh64-<hex> asset: <object>`): {message}; fields are `at:`, `fingerprint:`, `asset:`, in that order"
    )]
    InvalidLocator { line: usize, message: String },
    #[error("line {line}: {message}")]
    InvalidRegion { line: usize, message: String },
    #[error("line {line}: {message}")]
    ChoiceShape { line: usize, message: String },
    #[error(
        "line {0}: an image shares its line with prose; give the image its own line (inline images are a roadmap item, not silently torn from the sentence)"
    )]
    MixedImageLine(usize),
    #[error("line {0}: a table line must start and end with `|`; add the missing outer pipe")]
    TableLineMalformed(usize),
    #[error(
        "line {line}: a card table has 2 or 3 columns (front | back | note), this line has {found}; merge or split cells, or write the row as a `##` card"
    )]
    TableColumns { line: usize, found: usize },
    #[error(
        "line {line}: this table line has {found} cells but the header has {expected}; pad with empty `|` cells or drop the extras"
    )]
    TableRowWidth {
        line: usize,
        found: usize,
        expected: usize,
    },
    #[error(
        "line {line}: this delimiter row has {found} cells but the header has {expected}; give every header column its own `---` cell"
    )]
    TableDelimiterWidth {
        line: usize,
        found: usize,
        expected: usize,
    },
    #[error("line {0}: an image in a table cell is not supported; write that row as a `##` card")]
    TableCellImage(usize),
    #[error("line {line}: row stamp `{value}` is not 6 base32 chars")]
    TableRowStamp { line: usize, value: String },
    #[error("line {line}: row stamp `{value}` appears twice in one table")]
    TableDuplicateStamp { line: usize, value: String },
    #[error("line {line}: this card already carries an `id:`; one card holds one identity")]
    DuplicateCardId { line: usize },
    #[error("line {0}: only directive comments may follow a card table before the next `## ` card")]
    TableTrailing(usize),
    #[error(
        "line {0}: a thematic break line only divides a card's front from the answer attached below it; delete the line, or put `<!-- plain -->` below it to keep it literal"
    )]
    StrayDivider(usize),
    #[error(
        "line {0}: a `===` underline heading is not supported; write the heading as its own `#`-prefixed line instead"
    )]
    SetextUnderline(usize),
    #[error(
        "line {0}: this `$$` opens display math but never closes before the card ends; add a closing `$$` line, or write a one-line formula as `$$formula$$`"
    )]
    UnclosedDisplayMath(usize),
    #[error(
        "line {line}: `<` followed by a letter (column {column}) is reserved for HTML, which alix does not render; put literal markup in backticks, or escape the bracket as `\\<`"
    )]
    TagShape { line: usize, column: usize },
    #[error(
        "line {0}: four-space indentation opens a Markdown code block, which alix does not support; wrap the code in a ``` fence instead"
    )]
    IndentedCode(usize),
    #[error(
        "line {0}: quotes do not nest; keep the note one `>` deep, or put literal `>` text in a ``` fence"
    )]
    NestedQuote(usize),
}

impl ParseError {
    pub(crate) fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::UnclosedFrontmatter(_) => "unclosed_frontmatter",
            Self::FrontmatterSyntax { .. } => "frontmatter_syntax",
            Self::NonStringId { .. } => "non_string_id",
            Self::InvalidDeckId { .. } => "invalid_deck_id",
            Self::InvalidCardId { .. } => "invalid_card_id",
            Self::UnsupportedDeckVersion { .. } => "unsupported_deck_version",
            Self::NonIntegerVersion { .. } => "non_integer_version",
            Self::ControlChar { .. } => "control_character",
            Self::ReservedMarker(_) => "reserved_marker",
            Self::InvalidTitle { .. } => "invalid_title",
            Self::EmptyFront(_) => "empty_front",
            Self::FrontWithoutAnswer(_) => "front_without_answer",
            Self::OrphanSubCard(_) => "orphan_sub_card",
            Self::ProseBeforeFirstHeading(_) => "prose_before_first_heading",
            Self::ContextResetInCard(_) => "context_reset_in_card",
            Self::SectionDirective(_) => "section_directive",
            Self::OrphanCardId(_) => "orphan_card_id",
            Self::SubCardTableTitle(_) => "sub_card_table_title",
            Self::LeadingInvocation { .. } => "leading_invocation",
            Self::LeadingDirective { .. } => "leading_directive",
            Self::ContentAfterNote { .. } => "content_after_note",
            Self::FrontDirective { .. } => "front_directive",
            Self::InvalidLocator { .. } => "invalid_locator",
            Self::InvalidRegion { .. } => "invalid_region",
            Self::ChoiceShape { .. } => "choice_shape",
            Self::MixedImageLine(_) => "mixed_image_line",
            Self::TableLineMalformed(_) => "table_line_malformed",
            Self::TableColumns { .. } => "table_columns",
            Self::TableRowWidth { .. } => "table_row_width",
            Self::TableDelimiterWidth { .. } => "table_delimiter_width",
            Self::TableCellImage(_) => "table_cell_image",
            Self::TableRowStamp { .. } => "table_row_stamp",
            Self::TableDuplicateStamp { .. } => "table_duplicate_stamp",
            Self::DuplicateCardId { .. } => "duplicate_card_id",
            Self::TableTrailing(_) => "table_trailing",
            Self::StrayDivider(_) => "stray_divider",
            Self::SetextUnderline(_) => "setext_underline",
            Self::UnclosedDisplayMath(_) => "unclosed_display_math",
            Self::TagShape { .. } => "tag_shape",
            Self::IndentedCode(_) => "indented_code",
            Self::NestedQuote(_) => "nested_quote",
        }
    }

    pub(crate) fn line(&self) -> usize {
        match self {
            Self::UnclosedFrontmatter(line)
            | Self::EmptyFront(line)
            | Self::FrontWithoutAnswer(line)
            | Self::OrphanSubCard(line)
            | Self::ProseBeforeFirstHeading(line)
            | Self::ContextResetInCard(line)
            | Self::SectionDirective(line)
            | Self::OrphanCardId(line)
            | Self::SubCardTableTitle(line)
            | Self::MixedImageLine(line)
            | Self::ReservedMarker(line)
            | Self::TableLineMalformed(line)
            | Self::TableCellImage(line)
            | Self::TableTrailing(line)
            | Self::StrayDivider(line)
            | Self::SetextUnderline(line)
            | Self::UnclosedDisplayMath(line)
            | Self::IndentedCode(line)
            | Self::NestedQuote(line) => *line,
            Self::InvalidTitle { line, .. }
            | Self::FrontmatterSyntax { line, .. }
            | Self::NonStringId { line, .. }
            | Self::InvalidDeckId { line, .. }
            | Self::InvalidCardId { line, .. }
            | Self::UnsupportedDeckVersion { line, .. }
            | Self::NonIntegerVersion { line, .. }
            | Self::ControlChar { line, .. }
            | Self::InvalidLocator { line, .. }
            | Self::InvalidRegion { line, .. }
            | Self::ChoiceShape { line, .. }
            | Self::TableColumns { line, .. }
            | Self::TableRowWidth { line, .. }
            | Self::TagShape { line, .. }
            | Self::TableDelimiterWidth { line, .. }
            | Self::TableRowStamp { line, .. }
            | Self::LeadingInvocation { line, .. }
            | Self::LeadingDirective { line, .. }
            | Self::ContentAfterNote { line }
            | Self::FrontDirective { line, .. }
            | Self::TableDuplicateStamp { line, .. }
            | Self::DuplicateCardId { line } => *line,
        }
    }
}

pub fn parse(subject: &str, text: &str) -> Result<ParsedDeck, ParseError> {
    let document = parse_document(text)?;
    // Zero `## ` fronts is a valid, loadable zero-card deck, not a parse error.
    let subject: Arc<str> = Arc::from(subject);
    let deck_id: Arc<str> = Arc::from(document.frontmatter.id.as_deref().unwrap_or(""));
    let mut lints = document.lints;
    let mut cards = Vec::new();
    let mut tables = Vec::new();
    for block in document.blocks {
        match block {
            RawBlock::Card(raw) => {
                let block_start = cards.len();
                let prose = build_card(
                    &subject,
                    &deck_id,
                    raw,
                    document.frontmatter.tasklist,
                    &mut cards,
                    &mut lints,
                )?;
                build_region_cards(block_start, &mut cards, prose.as_ref(), &mut lints)?;
            }
            RawBlock::Table(raw) => {
                tables.push(TableStamping {
                    line: raw.line,
                    rows: raw
                        .rows
                        .iter()
                        .map(|row| TableRowStamping {
                            line: row.line,
                            stamp: row.stamp.clone(),
                        })
                        .collect(),
                    token: raw.directives.token.clone(),
                    end_line: raw.end_line,
                });
                build_table_cards(&subject, &deck_id, raw, &mut cards)?;
            }
        }
    }
    let table = std::sync::Arc::new(crate::inline::LinkDefinitions::new(&document.definitions));
    for card in &mut cards {
        card.definitions = table.clone();
    }
    Ok(ParsedDeck {
        deck_token: document.frontmatter.id.clone(),
        title: document.frontmatter.title.clone(),
        frontmatter: document.frontmatter,
        cards,
        definitions: document.definitions,
        lints,
        frontmatter_span: document.frontmatter_span,
        tables,
    })
}

pub fn parse_str(subject: &str, text: &str) -> Result<Vec<Card>, ParseError> {
    Ok(parse(subject, text)?.cards)
}

/// A personal sidecar, whose hierarchy is deliberately flat (D16): a
/// reader labels notes with whatever depth the source card used, and that
/// label must stay content rather than opening a sub-card that steals the
/// stamped card's answer or orphans the whole file.
pub fn parse_sidecar(subject: &str, text: &str) -> Result<Vec<Card>, ParseError> {
    SIDECAR_MODE.with(|flat| flat.set(true));
    let out = parse(subject, text).map(|deck| deck.cards);
    SIDECAR_MODE.with(|flat| flat.set(false));
    out
}

thread_local! {
    static SIDECAR_MODE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn sidecar_mode() -> bool {
    SIDECAR_MODE.with(std::cell::Cell::get)
}

pub fn card_front_lines(text: &str) -> Result<Vec<usize>, ParseError> {
    let mut lines = Vec::new();
    for card in parse("deck.md", text)?.cards {
        if lines.last() != Some(&card.line) {
            lines.push(card.line);
        }
    }
    Ok(lines)
}

fn opens_frontmatter(text: &str) -> bool {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .find(|line| !trim_ws(line).is_empty())
        .is_some_and(is_frontmatter_fence)
}

pub fn is_deck_content(text: &str) -> bool {
    match parse("deck.md", text) {
        Ok(deck) => !deck.cards.is_empty() || deck.frontmatter_span.is_some(),
        // An ordinary document may sit in a decks folder untouched, and one
        // that opens with prose trips the body-starts-with-a-heading rule
        // without ever claiming to be a deck. Frontmatter is the claim.
        Err(ParseError::ProseBeforeFirstHeading(_)) => opens_frontmatter(text),
        // Any other parse failure counts as deck content: a broken deck
        // should surface to doctor rather than silently vanish.
        Err(_) => true,
    }
}

pub fn deck_identity(text: &str) -> Result<Option<String>, ParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = prepare(text)?;
    let mut lints = Vec::new();
    let (frontmatter, _, _) = parse_frontmatter(&lines, &mut lints)?;
    Ok(frontmatter.id)
}

/// The deck a personal file names as its parent, when it names one.
pub fn personal_parent(text: &str) -> Result<Option<String>, ParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = prepare(text)?;
    let mut lints = Vec::new();
    let (frontmatter, _, _) = parse_frontmatter(&lines, &mut lints)?;
    Ok(frontmatter.personal_for)
}

pub fn image_references(text: &str) -> Vec<ImageReference> {
    let mut references = Vec::new();
    let mut offset = 0;
    let mut frontmatter = false;
    let mut saw_content = false;

    for segment in text.split_inclusive('\n') {
        let line = segment.strip_suffix('\n').unwrap_or(segment);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if !saw_content && !trim_ws(line).is_empty() {
            saw_content = true;
            if is_frontmatter_fence(line) {
                frontmatter = true;
                offset += segment.len();
                continue;
            }
        }
        if frontmatter {
            if is_frontmatter_fence(line) {
                frontmatter = false;
            }
            offset += segment.len();
            continue;
        }
        image_references_in_line(line, offset, &mut references);
        offset += segment.len();
    }
    references
}

fn image_references_in_line(line: &str, offset: usize, out: &mut Vec<ImageReference>) {
    let mut cursor = 0;
    while let Some(relative) = line[cursor..].find("![") {
        let marker = cursor + relative;
        if escaped_marker(line, marker) {
            cursor = marker + 2;
            continue;
        }
        let alt_start = marker + 2;
        let Some(alt_end_relative) = line[alt_start..].find(']') else {
            break;
        };
        let alt_end = alt_start + alt_end_relative;
        let Some(paren) = line
            .get(alt_end + 1..)
            .and_then(|tail| tail.strip_prefix('('))
        else {
            cursor = alt_end.saturating_add(1);
            continue;
        };
        let Some((source, after)) = cloze::scan_src(paren) else {
            cursor = alt_end.saturating_add(1);
            continue;
        };
        let consumed = paren.len() - after.len();
        let paren_start = alt_end + 2;
        let destination = if paren.starts_with('<') {
            paren_start + 1..paren_start + consumed.saturating_sub(2)
        } else {
            paren_start..paren_start + consumed.saturating_sub(1)
        };
        if line.get(destination.clone()).is_none() {
            cursor = paren_start + consumed;
            continue;
        }
        out.push(ImageReference {
            source,
            destination: offset + destination.start..offset + destination.end,
        });
        cursor = paren_start + consumed;
    }
}

/// Does a section heading's tail carry a RECOGNIZED card directive? Asks
/// `apply_directive` itself rather than keeping a second list that can drift
/// (that list already omitted `diagram` once): a key is recognized when
/// applying it raises no unknown-key lint. It applies a throwaway probe
/// value so the answer is about the KEY: an invalid value only lints, and a
/// lint would let the comment be stripped and the setting silently lost.
fn section_carries_directive(rest: &str, line: usize) -> bool {
    let mut tail = rest;
    while let Some(open) = tail.find("<!--") {
        let after = &tail[open + 4..];
        let Some(close) = after.find("-->") else {
            return false;
        };
        if let Some((key, _)) = directive(&after[..close]) {
            let mut probe = CardDirectives::default();
            let mut probe_lints = Vec::new();
            let recognized =
                match apply_directive(&mut probe, &key, "x".to_string(), line, &mut probe_lints) {
                    Err(_) => true,
                    Ok(()) => !probe_lints.iter().any(
                        |lint| matches!(&lint.kind, LintKind::UnknownKey { key: k } if *k == key),
                    ),
                };
            if recognized {
                return true;
            }
        }
        tail = &after[close + 3..];
    }
    false
}

/// Depth 1 is a section; 2 through 6 are card depths (a card and four
/// sub-card levels). Every consumer asks this instead of spelling the
/// range: two depth widenings each left range literals behind.
pub(crate) fn is_card_depth(depth: usize) -> bool {
    (2..=6).contains(&depth)
}

pub(crate) fn heading_depth(raw: &str) -> Option<(usize, &str)> {
    let hashes = raw.len() - raw.trim_start_matches('#').len();
    // ATX stops at six: seven or more hashes are ordinary prose (CommonMark).
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &raw[hashes..];
    if rest.is_empty() {
        return Some((hashes, ""));
    }
    rest.strip_prefix(' ').map(|rest| (hashes, rest))
}

fn escaped_marker(line: &str, marker: usize) -> bool {
    let slashes = line[..marker]
        .chars()
        .rev()
        .take_while(|character| *character == '\\')
        .count();
    slashes % 2 == 1
}

// ── Internal representation ──

struct Document {
    frontmatter: Frontmatter,
    blocks: Vec<RawBlock>,
    definitions: Vec<String>,
    lints: Vec<Lint>,
    frontmatter_span: Option<LineSpan>,
}

enum RawBlock {
    Card(RawCard),
    Table(RawTable),
}

struct RawCard {
    line: usize,
    section: Vec<String>,
    /// 2 for a `## ` card, 3 or 4 for a sub-card. Gating reads it; identity
    /// never does, so re-heading a card keeps its token.
    depth: usize,
    /// The block line of the enclosing `## `/`### ` block, when this is a
    /// sub-card.
    parent: Option<usize>,
    front: String,
    front_extra: Vec<(usize, String)>,
    back: Vec<(usize, String)>,
    divided: bool,
    /// The `---` line's number when divided: the side boundary for region
    /// binding, which first-answer-content would misplace for a directive
    /// sitting between the divider and the first content line.
    divider_line: Option<usize>,
    notes: Vec<Note>,
    directives: CardDirectives,
    mapping: Option<Mapping>,
    /// What opened this card's trailing machinery run; content arriving
    /// afterward proves the run sat above content.
    machinery: Option<TrailingStart>,
    /// The line of a bare `$$` opener still unclosed; a card may not end
    /// with one open.
    open_math: Option<usize>,
    /// The same rule for the note stream: a `> $$` opener, tracked on the
    /// stripped note text.
    open_note_math: Option<usize>,
    /// The badge of the note run being scanned, until a body line opens the
    /// note it belongs to.
    pending_badge: Option<Badge>,
}

/// What opened a card's trailing machinery run, and where.
enum TrailingStart {
    Directive { line: usize, key: String },
    Note { line: usize },
}

/// What the blockquote run currently being scanned turned out to be.
#[derive(Clone, Copy, PartialEq, Eq)]
enum QuoteRun {
    Note,
    Quote,
}

impl RawCard {
    fn machinery_stays_trailing(&self) -> Result<(), ParseError> {
        match &self.machinery {
            Some(TrailingStart::Directive { line, key }) => Err(ParseError::LeadingDirective {
                line: *line,
                key: key.clone(),
            }),
            Some(TrailingStart::Note { line }) => Err(ParseError::ContentAfterNote { line: *line }),
            None => Ok(()),
        }
    }
}

struct RawTable {
    line: usize,
    section: Vec<String>,
    title: Option<String>,
    columns: usize,
    rows: Vec<RawRow>,
    directives: CardDirectives,
    rows_done: bool,
    end_line: usize,
    /// The one mapping comment that selected this table; every other
    /// mapping in its trailing zone is a second invocation of one block.
    invocation_line: Option<usize>,
}

struct RawRow {
    line: usize,
    cells: Vec<String>,
    stamp: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
struct CardDirectives {
    regions: Vec<region::RawRegion>,
    crops: Vec<region::RawCrop>,
    token: Option<String>,
    sampling: Option<bool>,
    reveal: Option<Reveal>,
    reveal_line: Option<usize>,
    input: Option<Input>,
    direction: Option<Direction>,
    citations: Vec<crate::card::SourceCitation>,
    diagrams: Vec<crate::card::DiagramStamp>,
    givens: Vec<String>,
}

/// `fingerprint: xxh64-<16 hex> asset: <object>.png geometry: <object>.json`,
/// exactly these three fields in order; anything else is invalid input.
fn parse_diagram_stamp(value: &str, line: usize) -> Result<crate::card::DiagramStamp, String> {
    let mut rest = value.trim();
    let mut field = |key: &str| -> Result<String, String> {
        rest = rest
            .strip_prefix(key)
            .ok_or_else(|| format!("`diagram:` needs `{key}` here, got `{rest}`"))?
            .trim_start();
        let (value, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let value = value.to_string();
        rest = tail.trim_start();
        Ok(value)
    };
    let fingerprint = field("fingerprint:")?;
    let hex = fingerprint.strip_prefix("xxh64-").unwrap_or_default();
    if hex.len() != 16 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "fingerprint `{fingerprint}` is not `xxh64-` plus 16 hex digits"
        ));
    }
    let asset = field("asset:")?;
    let geometry = field("geometry:")?;
    for (name, extension) in [(&asset, ".png"), (&geometry, ".json")] {
        if !crate::assets::is_object_name(name) {
            return Err(format!("`{name}` is not a content-addressed object name"));
        }
        if !name.ends_with(extension) {
            return Err(format!(
                "`{name}` is not a `{extension}` object for its role"
            ));
        }
    }
    if !rest.is_empty() {
        return Err(format!("unexpected trailing `{rest}`"));
    }
    Ok(crate::card::DiagramStamp {
        fingerprint,
        asset,
        geometry,
        line,
    })
}

/// Exactly the recognition `parse_document` applies to a `diagram:` stamp
/// line. The freeze scanner must agree with the parser in BOTH directions:
/// a looser scanner rule lets a malformed near-stamp with a current
/// fingerprint silently suppress freezing, a stricter one re-freezes valid
/// decks. One function, one language.
pub(crate) fn diagram_stamp_on_line(line: &str) -> Option<crate::card::DiagramStamp> {
    let trimmed = trim_ws(line);
    let body = trimmed.strip_prefix("<!--")?.strip_suffix("-->")?;
    let (key, value) = directive(body)?;
    (key == "diagram")
        .then(|| parse_diagram_stamp(&value, 0).ok())
        .flatten()
}

fn parse_document(text: &str) -> Result<Document, ParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = prepare(text)?;
    let mut lints = Vec::new();
    let (frontmatter, body_start, frontmatter_span) = parse_frontmatter(&lines, &mut lints)?;
    let table_default = frontmatter.table == Some(Mapping::Cards);
    let body = scan(&lines, body_start, table_default, &mut lints)?;
    Ok(Document {
        frontmatter,
        blocks: body.blocks,
        definitions: body.definitions,
        lints,
        frontmatter_span,
    })
}

fn prepare(text: &str) -> Result<Vec<&str>, ParseError> {
    let mut lines = Vec::new();
    for (idx, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(ch) = line
            .chars()
            .find(|c| matches!(*c as u32, 0x00..=0x1f) && !WHITESPACE.contains(c))
        {
            return Err(ParseError::ControlChar {
                line: idx + 1,
                found: format!("U+{:04X}", ch as u32),
            });
        }
        if line.contains(cloze::BLANK) || line.contains(cloze::HIDDEN) {
            return Err(ParseError::ReservedMarker(idx + 1));
        }
        lines.push(line);
    }
    Ok(lines)
}

/// Trims over the closed whitespace set only, never Unicode whitespace.
fn trim_ws(s: &str) -> &str {
    s.trim_matches(&WHITESPACE[..])
}

pub(crate) fn collapse(s: &str) -> String {
    s.split(&WHITESPACE[..])
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn fence_opener(line: &str) -> Option<(char, usize)> {
    let ch = if line.starts_with("```") {
        '`'
    } else if line.starts_with("~~~") {
        '~'
    } else {
        return None;
    };
    Some((ch, line.chars().take_while(|c| *c == ch).count()))
}

pub(crate) fn closes_fence(line: &str, ch: char, open: usize) -> bool {
    let run = line.chars().take_while(|c| *c == ch).count();
    run >= open && line.chars().skip(run).all(|c| WHITESPACE.contains(&c))
}

// ── The line scanner ──

struct ScannedBody {
    blocks: Vec<RawBlock>,
    definitions: Vec<String>,
}

/// A deck body opens with a heading. Before the first one there is no
/// section to join, so a line that would join it is the error instead.
fn section_line(
    section: &mut Vec<String>,
    seen_heading: bool,
    lineno: usize,
    line: String,
) -> Result<(), ParseError> {
    // A sidecar has no sections at all (D16), so it has no body-opens-with-a-
    // heading rule either: its lines are notes and personal cards.
    if sidecar_mode() {
        return Ok(());
    }
    if !seen_heading {
        return Err(ParseError::ProseBeforeFirstHeading(lineno));
    }
    section.push(line);
    Ok(())
}

fn indent_width(raw: &str) -> usize {
    let mut width = 0;
    for c in raw.chars() {
        match c {
            ' ' => width += 1,
            '\t' => width += 4 - width % 4,
            _ => break,
        }
    }
    width
}

/// A whole line of one break marker: the only shape a break can take, and the
/// shape the importer has to neutralize when it writes a cell.
#[cfg(feature = "full")]
pub(crate) fn is_thematic_break(line: &str) -> bool {
    let t = trim_ws(line);
    thematic_break(t) && indent_width(line) < 4
}

fn thematic_break(t: &str) -> bool {
    ['-', '*', '_'].iter().any(|marker| {
        t.chars().all(|c| c == *marker || c == ' ' || c == '\t')
            && t.chars().filter(|c| c == marker).count() >= 3
    })
}

fn scan(
    lines: &[&str],
    start: usize,
    table_default: bool,
    lints: &mut Vec<Lint>,
) -> Result<ScannedBody, ParseError> {
    let mut blocks: Vec<RawBlock> = Vec::new();
    let mut definitions: Vec<String> = Vec::new();
    let mut current: Option<RawCard> = None;
    let mut table: Option<RawTable> = None;
    let mut open_depths: Vec<(usize, usize)> = Vec::new();
    let mut section: Vec<String> = Vec::new();
    let mut seen_heading = false;
    let mut skip_delimiter = false;
    let mut skip_lines = 0usize;
    let mut fence: Option<(char, usize, usize)> = None;
    let mut prev_blank = false;
    let mut prev_heading = false;
    let mut prev_prose = false;
    let mut mappable_block: Option<MappableBlock> = None;
    let mut literal_table_invocation: Option<usize> = None;
    let mut quote_run: Option<QuoteRun> = None;

    for (idx, raw) in lines.iter().enumerate().skip(start) {
        if skip_lines > 0 {
            skip_lines -= 1;
            continue;
        }
        let lineno = idx + 1;
        let raw = *raw;
        let was_prose = prev_prose;
        prev_prose = false;
        let block_above = mappable_block.take();
        let run_above = quote_run.take();

        if skip_delimiter {
            skip_delimiter = false;
            continue;
        }

        // A fence never opens while a table is active (every non-table line
        // inside a table's scope is either a flush or a loud error).
        if let Some(tbl) = table.as_mut() {
            if let Some(column) = crate::inline::tag_shape_column(raw) {
                return Err(ParseError::TagShape {
                    line: lineno,
                    column,
                });
            }
            let next = lines.get(idx + 1).copied();
            if table_line(tbl, raw, lineno, next, lints)? {
                prev_blank = trim_ws(raw).is_empty();
                prev_heading = false;
                continue;
            }
            if let Some(tbl) = table.take() {
                // A titled table with rows stands in for its `## ` heading
                // as the depth-2 parent. Anything else leaves no parent: a
                // zero-row table expands to no card, and an UNTITLED table
                // is its own block, so a sub-card after it must not attach
                // to whatever card happened to precede it.
                let parents = tbl.title.is_some() && !tbl.rows.is_empty();
                if !parents {
                    open_depths.clear();
                }
                blocks.push(RawBlock::Table(tbl));
            }
        }

        if let Some((ch, open, _)) = fence {
            if closes_fence(raw, ch, open) {
                fence = None;
            }
            if current.is_none() {
                section_line(&mut section, seen_heading, lineno, raw.to_string())?;
                prev_blank = false;
                prev_heading = false;
                continue;
            }
            push_content(&mut current, lineno, raw.to_string())?;
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        // Between a card's `$$` opener and its bare closer every line is
        // verbatim math source: no card grammar (blank, note, heading,
        // table, divider, checklist, fence) may reinterpret it. An open
        // fence still wins above, so a `$$` inside code stays code.
        if current
            .as_ref()
            .is_some_and(|card| card.open_math.is_some())
        {
            if trim_ws(raw) == "$$"
                && let Some(card) = current.as_mut()
            {
                card.open_math = None;
            }
            push_content(&mut current, lineno, raw.to_string())?;
            prev_blank = false;
            prev_heading = false;
            prev_prose = true;
            continue;
        }

        if let Some((label, consumed)) = link_definition(lines, idx) {
            definitions.push(label);
            skip_lines = consumed - 1;
            prev_blank = false;
            prev_heading = false;
            prev_prose = true;
            continue;
        }

        if let Some(column) = crate::inline::tag_shape_column(raw) {
            return Err(ParseError::TagShape {
                line: lineno,
                column,
            });
        }

        if let Some((ch, open)) = fence_opener(raw) {
            fence = Some((ch, open, lineno));
            if current.is_none() {
                section_line(&mut section, seen_heading, lineno, raw.to_string())?;
                prev_blank = false;
                prev_heading = false;
                continue;
            }
            push_content(&mut current, lineno, raw.to_string())?;
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        let table_header = raw.starts_with('|')
            && lines
                .get(idx + 1)
                .is_some_and(|next| next.starts_with('|') && is_delimiter_row(next));
        let declared = table_header
            .then(|| trailing_table_mapping(lines, idx))
            .flatten();
        if let Some((line, Mapping::Plain)) = declared {
            literal_table_invocation = Some(line);
        }
        if table_header
            && match declared {
                Some((_, Mapping::Cards)) => true,
                Some(_) => false,
                None => table_default,
            }
        {
            let next = lines[idx + 1];
            let invocation_line = declared.map(|(line, _)| line);
            // An empty-bodied heading directly above the table is its TITLE,
            // not a card; any content or note keeps it a card.
            let mut title = None;
            let mut block_line = lineno;
            let mut directives = CardDirectives::default();
            if let Some(card) = current.as_ref()
                && card.depth > 2
                && card.front_extra.is_empty()
                && card.back.is_empty()
                && card.notes.is_empty()
                && !card.divided
            {
                return Err(ParseError::SubCardTableTitle(card.line));
            }
            if let Some(card) = take_card(&mut current)? {
                if card.front_extra.is_empty()
                    && card.back.is_empty()
                    && card.notes.is_empty()
                    && !card.divided
                {
                    card.machinery_stays_trailing()?;
                    title = Some(card.front);
                    block_line = card.line;
                    directives = card.directives;
                } else {
                    blocks.push(RawBlock::Card(card));
                }
            }
            let header = split_cells(raw).ok_or(ParseError::TableLineMalformed(lineno))?;
            if !(2..=3).contains(&header.len()) {
                return Err(ParseError::TableColumns {
                    line: lineno,
                    found: header.len(),
                });
            }
            check_cells(&header, lineno)?;
            let delimiter =
                split_cells(next).expect("is_delimiter_row only passes splittable lines");
            if delimiter.len() != header.len() {
                return Err(ParseError::TableDelimiterWidth {
                    line: lineno + 1,
                    found: delimiter.len(),
                    expected: header.len(),
                });
            }
            table = Some(RawTable {
                line: block_line,
                section: section.clone(),
                title,
                columns: header.len(),
                rows: Vec::new(),
                directives,
                rows_done: false,
                end_line: lineno + 1,
                invocation_line,
            });
            skip_delimiter = true;
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        if let Some((depth, rest)) = heading_depth(raw)
            && !(sidecar_mode() && depth != 2)
        {
            if depth == 1 {
                let attached_in_card = current.is_some() && !prev_blank && !prev_heading;
                if let Some(card) = take_card(&mut current)? {
                    blocks.push(RawBlock::Card(card));
                }
                // Scanned like any other heading, so an editorial comment
                // is stripped exactly as it is on a card front; only a
                // RECOGNIZED directive or an id is an error, because a
                // section owns no card to bind one to and a swallowed id
                // would sever its card's history.
                if section_carries_directive(rest, lineno) {
                    return Err(ParseError::SectionDirective(lineno));
                }
                let (text, _) = heading(rest, lineno, lints)?;
                let resets = trim_ws(&text).is_empty();
                if resets && attached_in_card {
                    return Err(ParseError::ContextResetInCard(lineno));
                }
                open_depths.clear();
                seen_heading = true;
                section = if resets { Vec::new() } else { vec![text] };
                prev_blank = false;
                prev_heading = true;
                continue;
            }
            // The stack rule: a front at depth N closes every open chain at
            // depth >= N, then requires the next-shallower one to still be
            // open. Depth 2 always opens.
            while open_depths.last().is_some_and(|(open, _)| *open >= depth) {
                open_depths.pop();
            }
            let parent = open_depths.last().map(|(_, line)| *line);
            if depth > 2 && open_depths.last().map(|(open, _)| *open) != Some(depth - 1) {
                return Err(ParseError::OrphanSubCard(lineno));
            }
            if let Some(card) = take_card(&mut current)? {
                blocks.push(RawBlock::Card(card));
            }
            let (front, directives) = heading(rest, lineno, lints)?;
            if front.is_empty() {
                return Err(ParseError::EmptyFront(lineno));
            }
            seen_heading = true;
            open_depths.push((depth, lineno));
            current = Some(RawCard {
                line: lineno,
                section: section.clone(),
                depth,
                parent,
                front,
                front_extra: Vec::new(),
                back: Vec::new(),
                divided: false,
                divider_line: None,
                notes: Vec::new(),
                directives,
                mapping: None,
                machinery: None,
                open_math: None,
                open_note_math: None,
                pending_badge: None,
            });
            prev_blank = false;
            prev_heading = true;
            continue;
        }

        let t = trim_ws(raw);

        if t.is_empty() {
            prev_blank = true;
            prev_heading = false;
            continue;
        }

        if indent_width(raw) >= 4 && (prev_blank || prev_heading) {
            return Err(ParseError::IndentedCode(lineno));
        }
        if was_prose && indent_width(raw) < 4 && t.chars().all(|c| c == '=') {
            return Err(ParseError::SetextUnderline(lineno));
        }
        // Only ever an opener here: the open state is consumed above.
        if t == "$$"
            && let Some(card) = current.as_mut()
        {
            card.open_math = Some(lineno);
        }

        if let Some(rest) = t.strip_prefix('\\')
            && (thematic_break(rest) || ESCAPABLE.iter().any(|marker| rest.starts_with(marker)))
        {
            if current.is_none() {
                section_line(&mut section, seen_heading, lineno, rest.to_string())?;
                prev_blank = false;
                prev_heading = false;
                prev_prose = true;
                continue;
            }
            push_content(&mut current, lineno, rest.to_string())?;
            prev_blank = false;
            prev_heading = false;
            prev_prose = true;
            continue;
        }

        if thematic_break(t) && indent_width(raw) < 4 {
            let plain_trails =
                invocation_below(lines, idx + 1).is_some_and(|(_, m)| m == Mapping::Plain);
            if plain_trails {
                if current.is_none() {
                    section_line(&mut section, seen_heading, lineno, t.to_string())?;
                } else {
                    push_content(&mut current, lineno, t.to_string())?;
                }
                mappable_block = Some(MappableBlock::Divider);
                prev_blank = false;
                prev_heading = false;
                continue;
            }
            let attached = lines
                .get(idx + 1)
                .is_some_and(|line| !trim_ws(line).is_empty());
            let divides =
                current.as_ref().is_some_and(|card| !card.divided) && (prev_blank || prev_heading);
            if attached && divides {
                if let Some(card) = current.as_mut() {
                    card.machinery_stays_trailing()?;
                    card.divided = true;
                    card.divider_line = Some(lineno);
                }
            } else {
                return Err(ParseError::StrayDivider(lineno));
            }
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        if let Some(rest) = t.strip_prefix('>') {
            let text = rest.strip_prefix(' ').unwrap_or(rest);
            // Inside an open note `$$` block the text after the one note
            // marker is verbatim math source, so a leading `>` is the
            // comparison operator, not a nested quote.
            let note_math_open = current
                .as_ref()
                .is_some_and(|card| card.open_note_math.is_some());
            if text.starts_with('>') && !note_math_open {
                return Err(ParseError::NestedQuote(lineno));
            }
            match current.as_mut() {
                Some(card) => {
                    // A badge opens a note; every other blockquote is a
                    // quote, which is answer content and reveals with it.
                    let run = run_above.unwrap_or_else(|| match Badge::parse(trim_ws(text)) {
                        Some(badge) => {
                            card.pending_badge = Some(badge);
                            QuoteRun::Note
                        }
                        None => {
                            if Badge::is_misspelled(trim_ws(text)) {
                                lints.push(Lint {
                                    line: lineno,
                                    kind: LintKind::BadgeShape {
                                        text: trim_ws(text).to_string(),
                                    },
                                });
                            }
                            QuoteRun::Quote
                        }
                    });
                    quote_run = Some(run);
                    match run {
                        QuoteRun::Note => {
                            if run_above.is_none() {
                                if card.machinery.is_none() {
                                    card.machinery = Some(TrailingStart::Note { line: lineno });
                                }
                                if note_run_is_empty(lines, idx + 1) {
                                    lints.push(Lint {
                                        line: lineno,
                                        kind: LintKind::EmptyNote,
                                    });
                                }
                            } else {
                                if trim_ws(text) == "$$" {
                                    card.open_note_math = match card.open_note_math {
                                        None => Some(lineno),
                                        Some(_) => None,
                                    };
                                }
                                append_note(card, text);
                            }
                        }
                        QuoteRun::Quote => push_content(&mut current, lineno, t.to_string())?,
                    }
                }
                // A section has no card to own a note, so the line is
                // ordinary section prose either way.
                None => section_line(&mut section, seen_heading, lineno, t.to_string())?,
            }
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        if t.starts_with("<!--") {
            if let Some(body) = t.strip_prefix("<!--").and_then(|s| s.strip_suffix("-->")) {
                if let Some(mapping) = Mapping::parse(trim_ws(body)) {
                    match (mapping, current.as_mut()) {
                        (Mapping::Plain, _) if literal_table_invocation == Some(lineno) => {}
                        (Mapping::Plain, None) => {}
                        (Mapping::Cards, _) | (_, None) => {
                            return Err(ParseError::LeadingInvocation {
                                line: lineno,
                                word: trim_ws(body).to_string(),
                            });
                        }
                        (mapping, Some(card)) => {
                            if !mapping.binds(block_above) {
                                return Err(ParseError::LeadingInvocation {
                                    line: lineno,
                                    word: trim_ws(body).to_string(),
                                });
                            }
                            card.mapping = Some(mapping);
                        }
                    }
                    prev_blank = false;
                    prev_heading = false;
                    continue;
                }
                if let Some((key, value)) = directive(body) {
                    match current.as_mut() {
                        Some(card) => {
                            apply_directive(&mut card.directives, &key, value, lineno, lints)?;
                            if is_known_card_key(&key) && card.machinery.is_none() {
                                card.machinery =
                                    Some(TrailingStart::Directive { line: lineno, key });
                            }
                        }
                        // A region outside any card has nothing to bind to and
                        // would otherwise vanish silently; other directives
                        // keep their historical tolerance here.
                        // An id names a card; with none open it would be
                        // swallowed, and the card it names would silently
                        // lose its history on the next stamping pass.
                        None if key == "id" => {
                            return Err(ParseError::OrphanCardId(lineno));
                        }
                        None if matches!(key.as_str(), "blank" | "cover" | "crop") => {
                            return Err(ParseError::InvalidRegion {
                                line: lineno,
                                message: format!(
                                    "`{key}:` appears before any card, so no media element or answer block can bind it"
                                ),
                            });
                        }
                        None => {
                            if !is_known_card_key(&key) && key != "diagram" {
                                lints.push(Lint {
                                    line: lineno,
                                    kind: LintKind::UnknownKey { key },
                                });
                            }
                        }
                    }
                    if machinery_rank(t).is_some() {
                        mappable_block = block_above;
                    }
                } else if !trim_ws(body).is_empty() {
                    lints.push(Lint {
                        line: lineno,
                        kind: LintKind::UnrecognizedComment,
                    });
                }
                prev_blank = false;
                prev_heading = false;
                continue;
            }
            lints.push(Lint {
                line: lineno,
                kind: LintKind::UnclosedComment,
            });
            // The line stays content.
        }

        if t.starts_with("## ") {
            lints.push(Lint {
                line: lineno,
                kind: LintKind::IndentedH2,
            });
        }

        // Prose outside any card belongs to the section it sits in, which
        // before the first heading is the unnamed one (ruled D3). A sidecar
        // has no sections at all (D16), so nothing accumulates there and a
        // personal card stays context-free.
        if current.is_none() {
            section_line(&mut section, seen_heading, lineno, t.to_string())?;
            prev_blank = false;
            prev_heading = false;
            prev_prose = true;
            continue;
        }

        // A prose line under a task list is a lazy continuation of the same
        // block, so it neither opens a block nor ends the one above it.
        if checklist::parse_line(t).is_some() || block_above == Some(MappableBlock::Checklist) {
            mappable_block = Some(MappableBlock::Checklist);
        }
        push_content(&mut current, lineno, t.to_string())?;
        prev_blank = false;
        prev_heading = false;
        prev_prose = true;
    }

    if let Some((_, _, open_line)) = fence {
        lints.push(Lint {
            line: open_line,
            kind: LintKind::UnclosedFence,
        });
    }
    if let Some(tbl) = table.take() {
        blocks.push(RawBlock::Table(tbl));
    }
    if let Some(card) = take_card(&mut current)? {
        blocks.push(RawBlock::Card(card));
    }
    Ok(ScannedBody {
        blocks,
        definitions,
    })
}

/// A complete GFM link-reference-definition block starting at `lines[at]`:
/// its label and how many lines it consumed, taken as deck-wide metadata
/// rather than answer content. Label, destination, and title each may
/// continue on a following line, so the whole candidate is scanned as one
/// joined block. An invalid or incomplete shape consumes nothing, and any
/// hole marker inside the candidate keeps every line prose so an authored
/// blank can never be silently eaten.
fn link_definition(lines: &[&str], at: usize) -> Option<(String, usize)> {
    let first = *lines.get(at)?;
    if indent_width(first) >= 4 || !trim_ws(first).starts_with('[') {
        return None;
    }
    let mut block = vec![trim_ws(first)];
    for line in lines.iter().skip(at + 1) {
        let text = trim_ws(line);
        if text.is_empty() || interrupts_definition(text) {
            break;
        }
        block.push(text);
    }
    let chars: Vec<char> = block.join("\n").chars().collect();

    let label_close = label_end(&chars, 1)?;
    let label = collapse(&chars[1..label_close].iter().collect::<String>());
    if label.is_empty() {
        return None;
    }
    if chars.get(label_close + 1) != Some(&':') {
        return None;
    }
    let destination = skip_space_across_one_newline(&chars, label_close + 2)?;
    let destination_close = destination_end(&chars, destination)?;
    let end = title_after_destination(&chars, destination_close)
        .filter(|&close| rest_of_line_blank(&chars, close))
        .unwrap_or(destination_close);
    if end == destination_close && !rest_of_line_blank(&chars, destination_close) {
        return None;
    }

    let consumed = 1 + chars[..end].iter().filter(|&&ch| ch == '\n').count();
    let holed = lines[at..at + consumed]
        .iter()
        .any(|line| line.contains("\\blank"));
    (!holed).then_some((label, consumed))
}

/// Chars the escape at `at` covers: two when the backslash escapes ASCII
/// punctuation (the CommonMark rule), else one, leaving a literal
/// backslash's neighbour its own structural meaning.
fn escape_len(chars: &[char], at: usize) -> usize {
    match chars[at] {
        '\\' if chars
            .get(at + 1)
            .is_some_and(|ch| ch.is_ascii_punctuation()) =>
        {
            2
        }
        _ => 1,
    }
}

/// Index of the label's unescaped closing `]`; None when it never closes
/// or an unescaped `[` nests inside it.
fn label_end(chars: &[char], from: usize) -> Option<usize> {
    let mut at = from;
    while at < chars.len() {
        let step = escape_len(chars, at);
        if step == 1 {
            match chars[at] {
                '[' => return None,
                ']' => return Some(at),
                _ => {}
            }
        }
        at += step;
    }
    None
}

/// Whitespace between a definition's parts may cross at most one line
/// ending; None when it crosses more.
fn skip_space_across_one_newline(chars: &[char], from: usize) -> Option<usize> {
    let mut at = from;
    let mut endings = 0usize;
    while chars.get(at).is_some_and(|ch| WHITESPACE.contains(ch)) {
        if chars[at] == '\n' {
            endings += 1;
            if endings > 1 {
                return None;
            }
        }
        at += 1;
    }
    Some(at)
}

/// Index one past a valid GFM destination: an `<angle>` form, or a bare
/// run with no unescaped whitespace or controls and balanced parentheses.
fn destination_end(chars: &[char], from: usize) -> Option<usize> {
    if chars.get(from) == Some(&'<') {
        let mut at = from + 1;
        while at < chars.len() {
            let step = escape_len(chars, at);
            if step == 1 {
                match chars[at] {
                    '<' | '\n' => return None,
                    '>' => return Some(at + 1),
                    _ => {}
                }
            }
            at += step;
        }
        return None;
    }
    let mut depth = 0usize;
    let mut at = from;
    while at < chars.len() {
        let step = escape_len(chars, at);
        if step == 1 {
            let ch = chars[at];
            if WHITESPACE.contains(&ch) {
                break;
            }
            match ch {
                '(' => depth += 1,
                ')' => depth = depth.checked_sub(1)?,
                _ if ch.is_ascii_control() => return None,
                _ => {}
            }
        }
        at += step;
    }
    (depth == 0 && at > from).then_some(at)
}

/// Index one past a title following the destination, which must be
/// whitespace-separated from it.
fn title_after_destination(chars: &[char], destination_close: usize) -> Option<usize> {
    let start = skip_space_across_one_newline(chars, destination_close)?;
    (start > destination_close).then_some(())?;
    title_end(chars, start)
}

/// Index one past a title's closing delimiter; None when it never closes
/// or a parenthesized title nests an unescaped `(`.
fn title_end(chars: &[char], from: usize) -> Option<usize> {
    let open = *chars.get(from)?;
    let closer = match open {
        '"' | '\'' => open,
        '(' => ')',
        _ => return None,
    };
    let mut at = from + 1;
    while at < chars.len() {
        let step = escape_len(chars, at);
        if step == 1 {
            match chars[at] {
                ch if ch == closer => return Some(at + 1),
                '(' if open == '(' => return None,
                _ => {}
            }
        }
        at += step;
    }
    None
}

fn rest_of_line_blank(chars: &[char], from: usize) -> bool {
    chars[from..]
        .iter()
        .take_while(|&&ch| ch != '\n')
        .all(|ch| WHITESPACE.contains(ch))
}

/// A definition never continues into a line the deck grammar owns.
fn interrupts_definition(text: &str) -> bool {
    thematic_break(text)
        || text.starts_with('#')
        || text.starts_with('>')
        || text.starts_with('|')
        || text.starts_with("<!--")
        || text.starts_with("```")
        || text.starts_with("~~~")
        || text.starts_with("- ")
        || text.starts_with("* ")
        || text.starts_with("+ ")
        || text == "$$"
}

// ── Table blocks ──

// `Ok(true)` = the line was consumed by the table; `Ok(false)` = the table's
// scope ends here and the line must be reprocessed (a `## ` front or the
// header of a directly-adjacent table).
fn table_line(
    tbl: &mut RawTable,
    raw: &str,
    lineno: usize,
    next: Option<&str>,
    lints: &mut Vec<Lint>,
) -> Result<bool, ParseError> {
    let next_is_delimiter = next.is_some_and(|n| n.starts_with('|') && is_delimiter_row(n));
    if raw.starts_with('|') && !next_is_delimiter {
        if tbl.rows_done {
            return Err(ParseError::TableTrailing(lineno));
        }
        let (cell_text, stamp) = match extract_row_stamp(raw) {
            Some((text, value)) => {
                if !crate::token::is_valid_row(&value) {
                    return Err(ParseError::TableRowStamp {
                        line: lineno,
                        value,
                    });
                }
                if tbl
                    .rows
                    .iter()
                    .any(|row| row.stamp.as_deref() == Some(&value))
                {
                    return Err(ParseError::TableDuplicateStamp {
                        line: lineno,
                        value,
                    });
                }
                (text, Some(value))
            }
            None => (raw.to_string(), None),
        };
        let cells = split_cells(&cell_text).ok_or(ParseError::TableLineMalformed(lineno))?;
        if cells.len() != tbl.columns {
            return Err(ParseError::TableRowWidth {
                line: lineno,
                found: cells.len(),
                expected: tbl.columns,
            });
        }
        check_cells(&cells, lineno)?;
        tbl.rows.push(RawRow {
            line: lineno,
            cells,
            stamp,
        });
        tbl.end_line = lineno;
        return Ok(true);
    }
    if raw.starts_with('|') {
        return Ok(false);
    }
    let t = trim_ws(raw);
    if t.is_empty() {
        tbl.rows_done = true;
        return Ok(true);
    }
    if heading_depth(raw).is_some() {
        return Ok(false);
    }
    if thematic_break(t) && indent_width(raw) < 4 {
        return Ok(false);
    }
    if let Some(body) = t.strip_prefix("<!--").and_then(|s| s.strip_suffix("-->")) {
        if Mapping::parse(trim_ws(body)).is_some() && tbl.invocation_line != Some(lineno) {
            return Err(ParseError::LeadingInvocation {
                line: lineno,
                word: trim_ws(body).to_string(),
            });
        }
        tbl.rows_done = true;
        tbl.end_line = lineno;
        if let Some((key, value)) = directive(body) {
            apply_directive(&mut tbl.directives, &key, value, lineno, lints)?;
        }
        return Ok(true);
    }
    Err(ParseError::TableTrailing(lineno))
}

pub(crate) fn split_cells(line: &str) -> Option<Vec<String>> {
    let line = trim_ws(line);
    let mut boundaries = Vec::new();
    for (i, b) in line.bytes().enumerate() {
        if b == b'|' && !escaped_marker(line, i) {
            boundaries.push(i);
        }
    }
    if boundaries.len() < 2 || boundaries[0] != 0 || *boundaries.last()? != line.len() - 1 {
        return None;
    }
    Some(
        boundaries
            .windows(2)
            .map(|pair| trim_ws(&line[pair[0] + 1..pair[1]]).replace("\\|", "|"))
            .collect(),
    )
}

pub(crate) fn is_delimiter_row(line: &str) -> bool {
    let Some(cells) = split_cells(line) else {
        return false;
    };
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let cell = cell.strip_prefix(':').unwrap_or(cell);
            let cell = cell.strip_suffix(':').unwrap_or(cell);
            !cell.is_empty() && cell.bytes().all(|b| b == b'-')
        })
}

/// A comment's place in the canonical trailing order, or None when the line
/// is not recognized machinery: content, an editorial comment, an unknown
/// key, or an unclosed comment. The forward scanner, both lookaheads, and
/// the reorder repair share this one classification.
fn machinery_rank(content: &str) -> Option<usize> {
    let body = content
        .trim()
        .strip_prefix("<!--")?
        .strip_suffix("-->")
        .map(trim_ws)?;
    if Mapping::parse(body).is_some() {
        return Some(0);
    }
    let (key, _) = directive(body)?;
    match key.as_str() {
        "blank" | "cover" | "crop" => Some(2),
        "at" => Some(3),
        "id" => Some(4),
        _ if is_known_card_key(&key) || matches!(key.as_str(), "diagram") => Some(1),
        _ => None,
    }
}

/// The invocation a block's trailing machinery run declares, read downward
/// from the line after the block: recognized machinery is transparent, and
/// anything else ends the run with the block unbound.
fn invocation_below(lines: &[&str], from: usize) -> Option<(usize, Mapping)> {
    let mut at = from;
    while machinery_rank(lines.get(at)?).is_some() {
        let body = lines[at].trim().strip_prefix("<!--")?.strip_suffix("-->")?;
        if let Some(mapping) = Mapping::parse(trim_ws(body)) {
            return Some((at + 1, mapping));
        }
        at += 1;
    }
    None
}

/// The mapping declared in a table-shaped block's trailing comment zone,
/// looking down from its header line: rows first, then adjacent comment
/// lines (the everything-trails position). None means the zone declares
/// nothing and the deck default decides.
fn trailing_table_mapping(lines: &[&str], header_idx: usize) -> Option<(usize, Mapping)> {
    let row = |at: usize| {
        lines
            .get(at)
            .is_some_and(|line| line.trim_end().starts_with('|'))
    };
    let mut idx = header_idx;
    while row(idx) {
        // A row followed by a delimiter opens the NEXT table: this block's
        // rows end above it, so its trailing zone is not ours to read.
        if idx > header_idx
            && lines
                .get(idx + 1)
                .is_some_and(|next| is_delimiter_row(next))
        {
            return None;
        }
        idx += 1;
    }
    invocation_below(lines, idx)
}

/// The opt-in doctor repair's half of spec choice 2 (liberal read,
/// canonical write): reorder each contiguous run of recognized machinery
/// comments into the canonical trailing order, the id last. Anything else
/// (content, an editorial comment, a fence line, an unknown key) bounds a
/// run, so the repair can never move a comment across content it might
/// change the meaning of.
pub fn reorder_card_comments(text: &str) -> Reorder {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<(char, usize)> = None;
    let mut run: Vec<(usize, &str)> = Vec::new();
    let flush = |out: &mut String, run: &mut Vec<(usize, &str)>| {
        run.sort_by_key(|(rank, _)| *rank);
        for (_, line) in run.drain(..) {
            out.push_str(line);
        }
    };
    for line in lines {
        let content = line.trim_end_matches(['\n', '\r']);
        if let Some((ch, open)) = fence {
            if closes_fence(content, ch, open) {
                fence = None;
            }
            flush(&mut out, &mut run);
            out.push_str(line);
            continue;
        }
        if let Some(opened) = fence_opener(content) {
            fence = Some(opened);
            flush(&mut out, &mut run);
            out.push_str(line);
            continue;
        }
        match machinery_rank(content) {
            Some(rank) => run.push((rank, line)),
            None => {
                flush(&mut out, &mut run);
                out.push_str(line);
            }
        }
    }
    flush(&mut out, &mut run);
    if out == text {
        Reorder::Unchanged
    } else {
        Reorder::Reordered(out)
    }
}

fn check_cells(cells: &[String], lineno: usize) -> Result<(), ParseError> {
    for cell in cells {
        let mut cursor = 0;
        while let Some(relative) = cell[cursor..].find("![") {
            let marker = cursor + relative;
            if !escaped_marker(cell, marker) {
                return Err(ParseError::TableCellImage(lineno));
            }
            cursor = marker + 2;
        }
    }
    Ok(())
}

// `(line without the stamp, stamp value)` when the row line's tail after
// the closing pipe is a `<!-- r:... -->` comment.
fn extract_row_stamp(line: &str) -> Option<(String, String)> {
    let trimmed = trim_ws(line);
    let prefix = trimmed.strip_suffix("-->")?;
    let start = prefix.rfind("<!--")?;
    let (key, value) = directive(&prefix[start + 4..])?;
    (key == "r").then(|| (trim_ws(&prefix[..start]).to_string(), value))
}

/// One span's bound authored location, carried from binding to masking so
/// both speak the same occurrence (the canonical stream resolved it once).
struct SpanSplice {
    directive_line: usize,
    cover: bool,
    math: bool,
    answer_index: usize,
    range: (usize, usize),
}

/// The prose a blank-bearing block hands its region cards: raw answer lines
/// (None where a line is image-only and rides the media list instead) plus
/// every span's bound splice.
struct BlockProse {
    lines: Vec<Option<String>>,
    splices: Vec<SpanSplice>,
}

/// Captures the block's closed mermaid fences in order (empty included:
/// one record per closed mermaid fence is what keeps records aligned with
/// units), each with every bound span's byte range in the LF-normalized
/// interior. Walks with
/// the render tokenizer's fence grammar so records align one-to-one with
/// the fence-shaped units clients consume; captured at parse because a
/// region card's context carries the fence MASKED, so display text can
/// recover neither the unmasked fingerprint nor authored offsets.
fn capture_answer_fences(
    answer: &[(usize, String)],
    splices: &[SpanSplice],
) -> Vec<crate::card::AnswerFence> {
    use crate::render::{fence_info, fence_marker};
    let mut fences = Vec::new();
    let mut open: Option<(char, usize, bool, usize)> = None;
    for (index, (_, text)) in answer.iter().enumerate() {
        match (open, fence_marker(text)) {
            (Some((ch, len, mermaid, from)), Some(_))
                if closes_fence(text.trim_start(), ch, len) =>
            {
                open = None;
                if !mermaid {
                    continue;
                }
                let offset_in = |line: usize, at: usize| -> usize {
                    let prefix: usize = answer[from..line]
                        .iter()
                        .map(|(_, text)| text.len() + 1)
                        .sum();
                    prefix + at
                };
                let spans = splices
                    .iter()
                    .filter(|splice| (from..index).contains(&splice.answer_index))
                    .map(|splice| crate::card::AnswerFenceSpan {
                        line: splice.directive_line,
                        start: offset_in(splice.answer_index, splice.range.0),
                        end: offset_in(splice.answer_index, splice.range.1),
                    })
                    .collect();
                let interior = answer[from..index]
                    .iter()
                    .map(|(_, text)| text.as_str())
                    .collect::<Vec<&str>>()
                    .join("\n");
                fences.push(crate::card::AnswerFence {
                    fingerprint: crate::diagram::fingerprint(&interior),
                    interior: std::sync::Arc::from(interior),
                    spans,
                });
            }
            (Some(_), _) => {}
            (None, Some((marker, run))) => {
                let mermaid = fence_info(text, marker).eq_ignore_ascii_case("mermaid");
                open = Some((marker, run, mermaid, index + 1));
            }
            (None, None) => {}
        }
    }
    fences
}

/// Synthesizes the region cards a block's blanks ask (ADR 0034): a named
/// group is one card asking every member, an ungrouped blank one card each,
/// a cover no card. A blank-bearing block is a template: its region cards
/// REPLACE the cards `build_card` pushed, so no plain card exists beside
/// them; cover/crop-only blocks keep theirs.
/// Whole-word, case-insensitive containment. Short answers are skipped:
/// a three-letter answer matches too much prose to be worth reporting.
fn names_answer(note: &str, answer: &str) -> bool {
    let answer = answer.trim();
    if answer.chars().count() < 4 {
        return false;
    }
    let (note_lower, answer_lower) = (note.to_lowercase(), answer.to_lowercase());
    note_lower.match_indices(&answer_lower).any(|(at, hit)| {
        let before = note_lower[..at].chars().next_back();
        let after = note_lower[at + hit.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// One authored note after its group-addressed lines are separated out.
struct SplitNote {
    badge: Option<crate::card::Badge>,
    block: Option<String>,
    addressed: Vec<(String, bool, String)>,
}

/// A `>` line addressed to one group of this block: `name: text` replaces
/// the block note for its card, `name+: text` keeps the block note above it.
fn note_address(text: &str) -> Option<(&str, bool, &str)> {
    let (head, rest) = text.split_once(':')?;
    if !rest.starts_with(WHITESPACE) {
        return None;
    }
    let payload = trim_ws(rest);
    if payload.is_empty() {
        return None;
    }
    let (name, append) = match head.strip_suffix('+') {
        Some(name) => (name, true),
        None => (head, false),
    };
    region::is_group_name(name).then_some((name, append, payload))
}

fn split_note(
    note: &crate::card::Note,
    names: &std::collections::HashSet<&str>,
    line: usize,
    lints: &mut Vec<Lint>,
) -> SplitNote {
    let mut block: Vec<&str> = Vec::new();
    let mut addressed = Vec::new();
    for text in note.body.lines() {
        match note_address(text) {
            Some((name, append, payload)) if names.contains(name) => {
                addressed.push((name.to_string(), append, payload.to_string()));
            }
            // Only a block that names a group can be addressing one, so a
            // note beginning `2:` on any other block is prose.
            Some((name, ..)) if !names.is_empty() => {
                lints.push(Lint {
                    line,
                    kind: LintKind::NoteNamesNoGroup {
                        name: name.to_string(),
                    },
                });
                block.push(text);
            }
            _ => block.push(text),
        }
    }
    SplitNote {
        badge: note.badge,
        block: (!block.is_empty()).then(|| block.join("\n")),
        addressed,
    }
}

fn resolve_note(
    block: Option<&str>,
    addressed: &[(String, bool, String)],
    name: Option<&str>,
) -> Option<String> {
    let mine: Vec<&(String, bool, String)> = name
        .map(|name| addressed.iter().filter(|(to, ..)| to == name).collect())
        .unwrap_or_default();
    let Some((_, append, _)) = mine.first() else {
        return block.map(str::to_string);
    };
    let mut lines: Vec<&str> = Vec::new();
    if *append && let Some(block) = block {
        lines.push(block);
    }
    lines.extend(mine.iter().map(|(_, _, text)| text.as_str()));
    Some(lines.join("\n"))
}

fn build_region_cards(
    block_start: usize,
    cards: &mut Vec<Card>,
    prose: Option<&BlockProse>,
    lints: &mut Vec<Lint>,
) -> Result<(), ParseError> {
    use region::{RawRegion, RegionKind};

    use crate::card::{GroupMember, RegionSlot};

    if cards.len() == block_start {
        return Ok(());
    }
    let template = cards[block_start].clone();
    let mut blanks: Vec<&RawRegion> = template
        .images
        .iter()
        .chain(template.images_back.iter())
        .flat_map(|image| image.regions.iter())
        .chain(template.span_regions.iter())
        .filter(|region| region.kind == RegionKind::Blank)
        .collect();
    // Collection above walks storage buckets (front images, back images,
    // spans); everything downstream owes the AUTHOR's order.
    blanks.sort_by_key(|region| region.line);
    if blanks.is_empty() {
        return Ok(());
    }
    let names: std::collections::HashSet<&str> = blanks
        .iter()
        .filter_map(|blank| blank.group.as_deref())
        .collect();
    let notes: Vec<SplitNote> = template
        .notes
        .iter()
        .map(|note| split_note(note, &names, template.line, lints))
        .collect();

    // The block's prose rides every region card as context, its own span as
    // the blank marker, sibling and cover spans as the hidden marker; a rect
    // card masks every span. Splices land on authored bytes, so styling and
    // escapes survive around the markers.
    let masked_context = |own: &[usize]| -> Vec<String> {
        let Some(prose) = prose else {
            return Vec::new();
        };
        prose
            .lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                let mut line = line.clone()?;
                let mut cuts: Vec<&SpanSplice> = prose
                    .splices
                    .iter()
                    .filter(|splice| splice.answer_index == index)
                    .collect();
                cuts.sort_by_key(|splice| std::cmp::Reverse(splice.range.0));
                for cut in cuts {
                    let marker = if !cut.cover && own.contains(&cut.directive_line) {
                        cloze::BLANK
                    } else {
                        cloze::HIDDEN
                    };
                    line.replace_range(cut.range.0..cut.range.1, marker);
                }
                Some(line)
            })
            .collect()
    };
    let math_line = |line: usize| {
        prose.is_some_and(|prose| {
            prose
                .splices
                .iter()
                .any(|splice| splice.directive_line == line && !splice.cover && splice.math)
        })
    };
    let region_card = |slot: RegionSlot, back: Vec<String>| {
        let mut card = template.clone();
        // The displayed answer regains its `$` delimiters so a math-classed
        // span reveals rendered; `back` stays the typed/graded plain text.
        let asked: Vec<(usize, &String)> = match &slot {
            RegionSlot::Single { line, .. } => back.iter().map(|text| (*line, text)).collect(),
            RegionSlot::Group { members, .. } => members
                .iter()
                .filter(|member| member.hidden.is_some())
                .zip(&back)
                .map(|(member, text)| (member.line, text))
                .collect(),
        };
        card.display_back = asked.iter().any(|(line, _)| math_line(*line)).then(|| {
            asked
                .iter()
                .map(|(line, text)| match math_line(*line) {
                    true => format!("${text}$"),
                    false => (*text).clone(),
                })
                .collect()
        });
        card.back = back;
        // card.line stays the authored block line: card_front_lines exposes
        // every distinct line as a Markdown block boundary, and a directive
        // line there once let removal truncate the parent's answer.
        let own: Vec<usize> = match &slot {
            RegionSlot::Single { line, .. } => vec![*line],
            RegionSlot::Group { members, .. } => members.iter().map(|m| m.line).collect(),
        };
        card.context = masked_context(&own);
        card.context_leads = !card.context.is_empty();
        card.notes = notes
            .iter()
            .filter_map(|note| {
                let group = match &slot {
                    RegionSlot::Group { name, .. } => Some(name.as_str()),
                    RegionSlot::Single { .. } => None,
                };
                resolve_note(note.block.as_deref(), &note.addressed, group).map(|body| Note {
                    badge: note.badge,
                    body,
                })
            })
            .collect();
        card.region = Some(slot);
        card.reversed = false;
        card.direction = None;
        card.input = None;
        card.row = None;
        // The effective question includes the masked context (ADR 0034): two
        // spans over the same word in different sentences are different
        // questions, so cached AI artifacts stale when the context moves.
        let mut question = card.context.clone();
        question.extend(card.back.iter().cloned());
        card.content_fingerprint = content_fingerprint(&card.front, &question);
        card
    };

    let mut groups: Vec<(&str, Vec<&RawRegion>)> = Vec::new();
    let mut new_cards = Vec::new();
    for blank in &blanks {
        match blank.group.as_deref() {
            Some(name) => match groups.iter_mut().find(|(group, _)| *group == name) {
                Some((_, members)) => members.push(blank),
                None => groups.push((name, vec![blank])),
            },
            None => {
                new_cards.push(region_card(
                    RegionSlot::Single {
                        stamp: blank.stamp.as_deref().map(Arc::from),
                        hidden: blank.hidden.clone(),
                        line: blank.line,
                    },
                    blank.hidden.iter().cloned().collect(),
                ));
            }
        }
    }
    for (name, members) in groups {
        // The all-or-none rule for a group's answers: mixed presence would
        // leave the card half-answerable.
        let with_hidden = members.iter().filter(|m| m.hidden.is_some()).count();
        if with_hidden != 0 && with_hidden != members.len() {
            return Err(ParseError::InvalidRegion {
                line: members[0].line,
                message: format!(
                    "group `[{name}]` mixes regions that carry `hidden=` with regions that do not"
                ),
            });
        }
        let hash = members
            .iter()
            .map(|m| m.stamp.as_deref())
            .collect::<Option<Vec<&str>>>()
            .map(|stamps| Arc::from(crate::token::derive_group_hash(&stamps)));
        let back: Vec<String> = members.iter().filter_map(|m| m.hidden.clone()).collect();
        let slot = RegionSlot::Group {
            name: name.to_string(),
            hash,
            members: members
                .iter()
                .map(|m| GroupMember {
                    stamp: m.stamp.as_deref().map(Arc::from),
                    hidden: m.hidden.clone(),
                    line: m.line,
                })
                .collect(),
        };
        new_cards.push(region_card(slot, back));
    }
    if new_cards.len() > 1 {
        for (index, blank) in blanks.iter().enumerate() {
            let Some(hidden) = &blank.hidden else {
                continue;
            };
            if notes.iter().any(|note| {
                note.block
                    .as_deref()
                    .is_some_and(|block| names_answer(block, hidden))
            }) {
                lints.push(Lint {
                    line: template.line,
                    kind: LintKind::NoteContainsBlankAnswer {
                        blank: index + 1,
                        answer: hidden.clone(),
                    },
                });
            }
        }
    }
    new_cards.sort_by_key(|card| {
        card.region
            .as_ref()
            .map(crate::card::RegionSlot::first_line)
            .unwrap_or(0)
    });
    cards.truncate(block_start);
    cards.extend(new_cards);
    Ok(())
}

fn build_table_cards(
    subject: &Arc<str>,
    deck_id: &Arc<str>,
    raw: RawTable,
    cards: &mut Vec<Card>,
) -> Result<(), ParseError> {
    let start = cards.len();
    let section = raw.section.clone();
    let block_line = raw.line;
    let out = build_table_cards_inner(subject, deck_id, raw, cards);
    for card in &mut cards[start..] {
        card.section_context = section.clone();
        card.block_line = block_line;
        card.parent_block = None;
    }
    out
}

fn build_table_cards_inner(
    subject: &Arc<str>,
    deck_id: &Arc<str>,
    raw: RawTable,
    cards: &mut Vec<Card>,
) -> Result<(), ParseError> {
    if let Some(line) = raw
        .directives
        .regions
        .first()
        .map(|region| region.line)
        .or(raw.directives.crops.first().map(|crop| crop.line))
    {
        return Err(ParseError::InvalidRegion {
            line,
            message:
                "a region binds to a media element or an answer block, and a card table has neither"
                    .into(),
        });
    }
    let token: Option<Arc<str>> = raw.directives.token.as_deref().map(Arc::from);
    for row in raw.rows {
        let front = row.cells[0].clone();
        if front.is_empty() {
            return Err(ParseError::EmptyFront(row.line));
        }
        let back = row.cells[1].clone();
        if back.is_empty() {
            return Err(ParseError::FrontWithoutAnswer(row.line));
        }
        let note = row.cells.get(2).filter(|cell| !cell.is_empty()).cloned();
        let notes = Vec::from_iter(note.map(Note::bare));
        let mut card = Card::plain(Arc::clone(subject), front, vec![back], notes, row.line);
        card.deck_id = Arc::clone(deck_id);
        card.context = raw.title.iter().cloned().collect();
        // An unstamped row is an unstamped card: composing an id from the
        // container alone would collide every such row on the base id.
        if let Some(stamp) = row.stamp {
            card.token = token.clone();
            card.row = Some(Arc::from(stamp.as_str()));
        }
        card.reveal = raw.directives.reveal;
        card.input = raw.directives.input;
        card.direction = raw.directives.direction;
        card.sampling = raw.directives.sampling;
        card.citations = raw.directives.citations.clone();
        card.diagrams = raw.directives.diagrams.clone();
        card.givens = raw.directives.givens.clone();
        cards.push(card);
    }
    Ok(())
}

fn take_card(current: &mut Option<RawCard>) -> Result<Option<RawCard>, ParseError> {
    let card = current.take();
    if let Some(line) = card
        .as_ref()
        .and_then(|card| card.open_math.or(card.open_note_math))
    {
        return Err(ParseError::UnclosedDisplayMath(line));
    }
    Ok(card)
}

fn push_content(
    current: &mut Option<RawCard>,
    lineno: usize,
    text: String,
) -> Result<(), ParseError> {
    if let Some(card) = current.as_mut() {
        card.machinery_stays_trailing()?;
        if card.divided {
            card.back.push((lineno, text));
        } else {
            card.front_extra.push((lineno, text));
        }
    }
    Ok(())
}

/// Whether the note run starting at `from` carries any visible text: a
/// draft or a pasted callout can leave a body of blank `>` lines, which
/// renders as nothing at all.
fn note_run_is_empty(lines: &[&str], from: usize) -> bool {
    lines[from.min(lines.len())..]
        .iter()
        .map(|line| trim_ws(line))
        .map_while(|line| line.strip_prefix('>'))
        .all(|body| trim_ws(body).is_empty())
}

fn append_note(card: &mut RawCard, text: &str) {
    if let Some(badge) = card.pending_badge.take() {
        card.notes.push(Note {
            badge: Some(badge),
            body: text.to_string(),
        });
        return;
    }
    match card.notes.last_mut() {
        Some(note) => {
            note.body.push('\n');
            note.body.push_str(text);
        }
        None => card.notes.push(Note::bare(text.to_string())),
    }
}

fn heading(
    rest: &str,
    lineno: usize,
    lints: &mut Vec<Lint>,
) -> Result<(String, CardDirectives), ParseError> {
    let mut directives = CardDirectives::default();
    let (text, bodies) = split_trailing_comments(rest);
    for body in bodies {
        if let Some((key, value)) = directive(&body) {
            let before = lints.len();
            let recognized = match apply_directive(&mut directives, &key, value, lineno, lints) {
                Err(_) => true,
                Ok(()) => !lints[before..]
                    .iter()
                    .any(|lint| matches!(&lint.kind, LintKind::UnknownKey { key: k } if *k == key)),
            };
            if recognized {
                return Err(ParseError::FrontDirective { line: lineno, key });
            }
        }
    }
    // `\#` is a literal front-text hash; never part of a trailing closing run.
    let front = strip_trailing_hashes(trim_ws(&text)).replace("\\#", "#");
    Ok((front, directives))
}

pub(crate) fn split_trailing_comments(text: &str) -> (String, Vec<String>) {
    let mut text = trim_ws(text);
    let mut bodies = Vec::new();
    while let Some(prefix) = text.strip_suffix("-->") {
        let Some(start) = prefix.rfind("<!--") else {
            break;
        };
        let body = &prefix[start + 4..];
        if body.contains("-->") {
            break;
        }
        bodies.push(body.to_string());
        text = trim_ws(&prefix[..start]);
    }
    bodies.reverse();
    (text.to_string(), bodies)
}

fn strip_trailing_hashes(text: &str) -> &str {
    let stripped = text.trim_end_matches('#');
    if stripped.len() == text.len() {
        text
    } else if stripped.is_empty() || stripped.ends_with(WHITESPACE) {
        trim_ws(stripped)
    } else {
        text
    }
}

pub(crate) fn directive(body: &str) -> Option<(String, String)> {
    let (key, value) = trim_ws(body).split_once(':')?;
    let key = trim_ws(key).to_ascii_lowercase();
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    Some((key, trim_ws(value).to_string()))
}

fn is_known_card_key(key: &str) -> bool {
    matches!(
        key,
        "id" | "reveal" | "input" | "direction" | "at" | "given" | "sampling"
    )
}

fn apply_directive(
    directives: &mut CardDirectives,
    key: &str,
    value: String,
    line: usize,
    lints: &mut Vec<Lint>,
) -> Result<(), ParseError> {
    if value.is_empty() && is_known_card_key(key) {
        lints.push(Lint {
            line,
            kind: LintKind::EmptyValue {
                key: key.to_string(),
            },
        });
        return Ok(());
    }
    match key {
        "id" => {
            // Markers hold base ids only; a sub-id suffix (`-N`, `-r`) never appears here.
            if !matches!(
                token::parse_prefixed_card_id(&value),
                Some((_, None, false, None))
            ) {
                return Err(ParseError::InvalidCardId { line, value });
            }
            if directives.token.is_some() {
                return Err(ParseError::DuplicateCardId { line });
            }
            directives.token = Some(value);
        }
        "reveal" => match parse_reveal(&value) {
            Some(reveal) => {
                directives.reveal = Some(reveal);
                directives.reveal_line = Some(line);
            }
            None => lints.push(bad_value(line, key, value)),
        },
        "input" => match Input::parse(&value) {
            Some(input) => directives.input = Some(input),
            None => lints.push(bad_value(line, key, value)),
        },
        "direction" => match Direction::parse(&value) {
            Some(direction) => directives.direction = Some(direction),
            None => lints.push(bad_value(line, key, value)),
        },
        "at" => {
            let bad = |message: String| ParseError::InvalidLocator { line, message };
            // `directive` consumed the `at:` key; the tokenizer wants the
            // whole `at: ...` comment body back.
            let fields = crate::source::parse_locator_fields(&format!("at: {value}"))
                .map_err(|error| bad(format!("{error:#}")))?;
            let fingerprint = fields
                .fingerprint
                .map(|raw| {
                    crate::source::parse_locator_fingerprint(&raw).ok_or_else(|| {
                        bad(format!(
                            "fingerprint `{raw}` is not `xxh64-` plus 16 hex digits"
                        ))
                    })
                })
                .transpose()?;
            directives.citations.push(crate::card::SourceCitation {
                locator: fields.at,
                fingerprint,
                asset: fields.asset,
                line,
            });
        }
        "diagram" => {
            let bad = |message: String| ParseError::InvalidLocator { line, message };
            directives
                .diagrams
                .push(parse_diagram_stamp(&value, line).map_err(bad)?);
        }
        "sampling" => match parse_sampling(&value) {
            Some(sampling) => directives.sampling = Some(sampling),
            None => lints.push(bad_value(line, key, value)),
        },
        "given" => directives.givens.push(value),
        "blank" => directives.regions.push(region::parse_blank(&value, line)?),
        "cover" => directives.regions.push(region::parse_cover(&value, line)?),
        "crop" => directives.crops.push(region::parse_crop(&value, line)?),
        _ => lints.push(Lint {
            line,
            kind: LintKind::UnknownKey {
                key: key.to_string(),
            },
        }),
    }
    Ok(())
}

// ── Card building and cloze ──

fn card_images(segments: &[Seg]) -> impl Iterator<Item = CardImage> + '_ {
    segments.iter().filter_map(|segment| match segment {
        Seg::Image { src, alt } => Some(CardImage {
            src: PathBuf::from(src),
            alt: alt.clone(),
            regions: Vec::new(),
            crop: None,
        }),
        Seg::Text(_) => None,
    })
}

fn bind_regions(
    regions: Vec<region::RawRegion>,
    crops: Vec<region::RawCrop>,
    front_media: &mut [(usize, CardImage)],
    back_media: &mut [(usize, CardImage)],
    answer_start: usize,
) -> Result<Vec<region::RawRegion>, ParseError> {
    fn target<'a>(
        front: &'a mut [(usize, CardImage)],
        back: &'a mut [(usize, CardImage)],
        line: usize,
        answer_start: usize,
    ) -> Option<&'a mut CardImage> {
        let side = if line >= answer_start { back } else { front };
        side.iter_mut()
            .rfind(|(media_line, _)| *media_line < line)
            .map(|(_, image)| image)
    }
    let mut spans = Vec::new();
    for crop in crops {
        let Some(image) = target(front_media, back_media, crop.line, answer_start) else {
            return Err(ParseError::InvalidRegion {
                line: crop.line,
                message: "`crop:` needs a preceding media element on its side of the card".into(),
            });
        };
        if image.crop.is_some() {
            return Err(ParseError::InvalidRegion {
                line: crop.line,
                message: "a media element takes at most one `crop:`".into(),
            });
        }
        image.crop = Some(crop);
    }
    for region in regions {
        if matches!(region.geometry, region::RegionGeometry::Span { .. }) {
            spans.push(region);
            continue;
        }
        let Some(image) = target(front_media, back_media, region.line, answer_start) else {
            return Err(ParseError::InvalidRegion {
                line: region.line,
                message: "a geometric region needs a preceding media element on its side of the card (`span` binds to the answer block instead)".into(),
            });
        };
        image.regions.push(region);
    }
    for (_, image) in front_media.iter_mut().chain(back_media.iter_mut()) {
        region::validate_media(&mut image.regions, image.crop.as_ref())?;
    }
    Ok(spans)
}

// The empty guard is load-bearing: an all-image line drops, but a blank content line (a fence's
// blank, which yields no segments) must stay.
fn image_only(segments: &[Seg]) -> bool {
    !segments.is_empty() && segments.iter().all(|s| matches!(s, Seg::Image { .. }))
}

fn mixed_image_line(segments: &[Seg]) -> bool {
    segments.iter().any(|s| matches!(s, Seg::Image { .. }))
        && segments.iter().any(|s| match s {
            Seg::Image { .. } => false,
            Seg::Text(text) => !text.trim().is_empty(),
        })
}

fn build_card(
    subject: &Arc<str>,
    deck_id: &Arc<str>,
    raw: RawCard,
    tasklist_default: Option<Mapping>,
    cards: &mut Vec<Card>,
    lints: &mut Vec<Lint>,
) -> Result<Option<BlockProse>, ParseError> {
    let start = cards.len();
    let section = raw.section.clone();
    let block_line = raw.line;
    let parent = raw.parent;
    let out = build_card_inner(subject, deck_id, raw, tasklist_default, cards, lints);
    for card in &mut cards[start..] {
        card.section_context = section.clone();
        card.block_line = block_line;
        card.parent_block = parent;
    }
    out
}

fn build_card_inner(
    subject: &Arc<str>,
    deck_id: &Arc<str>,
    raw: RawCard,
    tasklist_default: Option<Mapping>,
    cards: &mut Vec<Card>,
    lints: &mut Vec<Lint>,
) -> Result<Option<BlockProse>, ParseError> {
    let RawCard {
        line,
        // The wrapper stamps it onto every card this block emits.
        section: _,
        depth: _,
        // The wrapper stamps it onto every card this block emits.
        parent: _,
        front: heading,
        front_extra,
        back,
        divided,
        divider_line,
        notes,
        directives,
        mapping,
        machinery: _,
        open_math: _,
        open_note_math: _,
        pending_badge: _,
    } = raw;
    let mut front_media: Vec<(usize, CardImage)> = Vec::new();
    {
        let segments = scan_markers(&heading, line, lints)?;
        if segments.iter().any(|s| matches!(s, Seg::Image { .. })) {
            return Err(ParseError::MixedImageLine(line));
        }
    }
    let (front, answer) = if divided {
        let mut front_lines = vec![heading];
        for (lineno, text) in &front_extra {
            let segments = scan_markers(text, *lineno, lints)?;
            if mixed_image_line(&segments) {
                return Err(ParseError::MixedImageLine(*lineno));
            }
            front_media.extend(card_images(&segments).map(|image| (*lineno, image)));
            if !image_only(&segments) {
                front_lines.push(seg_display(&segments));
            }
        }
        (front_lines.join("\n"), back)
    } else {
        (heading, front_extra)
    };
    if answer.is_empty() {
        return Err(ParseError::FrontWithoutAnswer(line));
    }

    let mut parsed = Vec::with_capacity(answer.len());
    for (lineno, text) in &answer {
        let segments = scan_markers(text, *lineno, lints)?;
        if mixed_image_line(&segments) {
            return Err(ParseError::MixedImageLine(*lineno));
        }
        parsed.push(segments);
    }
    let mut back_media: Vec<(usize, CardImage)> = Vec::new();
    for ((lineno, _), segments) in answer.iter().zip(&parsed) {
        back_media.extend(card_images(segments).map(|image| (*lineno, image)));
    }

    // The side boundary is the divider itself: a directive between `---` and
    // the first answer content line is already answer-side.
    let answer_start = if divided {
        divider_line.map(|line| line + 1).unwrap_or(usize::MAX)
    } else {
        0
    };
    let mut span_regions = bind_regions(
        directives.regions,
        directives.crops,
        &mut front_media,
        &mut back_media,
        answer_start,
    )?;
    // Span binding consumes the canonical maskable stream (ADR 0034): the
    // hidden text must occur at least N times in its MATCHABLE text, or the
    // anchor is gone and the deck fails loudly. Plain blocks never pay for
    // the stream.
    let mut splices: Vec<SpanSplice> = Vec::new();
    let mut bound: Vec<(usize, usize, usize)> = Vec::new();
    let mut bound_positions: Vec<(usize, u32)> = Vec::new();
    let mut minted_occurrences: Vec<(usize, u32, usize, usize, usize)> = Vec::new();
    if !span_regions.is_empty() {
        let stream = stream::maskable_stream(&answer, &parsed);
        for (span_index, span) in span_regions.iter().enumerate() {
            let region::RegionGeometry::Span {
                occurrence: n,
                boundary,
            } = &span.geometry
            else {
                continue;
            };
            let hidden = span.hidden.as_deref().unwrap_or_default();
            let whole_word = *boundary == region::Boundary::Word;
            // Matchability decides acceptance INSIDE the advance loop: a
            // candidate that cannot bind (crossing, math, unbounded)
            // advances one scalar and must never consume an overlapping
            // candidate that can. Its class survives for the diagnostic.
            let mut rejected: Vec<stream::RangeClass> = Vec::new();
            let mut math_violation: Option<mathspan::Violation> = None;
            let candidates = region::occurrences_with(&stream.text, hidden, &mut |start, end| {
                let range = start..end;
                let class = stream.classify(&range);
                let unit = match class {
                    stream::RangeClass::Matchable => true,
                    stream::RangeClass::Math => {
                        let piece = stream
                            .math_piece(&range)
                            .expect("a math-classed range lies within one piece");
                        match mathspan::structural_unit(
                            &stream.text[piece.clone()],
                            &(start - piece.start..end - piece.start),
                        ) {
                            Ok(()) => true,
                            Err(violation) => {
                                math_violation.get_or_insert(violation);
                                false
                            }
                        }
                    }
                    stream::RangeClass::Crossing => false,
                };
                let accepted = unit
                    && (!whole_word || stream.word_bounded(&range))
                    && stream.grapheme_bounded(&range);
                if !accepted {
                    rejected.push(class);
                }
                accepted
            });
            let found = candidates.len();
            if found < *n as usize {
                let message = if rejected.contains(&stream::RangeClass::Crossing) {
                    "the span's hidden text only matches across a masking or style boundary (a hole, a link, or styled text); rephrase the target or split the span".to_string()
                } else if let Some(violation) = &math_violation {
                    format!(
                        "the span's hidden text is not a complete structural unit of its formula: {}; blank a complete unit or the whole formula",
                        violation.message()
                    )
                } else {
                    format!(
                        "the span's hidden text occurs {found} time(s) in the block's matchable text, fewer than the {n} its locator names"
                    )
                };
                return Err(ParseError::InvalidRegion {
                    line: span.line,
                    message,
                });
            }
            let (start, end) = candidates[*n as usize - 1];
            // The block invariant (ADR 0034): no two spans may resolve to
            // the same or overlapping stream range, atomically, before any
            // splice runs on it.
            if let Some((_, _, other)) = bound
                .iter()
                .find(|(from, to, _)| start < *to && *from < end)
            {
                return Err(ParseError::InvalidRegion {
                    line: span.line,
                    message: format!(
                        "two spans resolve to overlapping text (lines {other} and {}); retarget or remove one",
                        span.line
                    ),
                });
            }
            bound.push((start, end, span.line));
            bound_positions.push((span_index, stream.grapheme_position(start)));
            if let Some(minted) = span.minted_position
                && minted != stream.grapheme_position(start)
                && let Some(byte) = stream.grapheme_byte(minted)
                && let Some(index) = candidates.iter().position(|(from, _)| *from == byte)
            {
                let (from, to) = candidates[index];
                minted_occurrences.push((span_index, index as u32 + 1, from, to, span.line));
            }
            if let Some(piece) = stream.math_piece(&(start..end)) {
                let payload = &stream.text[piece.clone()];
                if let Err(error) = crate::math::parses(payload) {
                    return Err(ParseError::InvalidRegion {
                        line: span.line,
                        message: format!("the formula under the span does not parse: {error}"),
                    });
                }
                for marker in [r"\boxed{?}", r"\boxed{\cdots}"] {
                    let mut masked = payload.to_string();
                    masked.replace_range(start - piece.start..end - piece.start, marker);
                    if let Err(error) = crate::math::parses(&masked) {
                        return Err(ParseError::InvalidRegion {
                            line: span.line,
                            message: format!(
                                "masking the span leaves a formula that does not parse: {error}"
                            ),
                        });
                    }
                }
                // A typed span answer holding a command asks for its spelling
                // (cloze's untypable rule; math spans draw unless pinned).
                if span.kind == region::RegionKind::Blank
                    && directives.input == Some(Input::Type)
                    && hidden.contains('\\')
                {
                    lints.push(Lint {
                        line: span.line,
                        kind: LintKind::UntypableSpan {
                            answer: hidden.to_string(),
                        },
                    });
                }
            }
            let (answer_index, range) = stream
                .splice(&(start..end))
                .expect("an accepted candidate lies within one piece");
            splices.push(SpanSplice {
                directive_line: span.line,
                cover: span.kind == region::RegionKind::Cover,
                math: stream.math_piece(&(start..end)).is_some(),
                answer_index,
                range: (range.start, range.end),
            });
        }
        for (span_index, position) in bound_positions {
            span_regions[span_index].bound_position = Some(position);
        }
        // Keep-old-target is only offered when taking it would keep the
        // block invariant: an alternate range another span owns stays a
        // stale anchor, never doctor advice that breaks the deck.
        for (span_index, occurrence, from, to, line) in minted_occurrences {
            let taken = bound.iter().any(|(other_from, other_to, other_line)| {
                *other_line != line && from < *other_to && *other_from < to
            });
            if !taken {
                span_regions[span_index].minted_occurrence = Some(occurrence);
            }
        }
    }

    let images: Vec<CardImage> = front_media.into_iter().map(|(_, image)| image).collect();
    let images_back: Vec<CardImage> = back_media.into_iter().map(|(_, image)| image).collect();
    // A blank makes the block a template producing only region cards, so a
    // second card family on the same block has no coherent card set.
    let first_blank_line = span_regions
        .iter()
        .chain(
            images
                .iter()
                .chain(images_back.iter())
                .flat_map(|image| image.regions.iter()),
        )
        .filter(|region| region.kind == region::RegionKind::Blank)
        .map(|region| region.line)
        .min();

    // The block-level dedup key: front + cover-masked RAW answer lines
    // (literal `\blank{...}` markers count as text, so a plain card
    // repeating a hole's hidden text cannot collide; cover cuts make a
    // moved cover change the key). Every card of the block carries it,
    // while content_fingerprint stays the card's own effective question.
    let masked_answer: Vec<String> = answer
        .iter()
        .enumerate()
        .map(|(index, (_, text))| {
            let mut cuts: Vec<&SpanSplice> = splices
                .iter()
                .filter(|splice| splice.cover && splice.answer_index == index)
                .collect();
            cuts.sort_by_key(|splice| std::cmp::Reverse(splice.range.0));
            let mut line = text.clone();
            for cut in cuts {
                line.replace_range(cut.range.0..cut.range.1, cloze::HIDDEN);
            }
            line
        })
        .collect();
    let block_key = content_fingerprint(&front, &masked_answer);

    let mut task_lines = Vec::new();
    let mut has_other = false;
    let mut fence = None;
    for ((lineno, text), segments) in answer.iter().zip(&parsed) {
        if let Some((ch, open)) = fence {
            if closes_fence(text, ch, open) {
                fence = None;
            }
            has_other = true;
            continue;
        }
        if let Some(opened) = fence_opener(text) {
            fence = Some(opened);
            has_other = true;
            continue;
        }
        if trim_ws(text).is_empty() || image_only(segments) {
            continue;
        }
        match checklist::parse_line(text) {
            Some((checked, option)) => task_lines.push((*lineno, checked, option)),
            None => has_other = true,
        }
    }
    let mapping = match mapping.or(tasklist_default) {
        Some(m @ (Mapping::ChoicesSingle | Mapping::ChoicesMultiple)) => Some(m),
        _ => None,
    };
    if let Some(mapping) = mapping
        && !task_lines.is_empty()
    {
        if let Some(line) = first_blank_line {
            return Err(ParseError::InvalidRegion {
                line,
                message: "a `blank:` region cannot share a block with a task-list answer".into(),
            });
        }
        if has_other {
            return Err(ParseError::ChoiceShape {
                line: task_lines[0].0,
                message: "an invoked choice card mixes its task list with other answer content"
                    .into(),
            });
        }
        let choice_line = task_lines[0].0;
        let mut seen = HashSet::new();
        let mut options = Vec::new();
        let mut duplicate_line = None;
        for (lineno, checked, raw_option) in task_lines {
            let option = raw_option.trim().to_string();
            let content = crate::inline::strip_inline(&option);
            if seen.insert(content) {
                options.push((checked, option));
            } else if duplicate_line.is_none() {
                duplicate_line = Some(lineno);
            }
        }
        if let Some(line) = duplicate_line {
            lints.push(Lint {
                line,
                kind: LintKind::DuplicateChoiceOption,
            });
        }
        let checked_count = options.iter().filter(|(checked, _)| *checked).count();
        if notes
            .iter()
            .any(|note| choice::note_names_position(&note.body))
        {
            lints.push(Lint {
                line: choice_line,
                kind: LintKind::ChoiceNoteNamesPosition,
            });
        }
        let correct: Vec<String> = options
            .iter()
            .filter(|(checked, _)| *checked)
            .map(|(_, text)| text.clone())
            .collect();
        let distractors: Vec<String> = options
            .into_iter()
            .filter(|(checked, _)| !checked)
            .map(|(_, text)| text)
            .collect();
        match mapping {
            Mapping::ChoicesSingle => {
                if checked_count != 1 {
                    return Err(ParseError::ChoiceShape {
                        line: choice_line,
                        message: format!(
                            "`choices-single` needs exactly one `[x]`, found {checked_count}"
                        ),
                    });
                }
                if distractors.is_empty() {
                    return Err(ParseError::ChoiceShape {
                        line: choice_line,
                        message: "`choices-single` needs at least one unchecked `[ ]` distractor"
                            .into(),
                    });
                }
            }
            _ => {
                if checked_count == 0 {
                    return Err(ParseError::ChoiceShape {
                        line: choice_line,
                        message: "`choices-multiple` checks no option; mark at least one `[x]`"
                            .into(),
                    });
                }
            }
        }
        let mut card = Card::plain(Arc::clone(subject), front, correct, notes, line);
        card.deck_id = Arc::clone(deck_id);
        card.token = directives.token.as_deref().map(Arc::from);
        card.images = images;
        card.images_back = images_back;
        card.span_regions = span_regions;
        card.citations = directives.citations;
        card.diagrams = directives.diagrams;
        card.givens = directives.givens;
        card.authored_distractors = distractors;
        card.multiple_choice = mapping == Mapping::ChoicesMultiple;
        cards.push(card);
        return Ok(None);
    }

    let answer_fences = capture_answer_fences(&answer, &splices);

    let back_lines: Vec<String> = parsed
        .iter()
        .filter(|segments| !image_only(segments))
        .map(|segments| seg_display(segments))
        .collect();
    let mut card = Card::plain(Arc::clone(subject), front, back_lines, notes, line);
    card.deck_id = Arc::clone(deck_id);
    card.token = directives.token.as_deref().map(Arc::from);
    card.reveal = directives.reveal;
    card.input = directives.input;
    card.direction = directives.direction;
    card.sampling = directives.sampling;
    card.images = images;
    card.images_back = images_back;
    card.span_regions = span_regions;
    card.answer_fences = answer_fences;
    card.citations = directives.citations;
    card.diagrams = directives.diagrams;
    card.givens = directives.givens;
    card.block_fingerprint = block_key;
    cards.push(card);
    let prose = first_blank_line.is_some().then(|| BlockProse {
        lines: answer
            .iter()
            .zip(&parsed)
            .map(|((_, text), segments)| (!image_only(segments)).then(|| text.clone()))
            .collect(),
        splices,
    });
    Ok(prose)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Order;

    fn parse(text: &str) -> ParsedDeck {
        super::parse("deck.md", text).unwrap()
    }

    #[test]
    fn a_blank_marker_is_literal_answer_text() {
        let deck = parse_str("d.md", "## q\nParis is in \\blank{Europe}\n").expect("parses");
        assert_eq!(1, deck.len(), "one plain card, no sub-cards: {deck:?}");
        assert_eq!(
            vec!["Paris is in \\blank{Europe}"],
            deck[0].back,
            "the marker is ordinary literal answer text"
        );
    }

    #[test]
    fn one_authored_base_id_remains_legal_for_every_derived_or_inert_shape() {
        let id = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        let cloze = parse(&format!(
            "## cloze\nalpha and beta\n<!-- blank: span hidden=\"alpha\" b:a1b2c3 -->\n<!-- blank: span hidden=\"beta\" b:d4e5f6 -->\n<!-- id: {id} -->\n"
        ));
        assert_eq!(2, cloze.cards.len(), "span blanks derive two sub-ids");

        let reversed = parse(&format!(
            "## reverse\nfront\n\n---\nback\n<!-- direction: both -->\n<!-- id: {id} -->\n"
        ));
        assert_eq!(
            Some(Direction::Both),
            reversed.cards[0].direction,
            "the deck layer derives the reversed half from this base id"
        );

        let region = parse(&format!(
            "## region\n---\ntarget\n<!-- blank: span hidden=\"target\" b:a1b2c3 -->\n<!-- id: {id} -->\n"
        ));
        assert!(
            region.cards.iter().any(|card| card.region.is_some()),
            "a region derives an identity from its parent base"
        );

        let table = parse(&format!(
            "| front | back |\n|---|---|\n| q | a | <!-- r:4k2x9w -->\n<!-- cards -->\n<!-- id: {id} -->\n"
        ));
        assert_eq!(
            Some(format!("{id}-t4k2x9w")),
            table.cards[0].id(),
            "the row stamp composes with the table container id"
        );

        let personal = parse_sidecar(
            "deck.personal.md",
            &format!(
                "#### reader label\n## personal\nanswer\n> [!NOTE]\n> context\n<!-- id: {id} -->\n"
            ),
        )
        .unwrap();
        assert_eq!(Some(id.to_string()), personal[0].id());

        let fenced = parse(&format!(
            "## fence\n```text\n<!-- id: card-6v3c7x4k1m8q3z5t0b2n4d8f9w -->\n```\n<!-- id: {id} -->\n"
        ));
        assert_eq!(Some(id.to_string()), fenced.cards[0].id());

        let table_cell = parse(&format!(
            "| front | back |\n|---|---|\n| q <!-- id: card-6v3c7x4k1m8q3z5t0b2n4d8f9w --> | a | <!-- r:4k2x9w -->\n<!-- cards -->\n<!-- id: {id} -->\n"
        ));
        assert_eq!(Some(format!("{id}-t4k2x9w")), table_cell.cards[0].id());
    }

    #[test]
    fn link_definition_lines_are_deck_metadata_never_answer_content() {
        let deck = parse("## Q\nsee [the ref][r] here\n[r]: https://alix.study\n");
        assert_eq!(
            deck.cards[0].back,
            vec!["see [the ref][r] here"],
            "the definition line left the answer"
        );
        assert_eq!(
            deck.definitions,
            vec!["r"],
            "the label became deck-wide metadata"
        );

        for (text, back, why) in [
            (
                "## Q\n```\n[r]: x\n```\n",
                vec!["```", "[r]: x", "```"],
                "a fence keeps a definition line literal",
            ),
            (
                "## Q\n\\[r]: x\n",
                vec![r"\[r]: x"],
                "an escaped bracket stays prose",
            ),
            (
                "## Q\n[r]:\n",
                vec!["[r]:"],
                "an empty destination stays prose",
            ),
        ] {
            let deck = parse(text);
            assert_eq!(deck.cards[0].back, back, "{why}");
            assert!(deck.definitions.is_empty(), "{why}: {:?}", deck.definitions);
        }

        let holed = parse("## Q\n[\\blank{r}]: x\n");
        assert!(
            holed.definitions.is_empty(),
            "a hole marker keeps the line a card's prose"
        );
        assert!(!holed.cards.is_empty(), "the blank stays a drillable hole");

        let err = err("## Q\n[r]: x\n");
        assert!(
            matches!(err, ParseError::FrontWithoutAnswer(1)),
            "a definition-only answer leaves the card answerless: {err}"
        );
    }

    #[test]
    fn link_definition_grammar_keeps_invalid_raw_space_destinations_as_answer_prose() {
        let deck = parse("## Q\nprimary answer\n[ATP]: energy currency\n");
        assert_eq!(
            deck.cards[0].back,
            vec!["primary answer", "[ATP]: energy currency"],
            "a raw-space destination is not a GFM link definition"
        );
        assert!(
            deck.definitions.is_empty(),
            "invalid definition-shaped prose must not enter metadata"
        );
    }

    #[test]
    fn link_definition_grammar_consumes_definitions_outside_card_answers() {
        let deck = parse("# Section\n[r]: /url\n## Q\n[use][r]\n");
        assert_eq!(
            deck.definitions,
            vec!["r"],
            "the ruled definition table is deck-wide, including section prose"
        );
        assert!(
            deck.cards[0].context.iter().all(|line| line != "[r]: /url"),
            "hidden definition metadata must not leak into card context"
        );
    }

    #[test]
    fn link_definition_grammar_consumes_a_title_on_the_following_line() {
        let deck = parse("## Q\nprimary answer\n[r]: /url\n  \"reference title\"\n");
        assert_eq!(
            deck.cards[0].back,
            vec!["primary answer"],
            "a continued GFM definition title is hidden metadata, not an answer"
        );
        assert_eq!(deck.definitions, vec!["r"]);
    }

    #[test]
    fn link_definition_grammar_accepts_a_destination_on_the_following_line() {
        let deck = parse("# Section\n[r]:\n  /target\n## Q\n[reference][r]\n");

        assert_eq!(deck.definitions, vec!["r"]);
        assert!(
            deck.cards[0]
                .section_context
                .iter()
                .all(|line| line != "[r]:" && line.trim() != "/target"),
            "the complete definition block is metadata, never section prose"
        );
    }

    #[test]
    fn a_break_line_stops_a_definition_it_could_otherwise_have_continued() {
        for spelling in ["---", "----", "- - -", "***", "___"] {
            assert_eq!(
                ParseError::StrayDivider(3),
                err(&format!(
                    "# Section\n[r]:\n  {spelling}\n## Q\n[reference][r]\n"
                )),
                "a bare destination made only of break markers: {spelling}"
            );
        }
    }

    #[test]
    fn link_definition_grammar_accepts_a_label_split_across_lines() {
        let deck = parse("# Section\n[\nr\n]: /target\n## Q\n[reference][r]\n");

        assert_eq!(deck.definitions, vec!["r"]);
        assert!(
            deck.cards[0]
                .section_context
                .iter()
                .all(|line| !matches!(line.as_str(), "[" | "r" | "]: /target")),
            "the complete definition block is metadata, never section prose"
        );
    }

    /// The definition side normalizes its label by the same grammar class
    /// as the candidate side, and a bare destination keeps a separator that
    /// is not in that class.
    #[test]
    fn link_definition_labels_collapse_only_the_grammar_whitespace_class() {
        for gap in ['\t', '\x0B', '\x0C', ' '] {
            let deck = parse(&format!("# Section\n[a{gap}b]: /target\n## Q\nx\n"));
            assert_eq!(
                deck.definitions,
                vec!["a b"],
                "U+{:04X} is grammar whitespace and collapses in the label",
                gap as u32
            );
        }
        let deck = parse("# Section\n[a\u{a0}b]: /target\n## Q\nx\n");
        assert_eq!(
            deck.definitions,
            vec!["a\u{a0}b"],
            "a no-break space is label content, so it survives normalization"
        );
        let deck = parse("# Section\n[r]: /a\u{a0}b\n## Q\nx\n");
        assert_eq!(
            deck.definitions,
            vec!["r"],
            "a no-break space is not ASCII space, so the destination keeps it"
        );
        let deck = parse("# Section\n[r]: /target \"t\"\u{a0}\n## Q\nx\n");
        assert!(
            deck.definitions.is_empty(),
            "a no-break space is a further non-whitespace character on the \
             line, which the definition grammar forbids"
        );
        let deck = parse("# Section\n[r]:\u{a0}\n/target\n## Q\nx\n");
        assert!(
            deck.cards[0]
                .section_context
                .contains(&"/target".to_owned()),
            "a no-break space never separates a definition's parts, so the \
             next line stays prose instead of being eaten as the destination"
        );
    }

    /// Rows pinned to CommonMark 0.31.2 corpus examples, which the local
    /// harness carries: a definition's parts may each cross one line end.
    #[test]
    fn link_definition_grammar_accepts_the_corpus_multiline_forms() {
        for (block, label, why) in [
            (
                "   [foo]: \n      /url  \n           'the title'  ",
                "foo",
                "e193 verbatim: continuation lines carry their own indentation",
            ),
            (
                "[Foo bar]:\n<my url>\n'title'",
                "Foo bar",
                "e195 verbatim: an angle destination and title each take a line",
            ),
            (
                "[foo]:\n/url",
                "foo",
                "e198 verbatim: the destination may open the next line",
            ),
            (
                "[\nfoo\n]: /url",
                "foo",
                "e208 verbatim: the label itself may span lines",
            ),
            (
                "[the\nlabel]: /target",
                "the label",
                "a split label collapses to one space",
            ),
        ] {
            let deck = parse(&format!("# Section\n{block}\n## Q\nanswer\n"));
            assert_eq!(deck.definitions, vec![label], "{why}");
            let leaked: Vec<&String> = deck.cards[0]
                .section_context
                .iter()
                .filter(|line| block.lines().any(|part| part.trim() == line.trim()))
                .collect();
            assert!(
                leaked.is_empty(),
                "{why}: every consumed line is metadata, never section prose: {leaked:?}"
            );
        }
    }

    #[test]
    fn link_definition_grammar_accepts_the_exact_indented_commonmark_example_193() {
        let deck =
            parse("# Section\n   [foo]: \n      /url  \n           'the title'  \n## Q\n[foo]\n");

        assert_eq!(
            deck.definitions,
            vec!["foo"],
            "the exact continuation indentation from CommonMark 0.31.2 example 193 remains definition metadata"
        );
        assert!(
            deck.cards[0]
                .section_context
                .iter()
                .all(|line| !line.contains("/url") && !line.contains("the title")),
            "the destination and title continuation lines never leak into section context"
        );
    }

    #[test]
    fn link_definition_grammar_accepts_gfm_forms_and_rejects_trailing_prose() {
        let accepted = [
            ("[r]: <>", "an empty angle destination is valid"),
            ("[a\\]b]: /url", "an escaped bracket stays inside the label"),
            ("[r]: /url \"t\"", "a double-quoted same-line title"),
            ("[r]: /url 't'", "a single-quoted same-line title"),
            ("[r]: /url (t)", "a parenthesized same-line title"),
            ("[r]: </u rl> \"t\"", "an angle destination may hold spaces"),
            ("[r]: /u(r)l", "balanced parens stay in a bare destination"),
        ];
        for (line, why) in accepted {
            let deck = parse(&format!("## Q\nanswer\n{line}\n"));
            assert_eq!(deck.cards[0].back, vec!["answer"], "{why}: {line}");
            assert_eq!(deck.definitions.len(), 1, "{why}: {line}");
        }
        let rejected = [
            ("[r]: /url junk", "trailing prose after the destination"),
            ("[r]: /url \"t\" junk", "trailing prose after the title"),
            ("[r]: /url \"unclosed", "an unclosed same-line title"),
            ("[r]: /u(rl", "an unbalanced bare destination"),
            (
                "[r]: /url (with (nested))",
                "a paren title cannot reopen a paren",
            ),
            (
                "[r]: <>(t)",
                "a title needs whitespace after the destination",
            ),
        ];
        for (line, why) in rejected {
            let deck = parse(&format!("## Q\nanswer\n{line}\n"));
            assert_eq!(
                deck.cards[0].back,
                vec!["answer", line],
                "{why} keeps the line as answer prose: {line}"
            );
            assert!(deck.definitions.is_empty(), "{why}: {line}");
        }
        assert!(
            matches!(
                err("## Q\nanswer\n[r]: <un closed\n"),
                ParseError::TagShape { line: 3, .. }
            ),
            "an unclosed angle stays inside the tag-shape error, never silent prose"
        );
        let deck = parse("## Q\nanswer\n[r]: /url\n\"two\nline title\"\n");
        assert_eq!(
            deck.cards[0].back,
            vec!["answer"],
            "a title may continue across lines until its closer"
        );
        assert_eq!(deck.definitions, vec!["r"]);
        let deck = parse("## Q\nanswer\n[r]: /url\n\"sneaky\n## Q2\nanswer2\n");
        assert_eq!(
            deck.cards[0].back,
            vec!["answer", "\"sneaky"],
            "an unclosed continued title falls back to a destination-only definition"
        );
        assert_eq!(
            deck.cards.len(),
            2,
            "a structural line is never swallowed as title continuation"
        );
        assert_eq!(deck.definitions, vec!["r"]);
        let deck = parse("## Q\nanswer\n[r]:\n    /url with a space\n");
        assert_eq!(
            deck.cards[0].back,
            vec!["answer", "[r]:", "/url with a space"],
            "an indented continuation that does not complete the grammar \
             consumes nothing, indentation included"
        );
        assert!(deck.definitions.is_empty());
    }

    fn err(text: &str) -> ParseError {
        super::parse("deck.md", text).unwrap_err()
    }

    /// Spec law 1 (section-context arc): the heading grammar over depth
    /// times spacing times position. Each row states the OUTCOME, so it
    /// survives whatever the error variants end up being named; the
    /// per-variant assertions live with the code that raises them.
    ///
    /// Read `Cards(n)` as "parses, yielding n cards", `Err(line)` as "a
    /// parse error reported at that line".
    #[derive(Debug, PartialEq)]
    enum Outcome {
        Cards(usize),
        Err(usize),
    }

    fn outcome(text: &str) -> Outcome {
        match super::parse("deck.md", text) {
            Ok(deck) => Outcome::Cards(deck.cards.len()),
            Err(e) => Outcome::Err(e.line()),
        }
    }

    /// Decision 12 (reserved shapes): setext underlines, indented code,
    /// nested quotes, and card thematic breaks are hard errors; each
    /// error row is flanked by the nearest legal neighbor that must stay
    /// legal (fence interiors, paragraph continuations, standalone
    /// shapes, section breaks).
    #[test]
    fn reserved_shapes_error_as_ruled_and_their_neighbors_stay_legal() {
        for (name, deck, expected) in [
            (
                "setext under card prose",
                "## q\nanswer\n===\n",
                Outcome::Err(3),
            ),
            (
                "longer run, trailing space",
                "## q\nanswer\n======  \n",
                Outcome::Err(3),
            ),
            (
                "standalone === is prose",
                "## q\nanswer\n\n===\n",
                Outcome::Cards(1),
            ),
            (
                "setext under section prose",
                "# s\nprose\n===\n## q\na\n",
                Outcome::Err(3),
            ),
            (
                "=== inside a fence",
                "## q\n```\ntext\n===\n```\n",
                Outcome::Cards(1),
            ),
            (
                "=== under a note line stays content",
                "## q\na\n> note\n===\n",
                Outcome::Cards(1),
            ),
            (
                "indented after blank",
                "## q\na\n\n    code\n",
                Outcome::Err(4),
            ),
            (
                "indented as first answer line",
                "## q\n    code\n",
                Outcome::Err(2),
            ),
            (
                "tab indent after blank",
                "## q\na\n\n\tcode\n",
                Outcome::Err(4),
            ),
            (
                "paragraph continuation stays legal",
                "## q\na\n    still prose\n",
                Outcome::Cards(1),
            ),
            (
                "task-list continuation stays legal",
                "## q\n- [x] item\n    more\n",
                Outcome::Cards(1),
            ),
            (
                "indented inside a fence",
                "## q\n```\n    code\n```\n",
                Outcome::Cards(1),
            ),
            (
                "indented in a section after blank",
                "# s\nprose\n\n    code\n## q\na\n",
                Outcome::Err(4),
            ),
            (
                "three-space indent stays legal",
                "## q\na\n\n   shallow\n",
                Outcome::Cards(1),
            ),
            (
                "nested quote in a note",
                "## q\na\n> > deep\n",
                Outcome::Err(3),
            ),
            (
                "nested quote unspaced",
                "## q\na\n>> deep\n",
                Outcome::Err(3),
            ),
            (
                "one-level note stays legal",
                "## q\na\n> note\n",
                Outcome::Cards(1),
            ),
            (
                "nested quote in a section",
                "# s\n> > deep\n## q\na\n",
                Outcome::Err(2),
            ),
            (
                "inner > not at start stays legal",
                "## q\na\n> quoted > inline\n",
                Outcome::Cards(1),
            ),
            ("star break in a card", "## q\na\n***\n", Outcome::Err(3)),
            (
                "underscore break in a card",
                "## q\na\n___\n",
                Outcome::Err(3),
            ),
            (
                "spaced break in a card",
                "## q\na\n* * *\n",
                Outcome::Err(3),
            ),
            (
                "bold-italic line stays legal",
                "## q\n***bold***\n",
                Outcome::Cards(1),
            ),
            (
                "an attached section break is stray, as `---` already was",
                "# s\n***\n## q\na\n",
                Outcome::Err(2),
            ),
            (
                "a blank-surrounded section break is reserved",
                "# s\n\n***\n\n## q\na\n",
                Outcome::Err(3),
            ),
            (
                "break inside a fence",
                "## q\n```\n***\n```\n",
                Outcome::Cards(1),
            ),
            ("two stars stay legal", "## q\na\n**\n", Outcome::Cards(1)),
            (
                "underscore word stays legal",
                "## q\na\n___x\n",
                Outcome::Cards(1),
            ),
        ] {
            assert_eq!(expected, outcome(deck), "{name}: {deck:?}");
        }
    }

    /// Decision 12.5: the two-trailing-space hard break is a non-spelling.
    /// Content lines shed trailing whitespace at scan; fence interiors
    /// stay verbatim.
    #[test]
    fn trailing_spaces_are_stripped_from_content_but_not_fences() {
        let deck = parse("## q\nanswer  \n```\ncode  \n```\n");
        assert_eq!("answer", deck.cards[0].back[0]);
        assert!(
            deck.cards[0].back.iter().any(|l| l == "code  "),
            "fence interior keeps its bytes: {:?}",
            deck.cards[0].back
        );
    }

    /// Spec law 10: the section a card was authored under reaches the
    /// card, heading first, then its prose, including the line classes
    /// that take card-only branches elsewhere in the scanner.
    #[test]
    fn section_context_reaches_every_card_in_its_section() {
        let deck = parse(
            "# Vehicle safety\nApplies on public roads.\n## stopping distance\nanswer\n### wet roads\nlonger\n# Other\n## unrelated\nx\n",
        );
        assert_eq!(
            vec!["Vehicle safety", "Applies on public roads."],
            deck.cards[0].section_context
        );
        let fronts: Vec<&str> = deck.cards.iter().map(|c| c.front.as_str()).collect();
        assert_eq!(
            vec!["stopping distance", "wet roads", "unrelated"],
            fronts,
            "precondition: three cards in document order"
        );
        assert_eq!(
            vec!["Vehicle safety", "Applies on public roads."],
            deck.cards[1].section_context,
            "a sub-card sits in its section too"
        );
        assert_eq!(vec!["Other"], deck.cards[2].section_context);
    }

    /// A body opens with a heading or it does not open. Every line class
    /// that can reach the section must refuse to before the first heading,
    /// or one of them becomes a quiet way back in.
    #[test]
    fn every_line_class_refuses_to_open_a_deck_before_the_first_heading() {
        for (label, body, at) in [
            ("plain prose", "Applies throughout.\n## q\na\n", 1),
            ("a quote", "> quoted\n## q\na\n", 1),
            ("an escape", "\\## escaped\n## q\na\n", 1),
            ("a fence opener", "```\n## q\na\n", 1),
            ("a fence body", "```\ncode\n```\n## q\na\n", 1),
        ] {
            let err = err(body);
            assert!(
                matches!(err, ParseError::ProseBeforeFirstHeading(line) if line == at),
                "{label} before the first heading must error at line {at}, got {err:?}"
            );
        }
    }

    /// The bar is a HEADING, not a section: a deck of plain cards has no
    /// section at all and must still parse.
    #[test]
    fn a_body_that_opens_with_a_card_parses_with_no_section() {
        let deck = parse("## q\na\n");
        assert!(
            deck.cards[0].section_context.is_empty(),
            "a sectionless deck carries no context, got {:?}",
            deck.cards[0].section_context
        );
    }

    /// Prose still joins the section it sits under; only the region before
    /// the first heading is closed.
    #[test]
    fn prose_under_a_heading_is_still_that_sections_context() {
        let deck = parse("# S\nApplies throughout.\n## q\na\n# Later\n## r\nb\n");
        assert_eq!(
            vec!["S", "Applies throughout."],
            deck.cards[0].section_context
        );
        assert_eq!(vec!["Later"], deck.cards[1].section_context);
    }

    /// Every line class that has a card-only branch must still land in the
    /// section when no card is open, or it is silently dropped.
    #[test]
    fn section_prose_keeps_fences_quotes_dividers_and_escapes() {
        let deck = parse("# S\nplain\n> quoted\n\\---\n\\## escaped\n```\ncode\n```\n## q\na\n");
        assert_eq!(
            vec![
                "S",
                "plain",
                "> quoted",
                "---",
                "## escaped",
                "```",
                "code",
                "```"
            ],
            deck.cards[0].section_context
        );
    }

    /// A table's rows are cards, and they sit in the section too.
    #[test]
    fn table_row_cards_carry_their_section() {
        let deck = parse("# Capitals\n## Pairs\n| c | a |\n| --- | --- |\n| DE | Berlin |\n");
        assert_eq!(vec!["Capitals"], deck.cards[0].section_context);
    }

    /// D16: a sidecar has no sections, so a reader's hand-written label
    /// must not become context a personal card then carries and exposes.
    #[test]
    fn a_sidecar_never_turns_leading_headings_into_section_context() {
        let cards = parse_sidecar(
            "deck.personal.md",
            "#### reader label\n## personal\nanswer\n<!-- id: card-personal -->\n",
        )
        .unwrap();
        assert_eq!(1, cards.len());
        assert!(
            cards[0].section_context.is_empty(),
            "got {:?}",
            cards[0].section_context
        );
    }

    /// D19 judges a section directive by its KEY: an invalid value only
    /// lints, and a lint would let the comment be stripped and the author's
    /// intended setting vanish.
    #[test]
    fn a_recognized_directive_on_a_section_errors_whatever_its_value() {
        // Codex's finding: a hand-maintained key list had already drifted
        // past `diagram`, so this sweeps every recognized key and the
        // predicate asks apply_directive rather than keeping a copy.
        for tail in [
            "direction: both",
            "direction: sideways",
            "reveal: line",
            "id: card-x",
            "diagram: xxh64-0123456789abcdef",
            "input: draw",
            "sampling: on",
            "given: partial",
            "at: notes.md:1-2",
            "blank: span hidden=\"x\" b:a1b2c3",
            "cover: 1,2,3,4",
            "crop: 1,2,3,4",
        ] {
            let text = format!("# Topic <!-- {tail} -->\n## question\nanswer\n");
            let e = err(&text);
            assert!(
                matches!(e, ParseError::SectionDirective(1)),
                "tail {tail:?} gave {e:?}"
            );
        }
        // An editorial comment is not a directive and stays allowed.
        let deck = parse("# Topic <!-- editorial note -->\n## q\na\n");
        assert_eq!(1, deck.cards.len());
    }

    /// Every review unit one authored block expands to shares one
    /// `block_line`, and a sub-card points at its parent's. Gating reads
    /// nothing else, so a wrong stamp here silently ungates a whole deck.
    #[test]
    fn every_unit_of_a_block_shares_its_block_line_and_sub_cards_point_at_the_parent() {
        let chain = parse_str(
            "t",
            "## A\na\n\n### B\nb\n\n#### C\nc\n\n# Section\n\n## D\nd\n\n### E\ne\n",
        )
        .expect("the chain parses");
        let seen: Vec<(String, usize, Option<usize>)> = chain
            .iter()
            .map(|card| (card.front.clone(), card.block_line, card.parent_block))
            .collect();
        assert_eq!(
            vec![
                ("A".to_string(), 1, None),
                ("B".to_string(), 4, Some(1)),
                ("C".to_string(), 7, Some(4)),
                ("D".to_string(), 12, None),
                ("E".to_string(), 15, Some(12)),
            ],
            seen,
            "a section clears the chain; depth 4 hangs off depth 3"
        );

        let cloze = parse_str(
            "t",
            "## Q\none and two\n<!-- blank: span hidden=\"one\" -->\n<!-- blank: span hidden=\"two\" -->\n",
        )
        .expect("the blank block parses");
        assert_eq!(2, cloze.len(), "two spans are two units");
        assert!(
            cloze.iter().all(|card| card.block_line == 1),
            "both spans belong to the block on line 1"
        );

        let table = parse_str(
            "t",
            "## Vocabulary\n| word | meaning |\n|---|---|\n| one | eins |\n| two | zwei |\n<!-- cards -->\n",
        )
        .expect("the titled table parses");
        assert_eq!(2, table.len(), "two rows are two units");
        assert!(
            table.iter().all(|card| card.block_line == 1),
            "every row belongs to the title's block, not to its own line"
        );
        assert!(
            table.iter().all(|card| card.parent_block.is_none()),
            "a table title sits at depth 2"
        );

        let reversed = chain[1].reversed();
        assert_eq!(
            (chain[1].block_line, chain[1].parent_block),
            (reversed.block_line, reversed.parent_block),
            "the reverse half is gated with its forward half"
        );
    }

    /// Done-list law: identity is independent of depth, so re-filing a card
    /// under a different heading keeps its token and its fingerprints.
    #[test]
    fn re_heading_a_card_changes_neither_its_id_nor_its_fingerprints() {
        let stamped = "<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->";
        let shallow = parse(&format!("## q\na\n{stamped}\n"));
        let deep = parse(&format!("## parent\np\n### q\na\n{stamped}\n"));
        let a = &shallow.cards[0];
        let b = &deep.cards[1];
        assert_eq!(a.id(), b.id(), "the minted token is the identity");
        assert_eq!(a.content_fingerprint, b.content_fingerprint);
        assert_eq!(a.block_fingerprint, b.block_fingerprint);
    }

    /// Done-list law: moving a card between sections leaves every stamp
    /// untouched, which is what keeps cached AI artifacts valid (D18).
    #[test]
    fn moving_a_card_between_sections_changes_no_fingerprint() {
        let one = parse("# One\nprose one\n## q\na\n");
        let two = parse("# Two\nprose two\n## q\na\n");
        assert_ne!(one.cards[0].section_context, two.cards[0].section_context);
        assert_eq!(
            one.cards[0].content_fingerprint,
            two.cards[0].content_fingerprint
        );
        assert_eq!(
            one.cards[0].block_fingerprint,
            two.cards[0].block_fingerprint
        );
    }

    #[test]
    fn the_heading_grammar_matrix() {
        use Outcome::{Cards, Err};
        // (label, deck text, expected outcome)
        let rows: Vec<(&str, String, Outcome)> = vec![
            // ── depth, in the ordinary position ──
            (
                "h1 opens a section",
                "# S
## q
a
"
                .into(),
                Cards(1),
            ),
            (
                "h2 is a card",
                "## q
a
"
                .into(),
                Cards(1),
            ),
            (
                "h3 is a sub-card of the h2",
                "## q
a
### s
b
"
                .into(),
                Cards(2),
            ),
            (
                "h4 under h3",
                "## q
a
### s
b
#### t
c
"
                .into(),
                Cards(3),
            ),
            (
                "h5 is a sub-card under the full chain",
                "## q
a
### s
b
#### t
c
##### u
d
"
                .into(),
                Cards(4),
            ),
            (
                "h6 completes the six-depth chain",
                "## q
a
### s
b
#### t
c
##### u
d
###### v
e
"
                .into(),
                Cards(5),
            ),
            (
                "h5 skipping h4 is an orphan",
                "## q
a
### s
b
##### u
d
"
                .into(),
                Err(5),
            ),
            (
                "h6 straight under h2 is an orphan",
                "## q
a
###### u
d
"
                .into(),
                Err(3),
            ),
            (
                "seven hashes are ordinary content",
                "## q
a
####### deep
"
                .into(),
                Cards(1),
            ),
            // ── the stack rule ──
            (
                "h4 skipping h3 errors",
                "## q
a
#### t
c
"
                .into(),
                Err(3),
            ),
            (
                "h3 before any h2 is an orphan",
                "### s
b
"
                .into(),
                Err(1),
            ),
            (
                "a closed chain cannot be re-entered",
                "## a
1
### b
2
## c
3
#### d
4
"
                .into(),
                Err(7),
            ),
            (
                "a section closes the chain",
                "## a
1
### b
2
# S
### c
3
"
                .into(),
                Err(6),
            ),
            // ── spacing: the marker needs its space ──
            (
                "h1 without a space is content",
                "## q
a
#tight
"
                .into(),
                Cards(1),
            ),
            (
                "h2 without a space is content",
                "## q
a
##tight
"
                .into(),
                Cards(1),
            ),
            (
                "h3 without a space is content",
                "## q
a
###tight
"
                .into(),
                Cards(1),
            ),
            // ── empty heading text ──
            (
                "an empty section heading resets the context",
                "#\u{20}\n## q\na\n".into(),
                Cards(1),
            ),
            (
                "an empty sub-card front errors",
                "## q\na\n###\u{20}\nb\n".into(),
                Err(3),
            ),
            // ── escapes ──
            (
                "an escaped h1 is content",
                "## q
a
\\# not a section
"
                .into(),
                Cards(1),
            ),
            (
                "an escaped h3 is content",
                "## q
a
\\### not a sub-card
"
                .into(),
                Cards(1),
            ),
            // ── fences keep their contents ──
            (
                "a fenced h1 is content",
                "## q
a
```
# fenced
```
"
                .into(),
                Cards(1),
            ),
            (
                "a fenced h3 is content",
                "## q
a
```
### fenced
```
"
                .into(),
                Cards(1),
            ),
            // ── inside a divided front ──
            (
                "a section closes a front, so its answerless card errors at ITS line",
                "## line one
---
# S
a
"
                .into(),
                Err(1),
            ),
            // ── tables ──
            (
                "a section closes an open table",
                "## Capitals
| c | a |
| --- | --- |
| DE | Berlin |
<!-- cards -->
# Two
## Next
answer
"
                .into(),
                Cards(2),
            ),
            (
                "a sub-card above a table header is reserved",
                "## q
a
### t
| c | a |
| --- | --- |
| DE | Berlin |
<!-- cards -->
"
                .into(),
                Err(3),
            ),
            (
                "a sub-card under a zero-row titled table has no parent",
                "## Empty
| c | a |
| --- | --- |
<!-- cards -->
### Dependent
answer
"
                .into(),
                Err(5),
            ),
            // ── directives and ids on a section line ──
            (
                "a card directive on a section errors",
                "# S <!-- reveal: line -->
## q
a
"
                .into(),
                Err(1),
            ),
            (
                "an id comment on a section errors",
                "# S <!-- id: card-abc -->
## q
a
"
                .into(),
                Err(1),
            ),
            (
                "a stray id in section prose errors",
                "# S
<!-- id: card-abc -->
## q
a
"
                .into(),
                Err(2),
            ),
            // ── normalization ──
            (
                "a section strips its trailing hashes",
                "# S ###
## q
a
"
                .into(),
                Cards(1),
            ),
        ];

        let mut failures = Vec::new();
        for (label, text, expected) in rows {
            let got = outcome(&text);
            if got != expected {
                failures.push(format!("{label}: expected {expected:?}, got {got:?}"));
            }
        }
        assert!(
            failures.is_empty(),
            "grammar matrix:\n  {}",
            failures.join("\n  ")
        );
    }

    fn unknown(line: usize, key: &str) -> Lint {
        Lint {
            line,
            kind: LintKind::UnknownKey { key: key.into() },
        }
    }

    fn bad(line: usize, key: &str, value: &str) -> Lint {
        Lint {
            line,
            kind: LintKind::BadValue {
                key: key.into(),
                value: value.into(),
            },
        }
    }

    // ── Frontmatter ──

    #[test]
    fn frontmatter_opens_only_as_the_first_content_line() {
        let deck = parse("\n---\ntrace: a walk\n---\n## q\n---\na\n");
        assert_eq!(Some("a walk".to_string()), deck.frontmatter.trace);
        assert_eq!(1, deck.cards.len());

        assert_eq!(
            ParseError::StrayDivider(2),
            err("# S\n---\nid: nope\n---\n## q\na\n")
        );
    }

    #[test]
    fn for_names_the_deck_a_personal_file_belongs_to() {
        let deck = parse("---\nfor: deck-abc\n---\n## q\na\n");
        assert_eq!(Some("deck-abc".to_string()), deck.frontmatter.personal_for);
        assert_eq!(Vec::<Lint>::new(), deck.lints, "a known key never lints");
    }

    #[test]
    fn a_non_string_for_lints_rather_than_failing_the_file() {
        let deck = parse("---\nfor: 7\n---\n## q\na\n");
        assert_eq!(None, deck.frontmatter.personal_for);
        assert_eq!(vec![bad(2, "for", "an integer")], deck.lints);
    }

    #[test]
    fn a_missing_frontmatter_close_is_a_hard_error() {
        assert_eq!(
            ParseError::UnclosedFrontmatter(1),
            err("---\nformat-version: 1\nid: \"deck-abc\"\n## q\na\n")
        );
    }

    #[test]
    fn both_ends_of_the_frontmatter_fence_accept_the_same_spellings() {
        for fence in ["---", "--- ", "---\t"] {
            let deck = parse(&format!("{fence}\ntrace: a walk\n{fence}\n## q\na\n"));
            assert_eq!(
                Some("a walk".to_string()),
                deck.frontmatter.trace,
                "`{fence}` must open and close the fence"
            );
            assert_eq!(1, deck.cards.len(), "`{fence}` must leave one card");
        }
        for fence in [" ---", "----", "-- -"] {
            assert_eq!(
                ParseError::StrayDivider(1),
                err(&format!("{fence}\ntrace: a walk\n---\n## q\na\n")),
                "`{fence}` must not open the fence"
            );
            assert_eq!(
                ParseError::UnclosedFrontmatter(1),
                err(&format!("---\ntrace: a walk\n{fence}\n## q\na\n")),
                "`{fence}` must not close the fence"
            );
        }
    }

    #[test]
    fn a_frontmatter_closer_tolerates_trailing_whitespace() {
        let deck = parse("---\ntrace: a walk\n--- \n## q\na\n");
        assert_eq!(Some("a walk".to_string()), deck.frontmatter.trace);
        assert_eq!(1, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);

        assert_eq!(
            ParseError::UnclosedFrontmatter(1),
            err("---\ntrace: a walk\n ---\n## q\na\n")
        );
    }

    #[test]
    fn a_blank_line_before_the_frontmatter_closer_is_accepted() {
        let deck = parse(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n\n---\n## q\na\n",
        );
        assert_eq!(
            Some("deck-9w2c7x4k1m8q3z5t0v6b2n4d8f"),
            deck.deck_token.as_deref()
        );
        assert_eq!(Some((1, 5)), deck.frontmatter_span);
        assert_eq!(1, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);
    }

    #[test]
    fn an_unquoted_numeric_id_is_a_hard_error_naming_the_line() {
        assert_eq!(
            ParseError::NonStringId {
                line: 2,
                found: "an integer"
            },
            err("---\nid: 007\n---\n## q\na\n")
        );
    }

    #[test]
    fn a_bool_id_is_a_hard_error() {
        assert_eq!(
            ParseError::NonStringId {
                line: 2,
                found: "a boolean"
            },
            err("---\nid: true\n---\n## q\na\n")
        );
    }

    #[test]
    fn a_quoted_prefixed_id_parses_verbatim() {
        let deck = parse(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n",
        );
        assert_eq!(
            Some("deck-9w2c7x4k1m8q3z5t0v6b2n4d8f"),
            deck.deck_token.as_deref()
        );
        assert_eq!(deck.deck_token, deck.frontmatter.id);
        assert!(!deck.frontmatter.unspliceable);
    }

    #[test]
    fn an_initialized_deck_without_a_version_parses() {
        let deck = parse("---\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n");
        assert_eq!(
            Some("deck-9w2c7x4k1m8q3z5t0v6b2n4d8f"),
            deck.deck_token.as_deref()
        );
    }

    #[test]
    fn a_deck_version_from_a_newer_alix_is_refused_by_number() {
        assert_eq!(
            ParseError::UnsupportedDeckVersion {
                line: 2,
                version: 2
            },
            err("---\nformat-version: 2\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n")
        );
    }

    #[test]
    fn a_version_of_one_parses() {
        let deck = parse(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n",
        );
        assert_eq!(
            Some("deck-9w2c7x4k1m8q3z5t0v6b2n4d8f"),
            deck.deck_token.as_deref()
        );
    }

    #[test]
    fn deck_metadata_keys_parse_as_single_values_or_lists() {
        let deck = parse(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\nauthors: Alex\nlicense: CC-BY-4.0\ntitle: Memory\ndescription: |\n  Two lines\n  of description.\n---\n## q\na\n",
        );
        assert_eq!(vec!["Alex".to_string()], deck.frontmatter.authors);
        assert_eq!(Some("CC-BY-4.0"), deck.frontmatter.license.as_deref());
        assert_eq!(Some("Memory"), deck.frontmatter.title.as_deref());
        assert_eq!(
            Some("Two lines\nof description.\n"),
            deck.frontmatter.description.as_deref()
        );
    }

    /// The classification key was deferred, so the word that used to be
    /// parsed now takes the ordinary unknown-key lint.
    #[test]
    fn a_tags_key_is_no_longer_recognized() {
        let deck = parse(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\ntags: [rust]\n---\n## q\na\n",
        );
        assert!(
            deck.lints.iter().any(|l| matches!(
                &l.kind,
                LintKind::UnknownKey { key } if key == "tags"
            )),
            "lints: {:?}",
            deck.lints
        );
    }

    #[test]
    fn several_authors_are_accepted_as_a_list() {
        let deck = parse(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\nauthors:\n  - Alex\n  - Sam\n---\n## q\na\n",
        );
        assert_eq!(
            vec!["Alex".to_string(), "Sam".to_string()],
            deck.frontmatter.authors
        );
    }

    #[test]
    fn a_deck_without_frontmatter_parses_with_no_id() {
        let deck = parse("## q\na\n");
        assert_eq!(None, deck.frontmatter.id);
    }

    #[test]
    fn an_unrecognized_id_spelling_is_an_ordinary_unknown_key() {
        let deck = super::parse(
            "deck.md",
            "---\nalix-id: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n",
        )
        .unwrap();
        assert_eq!(None, deck.deck_token);
        assert!(
            deck.lints.contains(&unknown(2, "alix-id")),
            "{:?}",
            deck.lints
        );
    }

    #[test]
    fn a_bare_token_id_is_a_hard_error_not_an_unstamped_deck() {
        assert_eq!(
            ParseError::InvalidDeckId {
                line: 2,
                value: "9w2c7x4k1m8q3z5t0v6b2n4d8f".into()
            },
            err("---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n")
        );
    }

    #[test]
    fn a_card_prefixed_frontmatter_id_is_a_hard_error() {
        assert_eq!(
            ParseError::InvalidDeckId {
                line: 2,
                value: "card-9w2c7x4k1m8q3z5t0v6b2n4d8f".into()
            },
            err("---\nid: \"card-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n")
        );
    }

    #[test]
    fn a_flow_mapping_frontmatter_parses_but_is_reported_unspliceable() {
        let deck = parse("---\n{source: [a]}\n---\n## q\nb\n");
        assert_eq!(vec!["a".to_string()], deck.frontmatter.source);
        assert!(deck.frontmatter.unspliceable);
    }

    #[test]
    fn a_null_scalar_frontmatter_is_unspliceable() {
        let deck = parse("---\nnull\n---\n## q\na\n");
        assert_eq!(None, deck.frontmatter.id);
        assert!(deck.frontmatter.unspliceable);

        let deck = parse("---\n~\n---\n## q\na\n");
        assert_eq!(None, deck.frontmatter.id);
        assert!(deck.frontmatter.unspliceable);
    }

    #[test]
    fn the_frontmatter_span_locates_the_fences_or_is_none() {
        assert_eq!(None, parse("## q\na\n").frontmatter_span);
        assert_eq!(
            Some((1, 3)),
            parse("---\nsource: x\n---\n## q\na\n").frontmatter_span
        );
        assert_eq!(
            Some((2, 4)),
            parse("\n---\nsource: x\n---\n## q\na\n").frontmatter_span
        );
        let deck = parse("---\n{source: [a]}\n---\n## q\nb\n");
        assert_eq!(Some((1, 3)), deck.frontmatter_span);
        assert!(deck.frontmatter.unspliceable);
    }

    #[test]
    fn an_id_failing_the_charset_is_a_line_numbered_error() {
        assert_eq!(
            ParseError::InvalidDeckId {
                line: 3,
                value: "deck-ABC".into()
            },
            err("---\nformat-version: 1\nid: \"deck-ABC\"\n---\n## q\na\n")
        );
    }

    #[test]
    fn unknown_frontmatter_keys_are_linted_reserved_keys_are_not() {
        let deck = parse(
            "---\nlicense: MIT\nauthors: me\nlanguage: de\nrevision: 3\n\
             created-at: 2026-07-19\nfnord: 7\n---\n## q\na\n",
        );
        assert_eq!(vec![unknown(7, "fnord")], deck.lints);
    }

    #[test]
    fn invalid_frontmatter_yaml_is_a_hard_error() {
        let e = err("---\nid: [unclosed\n---\n## q\na\n");
        assert!(matches!(e, ParseError::FrontmatterSyntax { .. }), "{e:?}");
    }

    #[test]
    fn a_second_yaml_document_smuggled_by_a_carriage_return_is_rejected() {
        let e = err("---\n:\r---\n---\n## q\na\n");
        let ParseError::FrontmatterSyntax { message, .. } = e else {
            panic!("expected FrontmatterSyntax, got {e:?}");
        };
        assert!(message.contains("more than one yaml document"), "{message}");
    }

    #[test]
    fn an_empty_frontmatter_is_fine() {
        let deck = parse("---\n---\n## q\na\n");
        assert_eq!(Frontmatter::default(), deck.frontmatter);
        assert!(!deck.frontmatter.unspliceable);
    }

    /// `ImageReference.source` is consumed as a FILENAME by
    /// `assets::validate_image_at_root`, `share`, `explore` and `doctor`, so it
    /// has to be the resolved path rather than the bytes as typed.
    #[test]
    fn an_image_source_is_the_resolved_path_the_asset_lookup_will_use() {
        for (line, expected_source, expected_typed) in [
            (r"![](my\(file\).png)", "my(file).png", r"my\(file\).png"),
            (r"![](<a\>b.png>)", "a>b.png", r"a\>b.png"),
            ("![](  spaced.png  )", "spaced.png", "  spaced.png  "),
        ] {
            let found = image_references(line);
            assert_eq!(1, found.len(), "for {line}");
            assert_eq!(expected_source, found[0].source, "for {line}");
            assert_eq!(expected_typed, &line[found[0].destination.clone()]);
        }
    }

    #[test]
    fn directive_keys_are_nonempty_single_words() {
        assert_eq!(None, directive(": value"));
        assert_eq!(None, directive("bad key: value"));
        assert_eq!(
            Some(("key".into(), "value".into())),
            directive("KEY: value")
        );
    }

    #[test]
    fn frontmatter_lists_accept_a_scalar_as_a_singleton() {
        let deck = parse("---\nsource: notes.md\nrequires: basics\n---\n## q\na\n");
        assert_eq!(vec!["notes.md".to_string()], deck.frontmatter.source);
        assert_eq!(vec!["basics".to_string()], deck.frontmatter.requires);
    }

    // ── Document structure ──

    /// A decks folder may hold ordinary documents. Only a file that claims
    /// to be a deck (frontmatter, or real cards) may fail as one.
    #[test]
    fn an_ordinary_document_that_opens_with_prose_is_not_deck_content() {
        assert!(
            !is_deck_content("Just my notes.\n\nNothing to do with alix.\n"),
            "a plain note in a decks folder is not a deck"
        );
        assert!(
            is_deck_content("---\ntitle: T\n---\n\nstray prose\n\n## q\na\n"),
            "frontmatter claims deckhood, so the same mistake must surface"
        );
        assert!(
            is_deck_content("## q\na\n"),
            "a card is a claim too, with or without frontmatter"
        );
    }

    #[test]
    fn a_file_with_no_h2_fronts_is_a_zero_card_deck() {
        let deck = parse("---\ntitle: Title\n---\n# A section\njust prose\n");
        assert!(deck.cards.is_empty());
        assert_eq!(Some("Title"), deck.title.as_deref());
    }

    #[test]
    fn is_deck_content_requires_a_card_or_frontmatter() {
        assert!(!is_deck_content("# Notes\n\njust some prose here\n"));
        assert!(!is_deck_content("# Notes\n\n```\n## not a card\n```\n"));
        assert!(is_deck_content("## q\na\n"));
    }

    #[test]
    fn deck_identity_requires_a_prefixed_id_in_opening_frontmatter() {
        let id = "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f";
        assert_eq!(
            Ok(Some(id.to_string())),
            deck_identity(&format!(
                "---\nformat-version: 1\nid: \"{id}\"\n---\n## q\na\n"
            ))
        );
        assert_eq!(Ok(None), deck_identity("## q\nid: \"abc\"\na\n"));
        assert_eq!(
            Ok(None),
            deck_identity("---\nsource: notes.md\n---\n## q\na\n")
        );
        assert!(matches!(
            deck_identity("---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n"),
            Err(ParseError::InvalidDeckId { .. })
        ));
        assert!(matches!(
            deck_identity("---\nformat-version: 1\nid: \"deck-ABC\"\n---\n## q\na\n"),
            Err(ParseError::InvalidDeckId { .. })
        ));
        assert!(matches!(
            deck_identity("---\nalix-id: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n"),
            Ok(None)
        ));
    }

    #[test]
    fn deck_identity_survives_a_malformed_card_body() {
        let id = "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f";
        let text = format!("---\nformat-version: 1\nid: \"{id}\"\n---\n## unanswered\n");
        assert!(super::parse("deck.md", &text).is_err());
        assert_eq!(Ok(Some(id.to_string())), deck_identity(&text));
    }

    #[test]
    fn a_header_only_stub_is_deck_content() {
        assert!(is_deck_content("---\ntrace: a walk\n---\n"));
        assert!(is_deck_content("---\nsource: notes.md\n---\n"));
    }

    #[test]
    fn the_title_comes_from_frontmatter_and_an_h1_is_ordinary_section_context() {
        let deck =
            parse("---\ntitle: My Deck\n---\n# A section\nsome intro prose\n\n## q\n---\na\n");
        assert_eq!(Some("My Deck"), deck.title.as_deref());
        assert_eq!(1, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(vec!["a"], deck.cards[0].back);
    }

    /// Prose outside a card no longer becomes a deck description: the
    /// description is a frontmatter key, and body prose is section context.
    /// These three shapes must all still parse to their cards.
    #[test]
    fn body_prose_never_becomes_deck_metadata() {
        for text in ["# T\nline one\nline two\n\n## q\na\n", "# T\n\n## q\na\n"] {
            let deck = parse(text);
            assert_eq!(None, deck.title, "no title without frontmatter: {text:?}");
            assert_eq!(1, deck.cards.len(), "{text:?}");
        }
        assert!(
            matches!(
                err("just an intro\n\n## q\na\n"),
                ParseError::ProseBeforeFirstHeading(1)
            ),
            "an intro line has no heading to belong to, so it cannot be metadata either"
        );
    }

    #[test]
    fn a_card_runs_from_its_h2_to_the_next_h2_or_eof() {
        let deck = parse("## first\nalpha\nbeta\n## second\ngamma\n");
        assert_eq!(2, deck.cards.len());
        assert_eq!("first", deck.cards[0].front);
        assert_eq!(vec!["alpha", "beta"], deck.cards[0].back);
        assert_eq!(1, deck.cards[0].line);
        assert_eq!("second", deck.cards[1].front);
        assert_eq!(vec!["gamma"], deck.cards[1].back);
        assert_eq!(4, deck.cards[1].line);
    }

    #[test]
    fn an_h2_inside_a_fence_does_not_open_a_card() {
        let deck = parse("## q\n---\n```\n## not a front\n```\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["```", "## not a front", "```"], deck.cards[0].back);
    }

    #[test]
    fn fence_openers_carry_their_run_and_closers_need_at_least_it() {
        assert_eq!(Some(('`', 3)), fence_opener("```rust"));
        assert_eq!(Some(('~', 4)), fence_opener("~~~~ info"));
        assert_eq!(None, fence_opener("``"));
        assert!(closes_fence("`````", '`', 4), "longer closes");
        assert!(
            closes_fence("````\t ", '`', 4),
            "trailing whitespace closes"
        );
        assert!(!closes_fence("```", '`', 4), "shorter never closes");
        assert!(
            !closes_fence("```` x", '`', 4),
            "trailing text is not a closer"
        );
        assert!(
            !closes_fence("~~~~", '`', 4),
            "the other character never closes"
        );
    }

    #[test]
    fn a_shorter_delimiter_inside_a_longer_fence_is_content() {
        for (label, text) in [
            (
                "backtick",
                "## q\n---\n````\n```\n## not a front\n```\n````\ntail\n",
            ),
            (
                "tilde",
                "## q\n---\n~~~~\n~~~\n## not a front\n~~~\n~~~~\ntail\n",
            ),
        ] {
            let deck = parse(text);
            assert_eq!(1, deck.cards.len(), "{label}: nothing leaks out");
            assert!(
                deck.cards[0].back.iter().any(|l| l == "## not a front"),
                "{label}: the inner heading stays fenced content, got {:?}",
                deck.cards[0].back
            );
        }
    }

    #[test]
    fn a_shorter_delimiter_never_closes_a_longer_fence() {
        let deck = parse("## q\n---\n````\ncode\n```\n");
        assert_eq!(
            vec![Lint {
                line: 3,
                kind: LintKind::UnclosedFence
            }],
            deck.lints,
            "a three-run delimiter cannot close a four-run fence"
        );
    }

    #[test]
    fn a_longer_delimiter_still_closes_a_shorter_fence() {
        let deck = parse("## q\n---\n```\ncode\n`````\nafter\n");
        assert!(deck.lints.is_empty(), "{:?}", deck.lints);
    }

    #[test]
    fn an_unclosed_fence_at_eof_is_linted() {
        let deck = parse("## q\n---\na\n```\nb\n");
        assert_eq!(vec!["a", "```", "b", ""], deck.cards[0].back);
        assert_eq!(
            vec![Lint {
                line: 4,
                kind: LintKind::UnclosedFence
            }],
            deck.lints
        );
    }

    #[test]
    fn a_fence_closer_with_trailing_text_stays_inside_the_fence() {
        let deck = parse("## q\n---\nbefore\n```\n```rust\n## x\n```\nafter\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(
            vec!["before", "```", "```rust", "## x", "```", "after"],
            deck.cards[0].back
        );
        assert!(deck.lints.is_empty(), "{:?}", deck.lints);
    }

    #[test]
    fn an_indented_h2_is_content_and_linted() {
        let deck = parse("## q\n  ## indented\n");
        assert_eq!(vec!["## indented"], deck.cards[0].back);
        assert_eq!(
            vec![Lint {
                line: 2,
                kind: LintKind::IndentedH2
            }],
            deck.lints
        );
    }

    #[test]
    fn a_trailing_hash_run_is_stripped_from_the_front() {
        let deck = parse("## Foo ##\nbar\n");
        assert_eq!("Foo", deck.cards[0].front);
    }

    #[test]
    fn an_unescaped_trailing_run_still_strips() {
        let deck = parse("## Foo ##\nbar\n");
        assert_eq!("Foo", deck.cards[0].front);
    }

    #[test]
    fn an_escaped_trailing_hash_survives_in_the_front() {
        let deck = parse("## delimited by a \\#\nbar\n");
        assert_eq!("delimited by a #", deck.cards[0].front);
    }

    #[test]
    fn escaped_and_unescaped_mixed() {
        let deck = parse("## Foo \\# ##\nbar\n");
        assert_eq!("Foo #", deck.cards[0].front);
    }

    #[test]
    fn a_mid_line_escaped_hash_unescapes() {
        let deck = parse("## use \\#tags here\nbar\n");
        assert_eq!("use #tags here", deck.cards[0].front);
    }

    #[test]
    fn an_escaped_trailing_hash_does_not_leak_into_the_fingerprint() {
        let deck = parse("## delimited by a \\#\nanswer\n");
        let expected = content_fingerprint("delimited by a #", &["answer".to_string()]);
        assert_eq!(expected, deck.cards[0].content_fingerprint);
    }

    #[test]
    fn a_card_with_no_answer_is_an_error() {
        assert_eq!(ParseError::FrontWithoutAnswer(1), err("## q\n## r\nb\n"));
        assert_eq!(ParseError::FrontWithoutAnswer(1), err("## q\n"));
    }

    // ── Divider, answer, notes ──

    #[test]
    fn the_first_bare_divider_splits_front_from_answer() {
        let deck = parse("## Q\nmore question\n\n---\nthe answer\n");
        assert_eq!("Q\nmore question", deck.cards[0].front);
        assert_eq!(vec!["the answer"], deck.cards[0].back);
    }

    /// A dash family narrower than the star and underscore families did not
    /// error on the odd spellings, it silently declined to divide: the question
    /// text below the heading became answer text, so the card drilled something
    /// its author never wrote.
    #[test]
    fn four_dashes_divide_the_same_multiline_front_as_four_underscores() {
        let expected = parse("## question\nfront detail\n\n____\nanswer\n");
        let actual = parse("## question\nfront detail\n\n----\nanswer\n");
        assert_eq!(
            (&expected.cards[0].front, &expected.cards[0].back),
            (&actual.cards[0].front, &actual.cards[0].back)
        );
    }

    #[test]
    fn spaced_dashes_divide_the_same_multiline_front_as_spaced_stars() {
        let expected = parse("## question\nfront detail\n\n* * *\nanswer\n");
        let actual = parse("## question\nfront detail\n\n- - -\nanswer\n");
        assert_eq!(
            (&expected.cards[0].front, &expected.cards[0].back),
            (&actual.cards[0].front, &actual.cards[0].back)
        );
    }

    #[test]
    fn a_later_break_line_in_an_answer_needs_the_escape() {
        let deck = parse("## Q\n\n---\na\n\\---\n\\----\nb\n");
        assert_eq!(vec!["a", "---", "----", "b"], deck.cards[0].back);
        for spelling in ["---", "----", "- - -", "***", "___"] {
            assert_eq!(
                ParseError::StrayDivider(5),
                err(&format!("## Q\n\n---\na\n{spelling}\nb\n")),
                "an unescaped break below a divided front is a stray: {spelling}"
            );
        }
    }

    // ── The ruled `---` grammar (element 41 / D22 amendment) ──

    #[test]
    fn an_attached_divider_right_under_the_heading_divides() {
        let deck = parse("## q\n---\na\n");
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(vec!["a"], deck.cards[0].back);
    }

    #[test]
    fn the_stray_break_message_never_excludes_a_heading_only_front() {
        let deck = parse("## q\n---\na\n");
        assert_eq!("q", deck.cards[0].front, "a one-line front divides");

        let message = err("## q\na\n\n---\n").to_string();
        assert!(
            !message.contains("multi-line"),
            "the message must not name a grammar narrower than the parser accepts: {message}"
        );
    }

    #[test]
    fn a_break_in_neither_divider_nor_section_position_errors() {
        for (name, text, line) in [
            (
                "divider then blank then prose (probe p15)",
                "## q\nans\n---\n\nprose\n",
                3,
            ),
            ("blank after, heading before", "## q\n---\n\na\n", 2),
            (
                "attached but prose directly above",
                "## q\ntext\n---\nanswer\n",
                3,
            ),
            (
                "attached inside an already-divided card",
                "## q\n\n---\na\n---\nb\n",
                5,
            ),
            (
                "attached in section prose",
                "# S\n---\ntext\n\n## q\na\n",
                2,
            ),
            (
                "leading the body directly after frontmatter",
                "---\ntitle: T\n---\n---\n\n## q\na\n",
                4,
            ),
            ("dangling at EOF under the heading", "## q\n---\n", 2),
        ] {
            assert_eq!(
                ParseError::StrayDivider(line),
                super::parse("deck.md", text).unwrap_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_break_after_a_card_table_is_read_by_the_outer_grammar() {
        let table = "# S\n\n| front | back |\n|---|---|\n| one | two |\n<!-- cards -->\n";
        for spelling in ["---", "----", "- - -", "***", "___"] {
            assert_eq!(
                ParseError::StrayDivider(8),
                err(&format!("{table}\n{spelling}\n\n## q\na\n")),
                "a blank-surrounded break after a table is reserved, not trailing prose: {spelling}"
            );

            let deck = parse(&format!("{table}\n{spelling}\n<!-- plain -->\n\n## q\na\n"));
            assert_eq!(
                vec!["S".to_string(), spelling.to_string()],
                deck.cards
                    .last()
                    .expect("the deck holds cards")
                    .section_context,
                "a plain-marked break after a table is literal section content: {spelling}"
            );
        }

        assert_eq!(
            ParseError::TableTrailing(8),
            err(&format!("{table}\nprose\n\n## q\na\n")),
            "ordinary trailing prose after a table is still refused"
        );
    }

    #[test]
    fn a_break_takes_its_meaning_from_position_not_from_its_spelling() {
        for spelling in ["---", "----", "- - -", "***", "___"] {
            let deck = parse(&format!("## a\n{spelling}\nanswer\n"));
            assert_eq!(
                vec!["answer".to_string()],
                deck.cards[0].back,
                "attached in a card divides the front: {spelling}"
            );

            assert_eq!(
                ParseError::StrayDivider(4),
                err(&format!("# S\nctx\n\n{spelling}\n\n## q\na\n")),
                "blank-surrounded in a section errors: {spelling}"
            );

            assert_eq!(
                ParseError::StrayDivider(4),
                err(&format!("## a\nx\n\n{spelling}\n\ny\n")),
                "blank-surrounded in a card errors: {spelling}"
            );

            assert_eq!(
                ParseError::StrayDivider(3),
                err(&format!("# S\nctx\n{spelling}\n\n## q\na\n")),
                "attached with no card open is stray: {spelling}"
            );

            let deck = parse(&format!("## a\n{spelling}\n<!-- plain -->\nanswer\n"));
            assert_eq!(
                vec![spelling.to_string(), "answer".to_string()],
                deck.cards[0].back,
                "a trailing `<!-- plain -->` keeps the break literal: {spelling}"
            );

            assert_eq!(
                ParseError::LeadingInvocation {
                    line: 2,
                    word: "plain".to_string()
                },
                err(&format!("## a\n<!-- plain -->\n{spelling}\nanswer\n")),
                "a leading `<!-- plain -->` is machinery out of order: {spelling}"
            );
        }
    }

    #[test]
    fn only_three_dashes_open_frontmatter() {
        for spelling in ["----", "- - -", "***", "___"] {
            assert_eq!(
                ParseError::StrayDivider(1),
                err(&format!(
                    "{spelling}\nid: \"deck-abc\"\n{spelling}\n\n## q\na\n"
                )),
                "a non-dash break on line 1 is not a frontmatter fence: {spelling}"
            );
        }
    }

    #[test]
    fn a_bare_hash_resets_the_section_context() {
        let deck = parse("# S\nctx\n\n## a\nx\n\n#\n\n## b\ny\n");
        assert_eq!(
            vec!["S".to_string(), "ctx".to_string()],
            deck.cards[0].section_context,
            "the card before the reset keeps its context"
        );
        assert!(
            deck.cards[1].section_context.is_empty(),
            "the card after the reset carries none, got {:?}",
            deck.cards[1].section_context
        );
        assert!(deck.lints.is_empty(), "a reset draws no lint");
    }

    #[test]
    fn a_reset_with_nothing_after_it_changes_no_parse() {
        for text in ["## a\nx\n\n#\n", "## a\nx\n\n#\n\n#\n"] {
            let deck = parse(text);
            assert_eq!(1, deck.cards.len(), "one card: {text:?}");
            assert_eq!(
                vec!["x".to_string()],
                deck.cards[0].back,
                "the answer is untouched: {text:?}"
            );
            assert!(
                deck.lints.is_empty(),
                "a reset that resets nothing draws no lint: {text:?}, got {:?}",
                deck.lints
            );
        }
    }

    #[test]
    fn a_bare_hash_attached_inside_a_card_errors() {
        assert_eq!(
            ParseError::ContextResetInCard(3),
            err("## a\nmore\n#\n---\nanswer\n")
        );
    }

    #[test]
    fn prose_after_a_reset_joins_the_emptied_section() {
        let deck = parse("# S\n\n## a\nx\n\n#\n\nstray\n\n## b\ny\n");
        assert_eq!(
            vec!["stray".to_string()],
            deck.cards[1].section_context,
            "prose after a reset opens the new, empty context"
        );
    }

    #[test]
    fn a_break_that_closes_a_front_without_an_answer_errors() {
        assert_eq!(
            ParseError::StrayDivider(4),
            err("# S\n## q\n\n---\n\n## r\ny\n")
        );
    }

    #[test]
    fn the_backslash_escape_reaches_every_break_spelling() {
        for spelling in ["---", "----", "- - -", "***", "___"] {
            let deck = parse(&format!("## q\nans\n\\{spelling}\nmore\n"));
            assert_eq!(
                vec!["ans", spelling, "more"],
                deck.cards[0].back,
                "a backslash keeps a break line literal, and drops itself: {spelling}"
            );

            let deck = parse(&format!("# S\n\\{spelling}\n\n## q\na\n"));
            assert_eq!(
                vec!["S".to_string(), spelling.to_string()],
                deck.cards[0].section_context,
                "the same escape works in a section: {spelling}"
            );
        }
    }

    #[test]
    fn a_plain_marked_break_line_is_literal_content() {
        for spelling in ["---", "----", "- - -", "***", "___"] {
            let deck = parse(&format!(
                "## q\nans\n\n{spelling}\n<!-- plain -->\n\nmore\n"
            ));
            assert_eq!(
                vec!["ans", spelling, "more"],
                deck.cards[0].back,
                "trailing plain keeps a break literal in a card: {spelling}"
            );

            let deck = parse(&format!("# S\n\n{spelling}\n<!-- plain -->\n\n## q\na\n"));
            assert_eq!(
                vec!["S".to_string(), spelling.to_string()],
                deck.cards[0].section_context,
                "trailing plain keeps a break literal in a section: {spelling}"
            );
        }
    }

    #[test]
    fn an_eol_terminated_hash_run_is_a_heading_so_empty_text_resets() {
        for text in ["#\n\n## q\na\n", "# \n\n## q\na\n"] {
            let deck = parse(text);
            assert!(
                deck.cards[0].section_context.is_empty(),
                "a bare `#` opens an empty context, got {:?}",
                deck.cards[0].section_context
            );
        }
        assert_eq!(ParseError::EmptyFront(2), err("# S\n##\n"));
        assert_eq!(
            ParseError::ProseBeforeFirstHeading(1),
            err("#foo\n## q\na\n"),
            "no space and no EOL after the run stays prose, per CommonMark"
        );
    }

    #[test]
    fn a_sub_card_chain_does_not_cross_a_context_reset() {
        assert_eq!(
            ParseError::OrphanSubCard(8),
            err("# S\n\n## p\na\n\n#\n\n### s\nb\n")
        );
    }

    #[test]
    fn consecutive_quote_lines_concatenate_into_the_note() {
        let deck = parse("## q\n---\nans\n> [!NOTE]\n> one\n> two\n");
        assert_eq!(Some("one\ntwo"), deck.cards[0].only_note());
    }

    #[test]
    fn an_all_task_list_answer_is_a_single_correct_checkbox_card() {
        let deck =
            parse("## Which is prime?\n- [ ] 4\n- [x] 5\n- [ ] 6\n<!-- choices-single -->\n");
        let card = &deck.cards[0];
        assert_eq!(vec!["5"], card.back);
        assert_eq!(
            vec!["4".to_string(), "6".to_string()],
            card.authored_distractors
        );
        assert!(deck.lints.is_empty(), "{:?}", deck.lints);
    }

    // ── The opt-in mapping doctrine (task #164): bare is literal, ──
    // ── invocation is named, loudness follows invocation (A1/A2)   ──

    #[test]
    fn a_bare_task_list_is_literal_answer_content() {
        let deck = parse("## q\n- [x] a\n- [ ] b\n");
        assert_eq!(vec!["- [x] a", "- [ ] b"], deck.cards[0].back);
        assert!(deck.cards[0].authored_distractors.is_empty());
        assert_eq!(Vec::<Lint>::new(), deck.lints, "bare is literal and silent");
    }

    #[test]
    fn an_invocation_below_the_task_list_makes_the_choice_card() {
        let deck = parse("## q\n- [x] a\n- [ ] b\n<!-- choices-single -->\n");
        assert_eq!(vec!["a"], deck.cards[0].back);
        assert_eq!(vec!["b"], deck.cards[0].authored_distractors);
        assert_eq!(Vec::<Lint>::new(), deck.lints);
    }

    #[test]
    fn a_single_invocation_with_two_checks_fails_loudly() {
        let error = err("## q\n- [x] a\n- [x] b\n- [ ] c\n<!-- choices-single -->\n");
        assert!(
            matches!(error, ParseError::ChoiceShape { line: 2, .. }),
            "expected ChoiceShape at the first task line, got {error:?}"
        );
    }

    #[test]
    fn a_multiple_invocation_collects_every_checked_option() {
        let deck = parse("## q\n- [x] a\n- [ ] b\n- [x] c\n<!-- choices-multiple -->\n");
        assert_eq!(vec!["a", "c"], deck.cards[0].back);
        assert_eq!(vec!["b"], deck.cards[0].authored_distractors);
        assert_eq!(Vec::<Lint>::new(), deck.lints);
    }

    #[test]
    fn one_check_under_multiple_is_legal_and_silent() {
        let deck = parse("## q\n- [x] a\n- [ ] b\n<!-- choices-multiple -->\n");
        assert_eq!(vec!["a"], deck.cards[0].back);
        assert_eq!(vec!["b"], deck.cards[0].authored_distractors);
        assert_eq!(
            Vec::<Lint>::new(),
            deck.lints,
            "a one-answer select-all is legal"
        );
    }

    #[test]
    fn a_bare_pipe_table_is_literal_not_cards() {
        let deck = parse("# S\n| a | b |\n|---|---|\n| x | y |\n\n## q\nans\n");
        assert_eq!(1, deck.cards.len(), "no row cards from a bare table");
        assert!(
            deck.cards[0]
                .section_context
                .contains(&"| x | y |".to_string()),
            "the table rides the section literally, got {:?}",
            deck.cards[0].section_context
        );
    }

    #[test]
    fn an_invoked_table_still_births_row_cards() {
        let deck = parse("# S\n\n| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!("x", deck.cards[0].front);
    }

    #[test]
    fn the_comment_order_repair_reorders_machinery_runs_only() {
        let rows: [(&str, Reorder, &str); 5] = [
            (
                "## q\na\n<!-- id: card-x -->\n<!-- reveal: line -->\n",
                Reorder::Reordered(
                    "## q\na\n<!-- reveal: line -->\n<!-- id: card-x -->\n".into(),
                ),
                "the id moves last within its machinery run",
            ),
            (
                "## q\na\n<!-- reveal: line -->\n<!-- id: card-x -->\n",
                Reorder::Unchanged,
                "the canonical order stays untouched",
            ),
            (
                "## q\n```\n<!-- id: card-x -->\n<!-- reveal: line -->\n```\n",
                Reorder::Unchanged,
                "fence interiors are content, never machinery",
            ),
            (
                "## q\na\n<!-- id: card-x -->\n<!-- an editorial aside -->\n<!-- reveal: line -->\n",
                Reorder::Unchanged,
                "an editorial comment bounds the runs, so nothing crosses it",
            ),
            (
                "## q\n- [x] a\n- [ ] b\n<!-- id: card-x -->\n<!-- at: src/x.rs:1-2 -->\n<!-- blank: span hidden=\"a\" -->\n<!-- reveal: line -->\n<!-- choices-single -->\n",
                Reorder::Reordered(
                    "## q\n- [x] a\n- [ ] b\n<!-- choices-single -->\n<!-- reveal: line -->\n<!-- blank: span hidden=\"a\" -->\n<!-- at: src/x.rs:1-2 -->\n<!-- id: card-x -->\n".into(),
                ),
                "the full canonical order: invocation, directives, regions, locator, id",
            ),
        ];
        for (input, expected, law) in rows {
            assert_eq!(expected, reorder_card_comments(input), "{law}");
        }
    }

    #[test]
    fn a_trailing_invocation_maps_its_table() {
        let below = parse("# S\n\n| a | b |\n|---|---|\n| x | y |\n| u | v |\n<!-- cards -->\n");
        assert_eq!(2, below.cards.len(), "a trailing invocation maps its table");
        assert_eq!("x", below.cards[0].front);
    }

    /// The discriminating law: an invocation must not carry forward into a
    /// neighbouring uninvoked block, which would silently turn a reference
    /// table's rows into study cards.
    #[test]
    fn a_trailing_invocation_maps_only_its_immediately_preceding_table() {
        let run = parse(
            "# S\n\n| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n\n| c | d |\n|---|---|\n| u | v |\n",
        );
        assert_eq!(
            1,
            run.cards.len(),
            "only the invoked table maps: {:?}",
            run.cards.iter().map(|card| &card.front).collect::<Vec<_>>()
        );
        assert_eq!(
            "x", run.cards[0].front,
            "the card comes from the invoked first table, not the uninvoked second"
        );
    }

    #[test]
    fn each_table_in_a_run_takes_its_own_trailing_invocation() {
        let run = parse(
            "# S\n\n| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n\n| c | d |\n|---|---|\n| u | v |\n<!-- cards -->\n",
        );
        assert_eq!(
            vec!["x", "u"],
            run.cards
                .iter()
                .map(|card| card.front.as_str())
                .collect::<Vec<_>>(),
            "each table in a run maps under its own invocation"
        );
    }

    #[test]
    fn a_leading_table_invocation_is_machinery_out_of_position() {
        let error = err("# S\n\n<!-- cards -->\n| a | b |\n|---|---|\n| x | y |\n");
        let ParseError::LeadingInvocation { line, .. } = error else {
            panic!("a leading invocation is recognized machinery out of position, got {error:?}");
        };
        assert_eq!(3, line, "the error names the invocation's line");
    }

    #[test]
    fn a_trailing_choices_invocation_maps_its_task_list() {
        let choices = parse("## Pick\n- [x] right\n- [ ] wrong\n<!-- choices-single -->\n");
        assert_eq!(1, choices.cards.len());
        assert_eq!(vec!["right"], choices.cards[0].back);
        assert_eq!(
            vec!["wrong".to_string()],
            choices.cards[0].authored_distractors,
            "a choices invocation below its task list maps the card"
        );
    }

    /// The block's locator belongs to every card the block produces. A
    /// blank block produces one card per span, and each is a review card an
    /// author flips to its source on reveal.
    #[test]
    fn every_blank_card_carries_the_blocks_source_citation() {
        let deck = parse(
            "## Complete the sentence\nThe owner is dropped at scope end.\n\
             <!-- blank: span hidden=\"dropped\" -->\n<!-- blank: span hidden=\"scope\" -->\n\
             <!-- at: src/lib.rs:3-4 -->\n> [!NOTE]\n> deterministic cleanup\n",
        );
        assert_eq!(2, deck.cards.len(), "one card per span: {deck:?}");
        for card in &deck.cards {
            assert_eq!(
                vec!["src/lib.rs:3-4"],
                card.citations
                    .iter()
                    .map(|citation| citation.locator.as_str())
                    .collect::<Vec<_>>(),
                "card {:?} keeps the block's locator: {card:?}",
                card.back
            );
        }
    }

    #[test]
    fn a_leading_choices_invocation_is_machinery_out_of_position() {
        let leading_choices = err("## Pick\n<!-- choices-single -->\n- [x] right\n- [ ] wrong\n");
        assert!(
            matches!(
                leading_choices,
                ParseError::LeadingInvocation { line: 2, .. }
            ),
            "a choices invocation above its list errors, got {leading_choices:?}"
        );
    }

    #[test]
    fn a_trailing_plain_keeps_its_divider_literal() {
        let plain = parse("## Q\nfirst\n\n---\n<!-- plain -->\n\nsecond\n");
        assert_eq!(
            1,
            plain.cards.len(),
            "a trailing plain keeps the divider content"
        );
        assert!(
            plain.cards[0].back.iter().any(|line| line == "---"),
            "the divider stays content under a trailing plain: {:?}",
            plain.cards[0].back
        );
    }

    #[test]
    fn a_card_directive_above_content_is_machinery_out_of_position() {
        let rows = [
            ("<!-- reveal: line -->", "reveal"),
            ("<!-- input: draw -->", "input"),
            ("<!-- direction: both -->", "direction"),
            ("<!-- at: src/x.rs:1-2 -->", "at"),
            ("<!-- given: a hint -->", "given"),
            ("<!-- sampling: off -->", "sampling"),
            ("<!-- id: card-3g12jfjv4pypppsrx5wvtx65y5 -->", "id"),
        ];
        for (comment, key) in rows {
            let error = err(&format!("## q\n{comment}\nanswer\n"));
            match &error {
                ParseError::LeadingDirective { line, key: found } => {
                    assert_eq!((2, key), (*line, found.as_str()), "{key} above content");
                }
                other => panic!("{key} above content: expected LeadingDirective, got {other:?}"),
            }
        }
        for (comment, key) in rows {
            let deck = parse(&format!("## q\nanswer\n{comment}\n"));
            assert_eq!(1, deck.cards.len(), "{key} in the trailing zone is legal");
        }

        let quote = err("## q\nanswer\n<!-- reveal: line -->\n> a quotation\n");
        assert!(
            matches!(&quote, ParseError::LeadingDirective { line: 3, .. }),
            "an unbadged blockquote is answer content, so machinery above it \
             errors, got {quote:?}"
        );

        let divider = err("## q\n<!-- reveal: line -->\n\n---\nback\n");
        assert!(
            matches!(&divider, ParseError::LeadingDirective { line: 2, .. }),
            "the divider is card structure, so machinery above it errors, got {divider:?}"
        );

        let stamped = parse(
            "## q\n```mermaid\ngraph TD\n```\n<!-- diagram: fingerprint: xxh64-0123456789abcdef asset: sha256-0000000000000000000000000000000000000000000000000000000000000000.png geometry: sha256-0000000000000000000000000000000000000000000000000000000000000000.json -->\nprose after\n",
        );
        assert_eq!(
            1,
            stamped.cards.len(),
            "a diagram stamp trails its fence, not the card, so content after it stays legal"
        );
    }

    #[test]
    fn a_titled_table_does_not_bypass_the_trailing_directive_rule() {
        let explicit = super::parse(
            "deck.md",
            "## Vocabulary\n<!-- direction: both -->\n| word | meaning |\n|---|---|\n| one | eins |\n<!-- cards -->\n",
        );
        let defaulted = super::parse(
            "deck.md",
            "---\ntable: cards\n---\n## Vocabulary\n<!-- direction: both -->\n| word | meaning |\n|---|---|\n| one | eins |\n",
        );
        assert!(
            matches!(
                &explicit,
                Err(ParseError::LeadingDirective { line: 2, key }) if key == "direction"
            ) && matches!(
                &defaulted,
                Err(ParseError::LeadingDirective { line: 5, key }) if key == "direction"
            ),
            "an empty heading absorbed as a table title must not let leading card machinery bypass the card-content guard: explicit={explicit:?}, defaulted={defaulted:?}"
        );
    }

    #[test]
    fn a_titled_tables_trailing_directive_remains_legal() {
        for text in [
            "## Vocabulary\n| word | meaning |\n|---|---|\n| one | eins |\n<!-- cards -->\n<!-- direction: both -->\n",
            "---\ntable: cards\n---\n## Vocabulary\n| word | meaning |\n|---|---|\n| one | eins |\n<!-- direction: both -->\n",
        ] {
            let deck = super::parse("deck.md", text).unwrap();
            assert_eq!(1, deck.cards.len());
            assert_eq!(Some(Direction::Both), deck.cards[0].direction);
        }
    }

    #[test]
    fn a_recognized_directive_on_the_heading_line_is_machinery_out_of_position() {
        let rows = [
            ("reveal: line", "reveal"),
            ("input: draw", "input"),
            ("direction: both", "direction"),
            ("at: src/x.rs:1-2", "at"),
            ("given: a hint", "given"),
            ("sampling: off", "sampling"),
            ("id: card-3g12jfjv4pypppsrx5wvtx65y5", "id"),
            (
                "diagram: fingerprint: xxh64-0123456789abcdef asset: sha256-0000000000000000000000000000000000000000000000000000000000000000.png geometry: sha256-0000000000000000000000000000000000000000000000000000000000000000.json",
                "diagram",
            ),
            ("blank: span hidden=\"a\"", "blank"),
            ("cover: rect x=1 y=2 width=3 height=4", "cover"),
            ("crop: rect x=1 y=2 width=3 height=4", "crop"),
        ];
        for (body, key) in rows {
            let error = err(&format!("## q <!-- {body} -->\nanswer\n"));
            match &error {
                ParseError::FrontDirective { line, key: found } => {
                    assert_eq!(
                        (1, key),
                        (*line, found.as_str()),
                        "{key} on the heading line"
                    );
                }
                other => {
                    panic!("{key} on the heading line: expected FrontDirective, got {other:?}")
                }
            }
        }

        let unknown = parse("## q <!-- zzz: 1 -->\nanswer\n");
        assert_eq!(
            "q", unknown.cards[0].front,
            "an unknown key on the heading line stays a stripped lint, not an error"
        );
        let editorial = parse("## q <!-- keep this short -->\nanswer\n");
        assert_eq!(
            "q", editorial.cards[0].front,
            "an editorial comment on the heading line strips silently"
        );
    }

    #[test]
    fn everything_trails_invocation_boundary_table_does_not_reach_back() {
        let deck = parse(
            "# S\n\n| a | b |\n|---|---|\n| x | y |\n| c | d |\n|---|---|\n| u | v |\n<!-- cards -->\n",
        );

        assert_eq!(
            1,
            deck.cards.len(),
            "only the immediately preceding second table is invoked"
        );
        assert_eq!("u", deck.cards[0].front);
    }

    #[test]
    fn everything_trails_invocation_boundary_table_is_loud_when_nonadjacent() {
        let error = err("# S\n\n| a | b |\n|---|---|\n| x | y |\n\n<!-- cards -->\n");

        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 7, ref word } if word == "cards"
            ),
            "a nonadjacent table invocation must identify its line, got {error:?}"
        );
    }

    #[test]
    fn everything_trails_invocation_boundary_choices_is_loud_when_nonadjacent() {
        let error = err("## Pick\n- [x] right\n- [ ] wrong\n\n<!-- choices-single -->\n");

        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 5, ref word }
                    if word == "choices-single"
            ),
            "a nonadjacent choices invocation must identify its line, got {error:?}"
        );
    }

    #[test]
    fn everything_trails_invocation_does_not_reach_through_a_note_block() {
        let error =
            err("## Pick\n- [x] right\n- [ ] wrong\n> Remember why.\n<!-- choices-single -->\n");

        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 5, ref word }
                    if word == "choices-single"
            ),
            "the note is the directly preceding block, so the older task list cannot bind: {error:?}"
        );
    }

    #[test]
    fn everything_trails_invocation_does_not_reach_through_a_fence_block() {
        let error = err(
            "## Pick\n- [x] right\n- [ ] wrong\n```text\nnot an option\n```\n<!-- choices-single -->\n",
        );

        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 7, ref word }
                    if word == "choices-single"
            ),
            "the fence is the directly preceding block, so the older task list cannot bind: {error:?}"
        );
    }

    #[test]
    fn each_badged_blockquote_run_is_its_own_note() {
        let deck =
            parse("## Q\nanswer\n> [!NOTE]\n> first\n> still first\n\n> [!WARNING]\n> second\n");

        assert_eq!(
            vec![
                Note {
                    badge: Some(Badge::Note),
                    body: "first\nstill first".to_string(),
                },
                Note {
                    badge: Some(Badge::Warning),
                    body: "second".to_string(),
                },
            ],
            deck.cards[0].notes,
            "a blank line ends a note run, and the next badge opens another"
        );
    }

    #[test]
    fn a_badge_line_inside_an_open_note_run_is_body_text() {
        let deck = parse("## Q\nanswer\n> [!NOTE]\n> first\n> [!WARNING]\n> still first\n");

        assert_eq!(
            vec![Note {
                badge: Some(Badge::Note),
                body: "first\n[!WARNING]\nstill first".to_string(),
            }],
            deck.cards[0].notes,
            "a badge opens a note only at the start of a run, as GitHub reads it"
        );
    }

    /// The machinery run between a block and its invocation is a closed,
    /// recognized set; anything else standing there is a block of its own.
    /// A badge opens a note; every other blockquote is a quote, which is
    /// answer content and reveals with it.
    #[test]
    fn a_badge_opens_a_note_and_every_other_blockquote_is_a_quote() {
        for (spelling, badge, why) in [
            ("[!NOTE]", Badge::Note, "the neutral badge"),
            (
                "[!TIP]",
                Badge::Tip,
                "a tip is a plain styled note, not the retired hint",
            ),
            ("[!IMPORTANT]", Badge::Important, "the third badge"),
            ("[!WARNING]", Badge::Warning, "the fourth badge"),
            ("[!CAUTION]", Badge::Caution, "the fifth badge"),
        ] {
            let deck = parse(&format!("## Q\nanswer\n> {spelling}\n> because\n"));
            assert_eq!(
                vec![Note {
                    badge: Some(badge),
                    body: "because".to_string()
                }],
                deck.cards[0].notes,
                "{why}: the badge line opens the note, never joins its body, and is what the note carries"
            );
            assert_eq!(
                vec!["answer"],
                deck.cards[0].back,
                "{why}: a note is not answer content"
            );
            assert!(deck.lints.is_empty(), "{why}: a valid badge draws nothing");
        }

        let deck = parse("## Q\nanswer\n> a quoted line\n> and its second\n");
        assert_eq!(
            vec!["answer", "> a quoted line", "> and its second"],
            deck.cards[0].back,
            "a bare blockquote is a quote, so it is answer content"
        );
        assert_eq!(None, deck.cards[0].only_note(), "and it is not a note");
        assert!(
            deck.lints.is_empty(),
            "an ordinary quote is legitimate content and draws nothing: {:?}",
            deck.lints
        );
    }

    /// Badge misuse degrades to a quote with a doctor warning; hard errors
    /// stay reserved for shapes that silently corrupt meaning.
    #[test]
    fn a_badge_alix_does_not_know_stays_a_quote_and_is_named() {
        for (first, why) in [
            ("[!NOTES]", "a typo'd badge name"),
            (
                "[!note]",
                "GitHub casing is exact, so lowercase is a quote there too",
            ),
            (
                "[!NOTE] text on the same line",
                "GitHub wants the badge alone",
            ),
        ] {
            let deck = parse(&format!("## Q\nanswer\n> {first}\n> body\n"));
            assert_eq!(
                None,
                deck.cards[0].only_note(),
                "{why}: it never opens a note"
            );
            assert_eq!(
                3,
                deck.cards[0].back.len(),
                "{why}: the whole run is answer content"
            );
            assert!(
                matches!(deck.lints[0].kind, LintKind::BadgeShape { .. }),
                "{why}: the meaning shift is named, got {:?}",
                deck.lints
            );
        }

        let deck = parse("## Q\nanswer\n> [!NOTE]\n");
        assert_eq!(
            None,
            deck.cards[0].only_note(),
            "an empty note carries nothing"
        );
        assert!(
            matches!(deck.lints[0].kind, LintKind::EmptyNote),
            "a badge with no body is named rather than failing the deck: {:?}",
            deck.lints
        );
    }

    /// A badged note trails its card like every other recognized machinery:
    /// a directive may stand above it, and answer content may not follow it.
    #[test]
    fn a_note_is_trailing_machinery_that_content_may_not_follow() {
        let deck = parse("## q\nanswer\n<!-- reveal: line -->\n> [!WARNING]\n> body\n");
        assert_eq!(
            Some("body"),
            deck.cards[0].only_note(),
            "a directive above a note is two machinery items, not one above content"
        );
        assert_eq!(
            Some(crate::depth::Reveal::Line),
            deck.cards[0].reveal,
            "and the directive still applies"
        );

        let rows = [
            (
                "## q\nanswer\n> [!NOTE]\n> body\nmore answer\n",
                "prose below a note",
            ),
            (
                "## q\nanswer\n> [!NOTE]\n> body\n\n> a quotation\n",
                "a quote below a note, since a quote is answer content",
            ),
        ];
        for (src, why) in rows {
            let error = err(src);
            assert!(
                matches!(error, ParseError::ContentAfterNote { line: 3 }),
                "{why} must name the badge line, got {error:?}"
            );
        }

        let error = err("## q\nanswer\n<!-- reveal: line -->\n> [!NOTE]\n> body\nmore\n");
        assert!(
            matches!(error, ParseError::LeadingDirective { line: 3, .. }),
            "whatever opened the run is what the error names, got {error:?}"
        );
    }

    /// Emptiness is a property of the finished note body, not of the one
    /// line under the badge.
    #[test]
    fn an_empty_note_is_judged_on_its_whole_body() {
        for (src, why) in [
            ("## q\nanswer\n> [!NOTE]\n", "a badge with nothing under it"),
            (
                "## q\nanswer\n> [!NOTE]\n>\n>   \n",
                "a body of blank quote lines, as a draft or a pasted callout leaves",
            ),
        ] {
            let deck = parse(src);
            assert!(
                deck.lints
                    .iter()
                    .any(|lint| matches!(lint.kind, LintKind::EmptyNote)),
                "{why} renders nothing, so it is named: {:?}",
                deck.lints
            );
        }

        let deck = parse("## q\nanswer\n> [!NOTE]\n>\n> real text\n");
        assert!(
            deck.lints.is_empty(),
            "a blank spacer above real text is not an empty note: {:?}",
            deck.lints
        );
    }

    #[test]
    fn everything_trails_invocation_sees_through_machinery_and_nothing_else() {
        let deck = parse(
            "## Pick\n- [x] right\n- [ ] wrong\n<!-- reveal: line -->\n<!-- choices-single -->\n",
        );
        assert_eq!(
            vec!["wrong"],
            deck.cards[0].authored_distractors,
            "a recognized directive is machinery, so the list still binds"
        );

        let error = err(
            "## Pick\n- [x] right\n- [ ] wrong\n<!-- a reminder to myself -->\n<!-- choices-single -->\n",
        );
        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 5, ref word }
                    if word == "choices-single"
            ),
            "an editorial comment is not machinery, so it ends the block: {error:?}"
        );

        let error = err("## Q\nanswer\n<!-- choices-single -->\n");
        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 3, ref word }
                    if word == "choices-single"
            ),
            "ordinary answer prose is no task list, so nothing binds: {error:?}"
        );

        let error = err("## Q\nanswer\n---\n<!-- plain -->\n<!-- choices-single -->\n");
        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 5, ref word }
                    if word == "choices-single"
            ),
            "a divider is a block choices cannot map, even reached through \
             machinery: {error:?}"
        );

        let deck = parse("## Q\nfirst\n\n---\n<!-- reveal: line -->\n<!-- plain -->\n");
        assert!(
            deck.cards[0].back.contains(&"---".to_string()),
            "a recognized directive is transparent for a divider too, so plain \
             still keeps it literal: {:?}",
            deck.cards[0].back
        );
        assert_eq!(
            Some(crate::depth::Reveal::Line),
            deck.cards[0].reveal,
            "and the directive it reached across still applies"
        );

        let error = err(
            "## Pick\n- [x] right\n- [ ] wrong\n<!-- reveel: line -->\n<!-- choices-single -->\n",
        );
        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 5, ref word }
                    if word == "choices-single"
            ),
            "an unknown key is not recognized machinery, so it ends the run \
             instead of silently reclassifying the list: {error:?}"
        );

        let error = err(
            "## Pick\n- [x] right\n- [ ] wrong\n<!-- choices-single -->\n<!-- choices-multiple -->\n",
        );
        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 5, ref word }
                    if word == "choices-multiple"
            ),
            "one invocation consumes the block it binds, so a second cannot \
             silently win: {error:?}"
        );

        let error = err(
            "# Reference\n| term | meaning |\n|---|---|\n| one | eins |\n<!-- editorial note -->\n<!-- cards -->\n",
        );
        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 6, ref word } if word == "cards"
            ),
            "an editorial comment ends a table's trailing zone exactly as it \
             ends a checklist's: {error:?}"
        );

        let error = err(
            "# Reference\n| term | meaning |\n|---|---|\n| one | eins |\n<!-- cards -->\n<!-- plain -->\n",
        );
        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 6, ref word } if word == "plain"
            ),
            "a table is consumed by the one invocation that selected it, like \
             any other block: {error:?}"
        );

        let deck = parse("# S\n\nprose\n\n---\n<!-- plain -->\n");
        assert!(
            deck.cards.is_empty() && deck.lints.is_empty(),
            "a section divider does supply a block, so plain below one stays \
             legal: {:?}",
            deck.lints
        );

        let deck = parse(
            "---\ntable: cards\n---\n# Reference\n| term | meaning |\n|---|---|\n| one | eins |\n<!-- plain -->\n\n## q\nanswer\n",
        );
        assert_eq!(
            1,
            deck.cards.len(),
            "`plain` below one table is the documented escape from a `table: \
             cards` default, so the deck loads: {:?}",
            deck.cards
        );
        assert!(
            deck.cards[0]
                .section_context
                .contains(&"| one | eins |".to_string()),
            "and the escaped table stays literal in the section it sits in, \
             never silently dropped: {:?}",
            deck.cards[0].section_context
        );

        let deck = parse(
            "---\ntable: cards\n---\n## Compare the terms\n| term | meaning |\n|---|---|\n| one | eins |\n<!-- plain -->\n",
        );
        assert_eq!(
            vec!["| term | meaning |", "|---|---|", "| one | eins |"],
            deck.cards[0].back,
            "the same escape holds when the literal table IS a card's answer, \
             the shape both book chapters show: {:?}",
            deck.cards
        );

        let error = err("---\ntable: cards\n---\n## Q\nanswer\n<!-- plain -->\n");
        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 6, ref word } if word == "plain"
            ),
            "a table owns the escape, so `plain` under ordinary answer prose \
             is still loud: {error:?}"
        );

        let deck = parse(
            "---\ntable: cards\n---\n## Two\n| a | b |\n|---|---|\n| 1 | 2 |\n<!-- plain -->\n| c | d |\n|---|---|\n| 3 | 4 |\n<!-- plain -->\n",
        );
        assert_eq!(
            vec![
                "| a | b |",
                "|---|---|",
                "| 1 | 2 |",
                "| c | d |",
                "|---|---|",
                "| 3 | 4 |"
            ],
            deck.cards[0].back,
            "each literal table owns its own trailing `plain`, so one escape \
             never stands in for the next: {:?}",
            deck.cards
        );

        let error = err(
            "---\ntable: cards\n---\n## Q\n| a | b |\n|---|---|\n| 1 | 2 |\n<!-- plain -->\ntail\n<!-- plain -->\n",
        );
        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 10, ref word } if word == "plain"
            ),
            "ownership is one exact line, not a standing licence for the rest \
             of the card: {error:?}"
        );
    }

    #[test]
    fn everything_trails_invocation_boundary_plain_is_loud_when_leading() {
        let error = err("## Q\nanswer\n\n<!-- plain -->\n---\n");

        assert!(
            matches!(
                error,
                ParseError::LeadingInvocation { line: 4, ref word } if word == "plain"
            ),
            "the error must identify the misplaced invocation and its line, got {error:?}"
        );
    }

    #[test]
    fn tag_shapes_error_on_every_deck_surface_and_the_outs_stay_legal() {
        let error_rows = [
            ("## Q\na <div> b\n", 2, "card content"),
            ("# S\n\nprose with <span>\n", 3, "section content"),
            ("## What does <div> do?\nanswer\n", 1, "a heading front"),
            ("## Q\nanswer\n> note with <b>bold</b>\n", 3, "a note line"),
            (
                "| a | <div> |\n|---|---|\n| x | y |\n<!-- cards -->\n",
                1,
                "a table header cell",
            ),
            (
                "| a | b |\n|---|---|\n| x | <div> |\n<!-- cards -->\n",
                3,
                "a table body cell",
            ),
        ];
        for (deck, line, why) in error_rows {
            match err(deck) {
                ParseError::TagShape { line: at, .. } => {
                    assert_eq!(at, line, "{why}: wrong line");
                }
                other => panic!("{why}: expected TagShape, got {other:?}"),
            }
        }
        let legal_rows = [
            ("## Q\nsee <https://alix.study> now\n", "a uri autolink"),
            ("## Q\nH<sub>2</sub>O\n", "a subset pair"),
            ("## Q\n`<div>` in code\n", "a code span"),
            ("## Q\n![d](<old image.png>)\na\n", "an image destination"),
            (
                "| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n",
                "the invocation comment",
            ),
            (
                "## Q\n<!-- <div> disabled -->\na\n",
                "a channel comment line with a tag inside",
            ),
            ("## Q\n```\n<div>\n```\n", "a fence interior"),
            ("## Q\na < b and x<3\n", "non-tag brackets"),
            ("## Q\n$$\na<b\n$$\n", "verbatim math source"),
        ];
        for (deck, why) in legal_rows {
            parse(deck);
            let _ = why;
        }
    }

    #[test]
    fn cross_nested_subset_tags_fail_at_the_inner_opener() {
        assert_eq!(
            err("## Q\n<sub><sup>x</sub></sup>\n"),
            ParseError::TagShape { line: 2, column: 6 }
        );
    }

    #[test]
    fn an_incomplete_image_destination_does_not_exempt_a_tag_shape() {
        assert_eq!(
            err("## Q\n![diagram](<diagram.png>\n"),
            ParseError::TagShape {
                line: 2,
                column: 12
            }
        );
    }

    #[test]
    fn the_three_image_marker_consumers_agree_on_escape_parity() {
        for run in 0..=3usize {
            let deck = format!("## Q\n{}![d](<x.png>)\n", "\\".repeat(run));
            let result = super::parse("deck.md", &deck);
            let escaped = run % 2 == 1;
            match (run, result) {
                (0, Ok(deck)) => assert_eq!(
                    vec![PathBuf::from("x.png")],
                    img_srcs(&deck.cards[0].images_back),
                    "a bare marker consumes the image"
                ),
                (2, Err(ParseError::MixedImageLine(2))) => {}
                (_, Err(ParseError::TagShape { line: 2, column })) if escaped => assert_eq!(
                    run + 6,
                    column,
                    "an escaped marker denies the carve-out at the angle bracket, run {run}"
                ),
                (_, other) => panic!("run {run}: unexpected outcome {other:?}"),
            }
            let references: Vec<_> = super::image_references(&deck)
                .into_iter()
                .map(|reference| reference.source)
                .collect();
            let expected: Vec<String> = if escaped {
                Vec::new()
            } else {
                vec!["x.png".into()]
            };
            assert_eq!(
                expected, references,
                "image_references must share the marker's parity, run {run}"
            );
        }
    }

    #[test]
    fn an_image_destination_exemption_requires_a_consumed_image() {
        let result = super::parse("deck.md", "## Q\n\\\\![d](<x.png>)\n");
        match result {
            Err(ParseError::TagShape { line: 2, column: 8 } | ParseError::MixedImageLine(2)) => {}
            Ok(deck) => assert_eq!(
                vec![PathBuf::from("x.png")],
                img_srcs(&deck.cards[0].images_back),
                "the tag-shape classifier exempted the angle destination, but the card parser left the same marker as literal prose"
            ),
            Err(other) => panic!("unexpected diagnosis: {other:?}"),
        }
    }

    #[test]
    fn an_inline_comment_does_not_create_a_second_comment_channel() {
        assert_eq!(
            err("## Q\ntext <!-- <div> inside -->\n"),
            ParseError::TagShape {
                line: 2,
                column: 11
            }
        );
    }

    #[test]
    fn an_unclosed_display_math_opener_fails_at_its_line_and_neighbors_stay_legal() {
        let error_rows = [
            ("## Q\nanswer\n$$\nx^2\n", 3, "unclosed at end of file"),
            (
                "## Q\n$$\nx^2\n## R\nb\n",
                2,
                "unclosed at the next heading",
            ),
            (
                "## Q\n$$\nx\n$$\ntext\n$$\ntail\n",
                6,
                "a second opener reopens",
            ),
        ];
        for (deck, line, why) in error_rows {
            assert_eq!(
                err(deck),
                ParseError::UnclosedDisplayMath(line),
                "{why}: {deck:?}"
            );
        }
        let legal_rows = [
            ("## Q\n$$\nx^2\n$$\n", "a closed pair is legal"),
            ("## Q\n```\n$$\n```\n", "a fence interior never toggles"),
            (
                "# S\n\n$$\nx\n\n## Q\na\n",
                "section content keeps literal dollars",
            ),
            ("## Q\n$$x^2$$\n", "the single-line spelling is untouched"),
            (
                "## Q\n\\$$\nx\n",
                "an escaped marker is content, not an opener",
            ),
        ];
        for (text, why) in legal_rows {
            let deck = parse(text);
            let kept: String = deck
                .cards
                .iter()
                .flat_map(|card| card.back.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                text.starts_with("# S") || kept.contains('$'),
                "{why}: the dollars stay content, {kept:?}"
            );
        }
    }

    #[test]
    fn an_unclosed_display_math_note_fails_at_its_opener_line() {
        assert_eq!(
            err("## Q\nanswer\n> [!NOTE]\n> $$\n> x^2\n"),
            ParseError::UnclosedDisplayMath(4),
            "a note uses the same display-math spelling and hard-error contract"
        );
    }

    #[test]
    fn a_display_math_note_keeps_a_greater_than_source_line() {
        let deck = parse("## Q\nanswer\n> [!NOTE]\n> $$\n> x^2\n> > 0\n> $$\n");
        assert_eq!(deck.cards[0].only_note(), Some("$$\nx^2\n> 0\n$$"));
    }

    #[test]
    fn a_short_delimiter_row_recommends_a_valid_delimiter_cell() {
        let error = err("| front | back |\n|---|\n| one | two |\n<!-- cards -->\n");
        assert!(
            matches!(
                error,
                ParseError::TableDelimiterWidth {
                    line: 2,
                    found: 1,
                    expected: 2,
                }
            ),
            "the malformed delimiter row must be diagnosed at its own line: {error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains("`---` cell"),
            "an empty cell is invalid in a delimiter row, so the rewrite must name a valid dash cell: {message}"
        );
    }

    #[test]
    fn a_tasklist_deck_default_invokes_without_per_card_markers() {
        let deck = parse("---\ntasklist: choices-single\n---\n## q\n- [x] a\n- [ ] b\n");
        assert_eq!(vec!["a"], deck.cards[0].back);
        assert_eq!(vec!["b"], deck.cards[0].authored_distractors);
        assert_eq!(Vec::<Lint>::new(), deck.lints, "a known key never lints");
    }

    #[test]
    fn a_per_block_invocation_overrides_the_deck_default() {
        let deck = parse(
            "---\ntasklist: choices-multiple\n---\n## q\n- [x] a\n- [ ] b\n<!-- choices-single -->\n",
        );
        assert_eq!(vec!["a"], deck.cards[0].back);
        assert_eq!(
            Vec::<Lint>::new(),
            deck.lints,
            "single overrides, so no degenerate-multiple finding"
        );
    }

    #[test]
    fn a_plain_marker_escapes_the_deck_default() {
        let deck =
            parse("---\ntasklist: choices-single\n---\n## q\n- [x] a\n- [ ] b\n<!-- plain -->\n");
        assert_eq!(vec!["- [x] a", "- [ ] b"], deck.cards[0].back);
    }

    #[test]
    fn a_table_deck_default_invokes_cards() {
        let deck = parse("---\ntable: cards\n---\n| a | b |\n|---|---|\n| x | y |\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!("x", deck.cards[0].front);
    }

    #[test]
    fn a_typoed_invocation_shaped_comment_draws_a_lint() {
        let deck = parse("## q\n<!-- choices-singel -->\n- [x] a\n- [ ] b\n");
        assert_eq!(
            vec!["- [x] a", "- [ ] b"],
            deck.cards[0].back,
            "the block stays literal"
        );
        assert_eq!(1, deck.lints.len());
        assert!(matches!(deck.lints[0].kind, LintKind::UnrecognizedComment));
        assert_eq!(2, deck.lints[0].line);
    }

    #[test]
    fn blank_and_image_only_lines_do_not_turn_authored_choices_into_prose() {
        let deck = parse(
            "## Which is prime?\n\n- [ ] 4\n![](number-line.png)\n- [x] 5\n<!-- choices-single -->\n",
        );
        assert_eq!(vec!["5"], deck.cards[0].back);
        assert_eq!(vec!["4".to_string()], deck.cards[0].authored_distractors);
        assert_eq!(
            vec![PathBuf::from("number-line.png")],
            img_srcs(&deck.cards[0].images_back)
        );
        assert!(deck.lints.is_empty(), "{:?}", deck.lints);
    }

    #[test]
    fn a_divided_checkbox_card_takes_options_from_the_answer_region() {
        let deck = parse(
            "## Pick one\nsome stimulus\n\n---\n- [x] yes\n- [ ] no\n<!-- choices-single -->\n",
        );
        let card = &deck.cards[0];
        assert_eq!("Pick one\nsome stimulus", card.front);
        assert_eq!(vec!["yes"], card.back);
        assert_eq!(vec!["no".to_string()], card.authored_distractors);
    }

    #[test]
    fn a_bare_mix_of_task_list_and_prose_is_silent_and_an_invoked_one_errors() {
        let deck = parse("## q\n- [x] a\nnot an option\n");
        assert!(deck.cards[0].authored_distractors.is_empty());
        assert_eq!(vec!["- [x] a", "not an option"], deck.cards[0].back);
        assert_eq!(
            Vec::<Lint>::new(),
            deck.lints,
            "bare is literal, no finding"
        );

        for (name, text) in [
            (
                "prose",
                "## q\n- [x] a\nnot an option\n<!-- choices-single -->\n",
            ),
            (
                "quotation",
                "## q\n- [x] a\n> not an option\n- [ ] b\n<!-- choices-single -->\n",
            ),
            (
                "quotation under select-all",
                "## q\n- [x] a\n> not an option\n- [ ] b\n<!-- choices-multiple -->\n",
            ),
        ] {
            let error = err(text);
            assert!(
                matches!(error, ParseError::ChoiceShape { line: 2, .. }),
                "{name}: loudness follows invocation, got {error:?}"
            );
        }
    }

    #[test]
    fn single_rejects_extra_missing_or_undistracted_checks_loudly() {
        for (name, text) in [
            (
                "two checks",
                "## q\n- [x] a\n- [x] b\n<!-- choices-single -->\n",
            ),
            (
                "no check",
                "## q\n- [ ] a\n- [ ] b\n<!-- choices-single -->\n",
            ),
            ("no distractor", "## q\n- [x] a\n<!-- choices-single -->\n"),
        ] {
            let error = err(text);
            assert!(
                matches!(error, ParseError::ChoiceShape { line: 2, .. }),
                "{name}: expected ChoiceShape, got {error:?}"
            );
        }
    }

    #[test]
    fn a_duplicate_option_lints_and_keeps_first() {
        let deck = parse("## q\n- [x] a\n- [ ] b\n- [ ] b\n<!-- choices-single -->\n");
        assert_eq!(vec!["b".to_string()], deck.cards[0].authored_distractors);
        assert!(
            deck.lints
                .iter()
                .any(|lint| lint.kind == LintKind::DuplicateChoiceOption)
        );
    }

    #[test]
    fn a_choice_note_cannot_name_an_option_position_that_shuffle_changes() {
        let deck = parse(
            "## Pick one\n- [x] Correct claim\n- [ ] First misconception\n- [ ] Second misconception\n<!-- choices-single -->\n> [!NOTE]\n> Option 2 confuses identity with sampling.\n",
        );

        assert_eq!(
            1,
            deck.lints.len(),
            "a position-dependent note must be diagnosed before a shuffle makes it lie"
        );
        assert_eq!(LintKind::ChoiceNoteNamesPosition, deck.lints[0].kind);
    }

    #[test]
    fn a_fenced_task_list_answer_stays_a_plain_card() {
        let deck = parse("## q\n---\n```\n- [x] a\n- [ ] b\n```\n");
        assert!(deck.cards[0].authored_distractors.is_empty());
        assert_eq!(vec!["```", "- [x] a", "- [ ] b", "```"], deck.cards[0].back);
        assert!(deck.lints.is_empty(), "{:?}", deck.lints);
    }

    #[test]
    fn option_text_preserves_source_while_grading_uses_content() {
        let deck = parse("## q\n- [x] **Paris**\n- [ ] London\n<!-- choices-single -->\n");
        assert_eq!(vec!["**Paris**"], deck.cards[0].back);
        assert_eq!("Paris", crate::inline::strip_inline(&deck.cards[0].back[0]));
        assert_eq!(
            vec!["London".to_string()],
            deck.cards[0].authored_distractors
        );
    }

    #[test]
    fn math_checkbox_options_preserve_authored_source() {
        let deck = parse("## q\n- [x] $x^2$\n- [ ] $x^3$\n<!-- choices-single -->\n");
        assert_eq!(vec!["$x^2$"], deck.cards[0].back);
        assert_eq!(
            vec!["$x^3$".to_string()],
            deck.cards[0].authored_distractors
        );
        assert_eq!("x^2", crate::inline::strip_inline(&deck.cards[0].back[0]));
    }

    #[test]
    fn formatted_and_plain_checkbox_options_are_content_duplicates() {
        let deck = parse("## q\n- [x] $x$\n- [ ] x\n- [ ] y\n<!-- choices-single -->\n");
        assert_eq!(vec!["$x$"], deck.cards[0].back);
        assert_eq!(vec!["y".to_string()], deck.cards[0].authored_distractors);
        assert!(
            deck.lints
                .iter()
                .any(|lint| lint.kind == LintKind::DuplicateChoiceOption)
        );
    }

    #[test]
    fn editing_only_a_distractor_preserves_identity_and_fingerprint() {
        let before = parse(
            "## q\n- [x] right\n- [ ] wrong\n<!-- choices-single -->\n<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n",
        );
        let after = parse(
            "## q\n- [x] right\n- [ ] different\n<!-- choices-single -->\n<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n",
        );
        assert_eq!(before.cards[0].id(), after.cards[0].id());
        assert_eq!(
            before.cards[0].content_fingerprint,
            after.cards[0].content_fingerprint
        );
    }

    // ── Directives ──

    #[test]
    fn an_id_directive_yields_the_card_token() {
        let deck = parse(
            "## q\n---\na\n<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n## r\n---\nb\n\
             <!-- id: card-0m5v2 -->\n",
        );
        assert_eq!(
            Some("card-4jkya9q3m8z0tw5v9y2b4n6d8f"),
            deck.cards[0].token.as_deref()
        );
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(Some("card-0m5v2"), deck.cards[1].token.as_deref());
    }

    #[test]
    fn every_card_shape_is_stamped_with_the_decks_id() {
        let id = "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f";
        let text = format!(
            "---\nformat-version: 1\nid: \"{id}\"\n---\n\
             ## plain\na\n\
             ## choice\n- [x] a\n- [ ] b\n\
             ## cloze\nthe \\blank{{cat}} sat\n"
        );
        let deck = parse(&text);
        assert!(deck.cards.len() >= 3, "{:?}", deck.cards);
        for card in &deck.cards {
            assert_eq!(id, card.deck_id.as_ref(), "{card:?}");
        }
    }

    #[test]
    fn a_deck_without_an_id_stamps_cards_with_an_empty_deck_id() {
        let deck = parse("## q\na\n");
        assert_eq!("", deck.cards[0].deck_id.as_ref());
    }

    #[test]
    fn a_marker_failing_the_charset_is_a_line_numbered_error() {
        assert_eq!(
            ParseError::InvalidCardId {
                line: 4,
                value: "XYZ".into()
            },
            err("## q\n---\na\n<!-- id: XYZ -->\n")
        );
    }

    #[test]
    fn a_bare_token_marker_is_a_hard_error_not_an_unstamped_card() {
        let e = err("## q\n---\na\n<!-- id: 4jkya9q3m8z0tw5v9y2b4n6d8f -->\n");
        assert_eq!(
            ParseError::InvalidCardId {
                line: 4,
                value: "4jkya9q3m8z0tw5v9y2b4n6d8f".into()
            },
            e
        );
        assert!(e.to_string().contains("must hold a base"), "{e}");
    }

    #[test]
    fn a_deck_prefixed_marker_is_a_hard_error() {
        assert_eq!(
            ParseError::InvalidCardId {
                line: 4,
                value: "deck-4jkya9q3m8z0tw5v9y2b4n6d8f".into()
            },
            err("## q\n---\na\n<!-- id: deck-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n")
        );
    }

    #[test]
    fn a_sub_id_suffix_in_a_marker_is_a_hard_error() {
        assert_eq!(
            ParseError::InvalidCardId {
                line: 4,
                value: "card-t0-2".into()
            },
            err("## q\n---\na\n<!-- id: card-t0-2 -->\n")
        );
        assert_eq!(
            ParseError::InvalidCardId {
                line: 4,
                value: "card-t0-r".into()
            },
            err("## q\n---\na\n<!-- id: card-t0-r -->\n")
        );
    }

    #[test]
    fn directives_parse_the_closed_set_and_lint_unknown_keys() {
        let deck = parse(
            "## q\n---\na\n<!-- reveal: line -->\n<!-- input: draw -->\n\
             <!-- direction: both -->\n<!-- flavor: cherry -->\n",
        );
        assert_eq!(Some(Reveal::Line), deck.cards[0].reveal);
        assert_eq!(Some(Input::Draw), deck.cards[0].input);
        assert_eq!(Some(Direction::Both), deck.cards[0].direction);
        assert_eq!(vec![unknown(7, "flavor")], deck.lints);
    }

    /// A deck's `<!-- -->` comments are alix vocabulary, not editorial prose,
    /// so recognition is what decides the lint. Word count decided it before,
    /// which split the unrecognized set arbitrarily.
    #[test]
    fn every_comment_is_recognized_machinery_or_lints_whatever_its_word_count() {
        for (comment, recognized, why) in [
            ("<!-- reveal: line -->", true, "a known directive key"),
            ("<!-- direction: both -->", true, "another known key"),
            ("<!-- at: src/x.rs:1-2 -->", true, "a locator"),
            ("<!-- Transfer -->", false, "a one-word editorial label"),
            (
                "<!-- ## About Interfaces -->",
                false,
                "a commented-out heading, unrecognized and multi-word",
            ),
            (
                "<!-- Generated 2026-06 from a working session -->",
                false,
                "a provenance note, which belongs in `source:` or `description:`",
            ),
            (
                "<!-- % origin: /home/me/dev -->",
                false,
                "a key with whitespace is not a directive, so this is not machinery",
            ),
            (
                "<!-- flavor: cherry -->",
                false,
                "a directive-shaped unknown key",
            ),
        ] {
            let deck = parse(&format!("## q\n---\na\n{comment}\n"));
            assert_eq!(
                recognized,
                deck.lints.is_empty(),
                "{why}: {comment} should {} draw a lint, got {:?}",
                if recognized { "not" } else { "" },
                deck.lints
            );
        }
    }

    #[test]
    fn an_empty_value_lints_as_empty_only_for_a_known_key() {
        // A known key with no value is an authoring slip worth naming as such;
        // an unknown key is unknown whether or not it carries a value.
        let deck = parse("## q\n---\na\n<!-- reveal: -->\n");
        assert_eq!(
            vec![Lint {
                line: 4,
                kind: LintKind::EmptyValue {
                    key: "reveal".into()
                }
            }],
            deck.lints
        );

        let deck = parse("## q\n---\na\n<!-- flavor: -->\n");
        assert_eq!(vec![unknown(4, "flavor")], deck.lints);
    }

    #[test]
    fn unknown_content_directive_keys_lint_and_yield_no_image() {
        let deck = parse(
            "## q\n---\na\n<!-- img: moon.png -->\n<!-- img-back: phase.png -->\n\
             <!-- math: latex -->\n",
        );
        assert_eq!(
            vec![
                unknown(4, "img"),
                unknown(5, "img-back"),
                unknown(6, "math"),
            ],
            deck.lints
        );
        assert!(deck.cards[0].images.is_empty());
        assert!(deck.cards[0].images_back.is_empty());
    }

    #[test]
    fn unrecognized_directive_keys_lint_as_unknown() {
        let deck = parse(
            "## q\n---\na\n<!-- occlude: soon -->\n<!-- audio: a.mp3 -->\n\
             <!-- audio-back: b.mp3 -->\n<!-- img-alt: a moon -->\n",
        );
        assert_eq!(
            vec![
                unknown(4, "occlude"),
                unknown(5, "audio"),
                unknown(6, "audio-back"),
                unknown(7, "img-alt"),
            ],
            deck.lints
        );
    }

    #[test]
    fn at_is_repeatable_and_keeps_each_fingerprint_asset_pair() {
        let deck = parse(
            "## q\n---\na\n\
             <!-- at: src/caching.rs:46-66 fingerprint: xxh64-0123456789abcdef \
             asset: sha256-abc123.rs -->\n\
             <!-- at: src/store.rs:10-14 -->\n",
        );
        assert_eq!(
            vec![
                crate::card::SourceCitation {
                    locator: "src/caching.rs:46-66".to_string(),
                    fingerprint: Some(0x0123456789abcdef),
                    asset: Some("sha256-abc123.rs".to_string()),
                    line: 4,
                },
                crate::card::SourceCitation {
                    locator: "src/store.rs:10-14".to_string(),
                    fingerprint: None,
                    asset: None,
                    line: 5,
                },
            ],
            deck.cards[0].citations
        );

        let deck = parse("## q\n---\na\n<!-- at: src/from_x.rs:1-3 -->\n");
        assert_eq!("src/from_x.rs:1-3", deck.cards[0].citations[0].locator);
        assert_eq!(None, deck.cards[0].citations[0].fingerprint);
        assert_eq!(None, deck.cards[0].citations[0].asset);
    }

    #[test]
    fn an_at_locator_with_unexpected_content_is_a_hard_error() {
        let e = err("## q\n---\na\n\
             <!-- at: 29.rs @ xxh64:0123456789abcdef from src/caching.rs:46-66 -->\n");
        assert!(
            matches!(e, ParseError::InvalidLocator { line: 4, .. }),
            "{e:?}"
        );
        assert!(e.to_string().contains("in that order"), "{e}");

        let e = err("## q\n---\na\n<!-- at: 29.rs:1 from src/caching.rs:46-66 -->\n");
        assert!(
            matches!(e, ParseError::InvalidLocator { line: 4, .. }),
            "{e:?}"
        );
    }

    #[test]
    fn a_reordered_locator_error_carries_the_canonical_order_hint() {
        let e = err("## q\n---\na\n\
             <!-- at: notes.md asset: sha256-abc123.rs fingerprint: xxh64-0123456789abcdef -->\n");
        assert!(
            matches!(e, ParseError::InvalidLocator { line: 4, .. }),
            "{e:?}"
        );
        let message = e.to_string();
        assert!(
            message.contains("fields are `at:`, `fingerprint:`, `asset:`, in that order"),
            "{message}"
        );
    }

    #[test]
    fn a_fingerprint_without_the_xxh64_dash_prefix_is_a_hard_error() {
        let e =
            err("## q\n---\na\n<!-- at: src/lib.rs:1-3 fingerprint: xxh64:0123456789abcdef -->\n");
        assert!(
            matches!(e, ParseError::InvalidLocator { line: 4, .. }),
            "{e:?}"
        );
        assert!(e.to_string().contains("in that order"), "{e}");
    }

    #[test]
    fn a_malformed_at_fingerprint_is_a_hard_error_not_a_lint() {
        let e = err("## q\n---\na\n<!-- at: src/lib.rs:1-3 fingerprint: xxh64-ABC -->\n");
        assert!(
            matches!(e, ParseError::InvalidLocator { line: 4, .. }),
            "{e:?}"
        );
    }

    #[test]
    fn an_unknown_or_duplicate_at_field_is_a_hard_error() {
        let e = err("## q\n---\na\n<!-- at: a.rs:1 flavor: cherry -->\n");
        assert!(
            matches!(e, ParseError::InvalidLocator { line: 4, .. }),
            "{e:?}"
        );

        let e = err("## q\n---\na\n<!-- at: a.rs:1 at: b.rs:2 -->\n");
        assert!(
            matches!(e, ParseError::InvalidLocator { line: 4, .. }),
            "{e:?}"
        );
    }

    /// A `given:` is stored verbatim and both study phases hand it to
    /// `DisplayProjector::project` (`src/serve/dto.rs`), so an inline grammar
    /// defect reaches the learner through an ordinary authored card.
    #[test]
    fn a_linked_image_in_a_given_keeps_its_outer_link() {
        let deck = parse("## q\n---\na\n<!-- given: [![moon](moon.jpg)](/uri) -->\n");
        assert_eq!(
            vec!["[![moon](moon.jpg)](/uri)".to_string()],
            deck.cards[0].givens,
            "the given is stored exactly as authored"
        );

        let mut projector = crate::inline::DisplayProjector::default();
        let runs = projector.project(&deck.cards[0].givens[0]);
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(
            "![moon](moon.jpg)", text,
            "and projecting it leaves the outer link's label whole rather than \
             showing raw link syntax: {runs:?}"
        );
    }

    #[test]
    fn given_is_repeatable() {
        let deck = parse(
            "## q\n---\na\n<!-- given: state - the parser position -->\n\
             <!-- given: partial - the card -->\n",
        );
        assert_eq!(
            vec![
                "state - the parser position".to_string(),
                "partial - the card".to_string(),
            ],
            deck.cards[0].givens
        );
    }

    #[test]
    fn a_known_directive_key_with_a_bad_value_is_reported() {
        let deck = parse("---\nreveal: cloze\n---\n## q\n---\na\n<!-- direction: sideways -->\n");
        assert_eq!(None, deck.frontmatter.reveal);
        assert_eq!(None, deck.cards[0].direction);
        assert_eq!(
            vec![bad(2, "reveal", "cloze"), bad(7, "direction", "sideways")],
            deck.lints
        );
    }

    #[test]
    fn an_empty_valued_known_directive_key_is_linted() {
        let deck = parse("## q\n---\na\n<!-- id: -->\n");
        assert_eq!(None, deck.cards[0].token);
        assert_eq!(
            vec![Lint {
                line: 4,
                kind: LintKind::EmptyValue { key: "id".into() }
            }],
            deck.lints
        );
    }

    #[test]
    fn an_unknown_frontmatter_key_is_linted_not_special_cased() {
        let deck = super::parse("deck.md", "---\norigin: /crate\n---\n## q\na\n").unwrap();
        assert!(
            deck.lints.contains(&unknown(2, "origin")),
            "{:?}",
            deck.lints
        );
    }

    #[test]
    fn an_unknown_card_directive_key_is_linted_not_special_cased() {
        let deck = super::parse("deck.md", "## q\na\n<!-- origin: /crate -->\n").unwrap();
        assert!(
            deck.lints.contains(&unknown(3, "origin")),
            "{:?}",
            deck.lints
        );
    }

    #[test]
    fn image_folder_keys_are_ordinary_unknown_keys() {
        let deck = parse("---\nimg-dir: assets\n---\n## q\na\n");
        assert_eq!(vec![unknown(2, "img-dir")], deck.lints);

        let deck = parse("---\nimage-dir: sub\n---\n## q\na\n");
        assert_eq!(vec![unknown(2, "image-dir")], deck.lints);
    }

    // ── Escapes and bytes ──

    #[test]
    fn escaped_structural_markers_render_literal() {
        let deck =
            parse("## q\n---\n\\## x\n\\> y\n\\---\n\\<!-- z -->\n\\```\n> [!NOTE]\n> real note\n");
        assert_eq!(
            vec!["## x", "> y", "---", "<!-- z -->", "```"],
            deck.cards[0].back
        );
        assert_eq!(Some("real note"), deck.cards[0].only_note());
    }

    #[test]
    fn a_backslash_before_anything_else_is_literal() {
        let deck = parse("## q\n---\n\\d is a digit class\n\\# x\n");
        assert_eq!(
            vec!["\\d is a digit class", "# x"],
            deck.cards[0].back,
            "`\\d` is not a marker so it stays literal, while `\\#` now escapes \
             the section marker and gives up its backslash (ruled D5)"
        );
    }

    #[test]
    fn one_leading_bom_is_stripped() {
        let deck = parse("\u{feff}## q\n---\na\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);

        assert!(matches!(
            err("\u{feff}\u{feff}## q\n---\na\n"),
            ParseError::ProseBeforeFirstHeading(1)
        ));
    }

    #[test]
    fn crlf_line_endings_normalize() {
        let deck = parse("## q\r\n\r\n---\r\nan answer\r\n");
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(vec!["an answer"], deck.cards[0].back);
    }

    #[test]
    fn a_c0_control_outside_whitespace_is_a_line_numbered_error() {
        assert_eq!(
            ParseError::ControlChar {
                line: 3,
                found: "U+0007".into()
            },
            err("## q\n---\na\u{7} bell\n")
        );
        assert!(super::parse("deck.md", "## q\n---\na\u{b}b\n").is_ok());
    }

    #[test]
    fn fenced_content_is_verbatim_and_structurally_inert() {
        let deck = parse(
            "## q\n---\nbefore\n```\n## x\n> quoted\n<!-- id: zz -->\n---\n\\## kept\n```\nafter\n",
        );
        assert_eq!(1, deck.cards.len());
        assert_eq!(
            vec![
                "before",
                "```",
                "## x",
                "> quoted",
                "<!-- id: zz -->",
                "---",
                "\\## kept",
                "```",
                "after",
            ],
            deck.cards[0].back
        );
        assert_eq!(None, deck.cards[0].token);
        assert_eq!(None, deck.cards[0].only_note());
        assert!(deck.lints.is_empty());
    }

    // ── Blanks ──

    #[test]
    fn a_span_in_display_math_reveals_as_math_too() {
        let deck = parse("## q\n---\n$$x+y$$\n<!-- blank: span hidden=\"x+y\" -->\n");
        assert_eq!(["$x+y$"], *deck.cards[0].back_for_display());
    }

    #[test]
    fn a_blank_in_prose_reveals_exactly_as_written() {
        let deck =
            parse("## q\n---\nthe value is dropped\n<!-- blank: span hidden=\"dropped\" -->\n");
        assert_eq!(None, deck.cards[0].display_back);
        assert_eq!(["dropped"], *deck.cards[0].back_for_display());
    }

    /// Two spans on one line, only one of them a formula.
    #[test]
    fn only_the_span_inside_the_formula_reveals_as_math() {
        let deck = parse(
            "## q\n---\nthe sign in $x+y$\n<!-- blank: span hidden=\"sign\" -->\n<!-- blank: span hidden=\"x+y\" -->\n",
        );
        assert_eq!(2, deck.cards.len());
        assert_eq!(None, deck.cards[0].display_back);
        assert_eq!(["$x+y$"], *deck.cards[1].back_for_display());
    }

    #[test]
    fn a_group_may_reach_across_answer_lines() {
        let deck = parse(
            "## q\n---\nBerlin is the capital\nof Germany\n<!-- blank: span [c] hidden=\"Berlin\" -->\n<!-- blank: span [c] hidden=\"Germany\" -->\n",
        );
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["Berlin", "Germany"], deck.cards[0].back);
        assert_eq!(vec!["⍰ is the capital", "of ⍰"], deck.cards[0].context);
    }

    #[test]
    fn a_group_of_three_is_one_card() {
        let deck = parse(
            "## q\n---\nx y z\n<!-- blank: span [a] hidden=\"x\" -->\n<!-- blank: span [a] hidden=\"y\" -->\n<!-- blank: span [a] hidden=\"z\" -->\n",
        );
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["x", "y", "z"], deck.cards[0].back);
    }

    /// A merged card asks two spans, which are exact answers rather than the
    /// key points a multi-line plain answer holds, so it stays typed.
    #[test]
    fn a_merged_card_is_typed_at_reconstruct_not_self_graded() {
        let deck = parse(
            "## q\n---\nalpha, beta\n<!-- blank: span [hs] hidden=\"alpha\" -->\n<!-- blank: span [hs] hidden=\"beta\" -->\n",
        );
        assert_eq!(
            crate::answer::Mode::Typing,
            crate::depth::check_for(
                Reveal::Flip,
                crate::depth::Depth::Reconstruct,
                &deck.cards[0]
            )
        );
    }

    #[test]
    fn a_note_addressed_to_a_group_lands_on_the_merged_card() {
        let deck = parse(
            "## q\nalpha, beta, gamma\n\
             > [!NOTE]\n> Shared.\n> hs: Both halves of the opening.\n\
             <!-- blank: span [hs] hidden=\"alpha\" -->\n\
             <!-- blank: span [hs] hidden=\"beta\" -->\n\
             <!-- blank: span hidden=\"gamma\" -->\n",
        );
        let merged = deck
            .cards
            .iter()
            .find(|card| card.back == ["alpha", "beta"])
            .expect("the merged group card");
        let single = deck
            .cards
            .iter()
            .find(|card| card.back == ["gamma"])
            .expect("the ungrouped card");
        assert_eq!(Some("Both halves of the opening."), merged.only_note());
        assert_eq!(Some("Shared."), single.only_note());
    }

    #[test]
    fn a_block_note_naming_a_blanks_answer_is_reported_per_blank() {
        // The spec's motivating fixture: reviewing a later card shows a note
        // that spells out the first blank's answer.
        let deck = parse(
            "## The test pyramid, bottom to top\n\
             Unit, integration, end-to-end\n\
             > [!NOTE]\n> Unit tests sit at the base because they are fastest and most numerous.\n\
             <!-- blank: span hidden=\"Unit\" -->\n\
             <!-- blank: span hidden=\"integration\" -->\n\
             <!-- blank: span hidden=\"end-to-end\" -->\n",
        );
        assert_eq!(
            vec![Lint {
                line: 1,
                kind: LintKind::NoteContainsBlankAnswer {
                    blank: 1,
                    answer: "Unit".to_string()
                }
            }],
            deck.lints,
            "only the blank whose answer appears is named, and 1-based"
        );
    }

    #[test]
    fn an_addressed_note_with_no_block_note_stands_alone() {
        let deck = parse(
            "## q\nUnit, integration\n> [!NOTE]\n> base+: Fastest.\n\
             <!-- blank: span [base] hidden=\"Unit\" -->\n\
             <!-- blank: span hidden=\"integration\" -->\n",
        );
        let base = deck
            .cards
            .iter()
            .find(|card| card.back == ["Unit"])
            .expect("group base's card");
        let other = deck
            .cards
            .iter()
            .find(|card| card.back == ["integration"])
            .expect("the ungrouped card");
        assert_eq!(
            vec![Note {
                badge: Some(Badge::Note),
                body: "Fastest.".to_string()
            }],
            base.notes,
            "the badge belongs to the note it opened"
        );
        assert_eq!(
            Vec::<Note>::new(),
            other.notes,
            "a card that resolves no note resolves no badge either"
        );
    }

    #[test]
    fn two_lines_addressed_to_one_group_join_in_written_order() {
        let deck = parse(
            "## q\nUnit, integration\n\
             > [!NOTE]\n> base: First.\n> base: Second.\n\
             <!-- blank: span [base] hidden=\"Unit\" -->\n\
             <!-- blank: span hidden=\"integration\" -->\n",
        );
        let base = deck
            .cards
            .iter()
            .find(|card| card.back == ["Unit"])
            .expect("group base's card");
        assert_eq!(Some("First.\nSecond."), base.only_note());
    }

    /// A block with no named group cannot be addressing anything, so a note
    /// beginning `2:` is prose and stays prose.
    #[test]
    fn a_note_that_looks_addressed_is_prose_where_no_group_is_named() {
        let deck = parse(
            "## q\nUnit, integration\n> [!NOTE]\n> 2: the second one.\n\
             <!-- blank: span hidden=\"Unit\" -->\n\
             <!-- blank: span hidden=\"integration\" -->\n",
        );
        assert_eq!(Vec::<Lint>::new(), deck.lints);
        for card in &deck.cards {
            assert_eq!(Some("2: the second one."), card.only_note());
        }
    }

    #[test]
    fn an_address_is_separated_from_its_text_by_a_space() {
        let deck = parse(
            "## q\nUnit, integration\n> [!NOTE]\n> base:no space.\n\
             <!-- blank: span [base] hidden=\"Unit\" -->\n\
             <!-- blank: span hidden=\"integration\" -->\n",
        );
        assert_eq!(Vec::<Lint>::new(), deck.lints);
        for card in &deck.cards {
            assert_eq!(Some("base:no space."), card.only_note());
        }
    }

    #[test]
    fn an_address_naming_no_group_of_this_block_is_reported_and_kept() {
        let deck = parse(
            "## q\nUnit, integration\n> [!NOTE]\n> bass: typo.\n\
             <!-- blank: span [base] hidden=\"Unit\" -->\n\
             <!-- blank: span hidden=\"integration\" -->\n",
        );
        assert_eq!(
            vec![Lint {
                line: 1,
                kind: LintKind::NoteNamesNoGroup {
                    name: "bass".to_string()
                }
            }],
            deck.lints
        );
        for card in &deck.cards {
            assert_eq!(
                Some("bass: typo."),
                card.only_note(),
                "the line is still shown rather than lost"
            );
        }
    }

    #[test]
    fn the_pyramid_stops_leaking_once_its_note_is_addressed() {
        let deck = parse(
            "## The test pyramid, bottom to top\n\
             Unit, integration, end-to-end\n\
             > [!NOTE]\n> base: Unit tests sit at the base because they are fastest and most numerous.\n\
             <!-- blank: span [base] hidden=\"Unit\" -->\n\
             <!-- blank: span hidden=\"integration\" -->\n\
             <!-- blank: span hidden=\"end-to-end\" -->\n",
        );
        assert_eq!(
            Vec::<Lint>::new(),
            deck.lints,
            "no other card shows the note that names `Unit`"
        );
        let base = deck
            .cards
            .iter()
            .find(|card| card.back == ["Unit"])
            .expect("group base's card");
        assert!(
            base.only_note()
                .is_some_and(|note| note.starts_with("Unit tests sit at the base"))
        );
        for card in deck.cards.iter().filter(|card| card.back != ["Unit"]) {
            assert_eq!(None, card.only_note());
        }
    }

    #[test]
    fn a_note_naming_no_blank_answer_is_silent() {
        let deck = parse(
            "## q\nUnit, integration\n> [!NOTE]\n> Fastest at the base.\n\
             <!-- blank: span hidden=\"Unit\" -->\n\
             <!-- blank: span hidden=\"integration\" -->\n",
        );
        assert_eq!(Vec::<Lint>::new(), deck.lints);
    }

    #[test]
    fn a_single_blank_block_is_never_reported_however_the_note_reads() {
        // With one blank there is no other card to leak to: the note is on
        // the card whose answer it names, the ordinary way to write one.
        let deck = parse(
            "## q\nUnit tests\n> [!NOTE]\n> Unit tests are fastest.\n\
             <!-- blank: span hidden=\"Unit\" -->\n",
        );
        assert_eq!(Vec::<Lint>::new(), deck.lints);
    }

    #[test]
    fn a_blank_answer_inside_a_longer_word_is_not_a_match() {
        for note in ["Reunites the suites.", "Unitary tests are narrow."] {
            let deck = parse(&format!(
                "## q\nunit, integration\n> [!NOTE]\n> {note}\n\
                 <!-- blank: span hidden=\"unit\" -->\n\
                 <!-- blank: span hidden=\"integration\" -->\n"
            ));
            assert_eq!(
                Vec::<Lint>::new(),
                deck.lints,
                "`unit` inside {note:?} is not the answer appearing"
            );
        }
    }

    #[test]
    fn a_short_blank_answer_is_below_the_reporting_floor() {
        let deck = parse(
            "## q\nTCP, integration\n> [!NOTE]\n> TCP is a protocol.\n\
             <!-- blank: span hidden=\"TCP\" -->\n\
             <!-- blank: span hidden=\"integration\" -->\n",
        );
        assert_eq!(
            Vec::<Lint>::new(),
            deck.lints,
            "three characters match too much prose to be worth reporting"
        );
    }

    #[test]
    fn blank_cards_never_produce_a_reversed_twin() {
        let deck = parse(
            "---\ndirection: both\n---\n## q\n---\na b c\n<!-- direction: both -->\n<!-- blank: span hidden=\"b\" -->\n",
        );
        assert_eq!(Some(Direction::Both), deck.frontmatter.direction);
        assert_eq!(1, deck.cards.len());
        assert!(deck.cards[0].is_blank_card());
        assert!(!deck.cards[0].reversed);
        assert_eq!(None, deck.cards[0].direction);
    }

    #[test]
    fn a_plain_cards_direction_is_recorded_not_expanded() {
        let deck = parse("---\ndirection: both\n---\n## q\n---\na\n<!-- direction: both -->\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(Direction::Both), deck.cards[0].direction);
        assert!(!deck.cards[0].reversed);
    }

    // ── The directives snapshot ──

    #[test]
    fn a_full_directive_fixture_parses_to_exactly_this_snapshot() {
        let text = r#"---
format-version: 1
id: "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f"
source:
  - https://example.org/book
  - notes.md
requires:
  - basics
link:
  - https://docs.rs/tokio
trace: how a keypress becomes a grade
reveal: line
order: sequential
input: draw
direction: both
tasklist: choices-single
table: cards
title: The Title
license: MIT
authors: someone
language: de
revision: 3
created-at: 2026-07-19
---
# The Title

## The question

---
the answer
<!-- reveal: flip -->
<!-- input: type -->
<!-- direction: reverse -->
<!-- at: src/caching.rs:46-66 fingerprint: xxh64-0123456789abcdef asset: sha256-abc123.rs -->
<!-- given: state - the parser position -->
<!-- given: partial - the card -->
<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->
"#;
        let document = parse_document(text).unwrap();
        assert_eq!(
            Frontmatter {
                id: Some("deck-9w2c7x4k1m8q3z5t0v6b2n4d8f".into()),
                authors: vec!["someone".into()],
                created_at: Some("2026-07-19".into()),
                license: Some("MIT".into()),
                title: Some("The Title".into()),
                description: None,
                source: vec!["https://example.org/book".into(), "notes.md".into()],
                requires: vec!["basics".into()],
                link: vec!["https://docs.rs/tokio".into()],
                trace: Some("how a keypress becomes a grade".into()),
                reveal: Some(Reveal::Line),
                order: Some(Order::Sequential),
                input: Some(Input::Draw),
                direction: Some(Direction::Both),
                sampling: None,
                tasklist: Some(Mapping::ChoicesSingle),
                table: Some(Mapping::Cards),
                unspliceable: false,
                personal_for: None,
            },
            document.frontmatter
        );
        assert_eq!(1, document.blocks.len());
        let RawBlock::Card(raw_card) = &document.blocks[0] else {
            panic!("expected a card block");
        };
        assert_eq!(
            CardDirectives {
                regions: Vec::new(),
                crops: Vec::new(),
                token: Some("card-4jkya9q3m8z0tw5v9y2b4n6d8f".into()),
                reveal: Some(Reveal::Flip),
                reveal_line: Some(31),
                input: Some(Input::Type),
                direction: Some(Direction::Reverse),
                sampling: None,
                citations: vec![crate::card::SourceCitation {
                    locator: "src/caching.rs:46-66".into(),
                    fingerprint: Some(0x0123456789abcdef),
                    asset: Some("sha256-abc123.rs".into()),
                    line: 34,
                }],
                diagrams: Vec::new(),
                givens: vec![
                    "state - the parser position".into(),
                    "partial - the card".into(),
                ],
            },
            raw_card.directives
        );
        assert!(document.lints.is_empty(), "{:?}", document.lints);

        let deck = super::parse("deck.md", text).unwrap();
        let card = &deck.cards[0];
        assert_eq!("The question", card.front);
        assert_eq!(vec!["the answer"], card.back);
        assert_eq!(Some(Reveal::Flip), card.reveal);
        assert_eq!(Some(Input::Type), card.input);
        assert_eq!(Some(Direction::Reverse), card.direction);
        assert!(card.images.is_empty());
        assert!(card.images_back.is_empty());
        assert_eq!(
            vec![crate::card::SourceCitation {
                locator: "src/caching.rs:46-66".into(),
                fingerprint: Some(0x0123456789abcdef),
                asset: Some("sha256-abc123.rs".into()),
                line: 34,
            }],
            card.citations
        );
        assert_eq!(2, card.givens.len());
        assert_eq!(
            Some("card-4jkya9q3m8z0tw5v9y2b4n6d8f"),
            card.token.as_deref()
        );
    }

    // ── Inline Markdown images ──

    fn img_srcs(images: &[CardImage]) -> Vec<PathBuf> {
        images.iter().map(|i| i.src.clone()).collect()
    }

    #[test]
    fn an_undivided_back_image_fills_images_back_and_leaves_the_text() {
        let deck = parse("## q\nWaxing\n![](moon.png)\n");
        let card = &deck.cards[0];
        assert_eq!(vec![PathBuf::from("moon.png")], img_srcs(&card.images_back));
        assert!(card.images.is_empty());
        assert_eq!(vec!["Waxing"], card.back);
        assert!(!card.back.join("\n").contains("!["));
    }

    #[test]
    fn a_divided_front_image_fills_images_and_cleans_the_answer() {
        let deck = parse("## What phase?\n![](moon.png)\n\n---\nWaxing\n");
        let card = &deck.cards[0];
        assert_eq!(vec![PathBuf::from("moon.png")], img_srcs(&card.images));
        assert!(card.images_back.is_empty());
        assert_eq!("What phase?", card.front);
        assert_eq!(vec!["Waxing"], card.back);
        assert!(!card.front.contains("!["));
    }

    #[test]
    fn a_divider_directly_under_an_image_is_a_stray() {
        assert_eq!(
            ParseError::StrayDivider(3),
            err("## q\n![](x.png)\n---\nWaxing\n")
        );
    }

    #[test]
    fn two_answer_images_fill_images_back_in_order() {
        let deck = parse("## q\nSee both\n![](a.png)\n![](b.png)\n");
        let card = &deck.cards[0];
        assert_eq!(
            vec![PathBuf::from("a.png"), PathBuf::from("b.png")],
            img_srcs(&card.images_back)
        );
        assert!(card.images.is_empty());
        assert_eq!(vec!["See both"], card.back);
    }

    #[test]
    fn a_divided_front_is_not_scanned_for_cloze_but_yields_images() {
        let deck = parse("## front\n\\blank[pin] stays literal\n![](f.png)\n\n---\nthe answer\n");
        let card = &deck.cards[0];
        assert!(card.front.contains("\\blank[pin]"));
        assert_eq!(vec![PathBuf::from("f.png")], img_srcs(&card.images));
    }

    #[test]
    fn a_blank_card_carries_front_and_back_images() {
        let deck = parse(
            "## front\n![](f.png)\n\n---\nthe answer here\n![](b.png)\n<!-- blank: span hidden=\"answer\" -->\n",
        );
        assert_eq!(1, deck.cards.len());
        let card = &deck.cards[0];
        assert!(card.is_blank_card());
        assert_eq!(vec![PathBuf::from("f.png")], img_srcs(&card.images));
        assert_eq!(vec![PathBuf::from("b.png")], img_srcs(&card.images_back));
    }

    #[test]
    fn an_image_on_a_fenced_line_is_still_recognized() {
        let deck = parse("## q\n---\nbefore\n```\n![a diagram](d.png)\n```\n");
        let card = &deck.cards[0];
        assert_eq!(vec![PathBuf::from("d.png")], img_srcs(&card.images_back));
        assert_eq!(vec!["before", "```", "```"], card.back);
    }

    #[test]
    fn image_references_report_exact_destination_byte_spans() {
        let text = "---\ncover: ![private](ignored.png)\n---\n## q\nprefix ![one](images/Moon.PNG) and ![two](<with space/a.png>)\n";
        let references = image_references(text);

        assert_eq!(2, references.len());
        assert_eq!("images/Moon.PNG", references[0].source);
        assert_eq!("images/Moon.PNG", &text[references[0].destination.clone()]);
        assert_eq!("with space/a.png", references[1].source);
        assert_eq!("with space/a.png", &text[references[1].destination.clone()]);
    }

    #[test]
    fn an_escaped_image_after_a_long_prefix_never_hides_the_following_real_image() {
        let text = "## q\nabcdefghijklmnopqrstuvwxyz0123456789 \\![skip](private.png) ![keep](public.png)\n";
        let references = image_references(text);
        assert_eq!(1, references.len());
        assert_eq!("public.png", references[0].source);
        assert_eq!("public.png", &text[references[0].destination.clone()]);
    }

    #[test]
    fn image_references_ignore_escaped_markers_and_match_fenced_card_images() {
        let text = "## q\n\\![escaped](a.png)\n```\n![code](b.png)\n```\n![real](c.png)\n";
        let references = image_references(text);

        assert_eq!(2, references.len());
        assert_eq!("b.png", references[0].source);
        assert_eq!("c.png", references[1].source);
    }

    #[test]
    fn a_malformed_image_embed_lints_but_the_deck_still_parses() {
        let deck = parse("## q\n---\nsee ![alt](oops\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["see ![alt](oops"], deck.cards[0].back);
        assert!(deck.cards[0].images_back.is_empty());
        assert_eq!(
            vec![Lint {
                line: 3,
                kind: LintKind::ImageMalformed
            }],
            deck.lints
        );
    }

    #[test]
    fn adding_an_image_preserves_the_card_token() {
        let base = parse("## q\nWaxing\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n");
        let with =
            parse("## q\nWaxing\n![](moon.png)\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n");
        let card_base = &base.cards[0];
        let card_with = &with.cards[0];
        assert_eq!(card_base.id(), card_with.id());
        assert_eq!(
            Some("card-9w2c7x4k1m8q3z5t0v6b2n4d8f".to_string()),
            card_with.id()
        );
        assert_eq!(
            vec![PathBuf::from("moon.png")],
            img_srcs(&card_with.images_back)
        );
    }

    // ── Canonical content ──

    #[test]
    fn canonical_content_collapses_prose_but_not_fences() {
        let back: Vec<String> = ["a  b", "```rust", "let  x = 1;", "```", "c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            "The front\na b\n```rust\nlet  x = 1;\n```\nc",
            canonical_content("The  front", &back)
        );
    }

    #[test]
    fn content_fingerprint_is_whitespace_insensitive_but_word_sensitive() {
        let spaced = content_fingerprint("f", &["a  b".to_string()]);
        let tabbed = content_fingerprint("f", &["a\tb".to_string()]);
        let split = content_fingerprint("f", &["a".to_string(), "b".to_string()]);
        let reworded = content_fingerprint("f", &["a c".to_string()]);
        assert_eq!(spaced, tabbed);
        assert_eq!(spaced, split);
        assert_ne!(spaced, reworded);
    }

    // ── Card tables ──

    const CONTAINER: &str = "card-9w2c7x4k1m8q3z5t0v6b2n4d8f";

    #[test]
    fn a_two_column_table_emits_one_card_per_row() {
        let text = format!(
            "| word | meaning |\n|---|---|\n| hund | dog | <!-- r:4k2x9w -->\n| katze | cat | <!-- r:7m3p5q -->\n<!-- cards -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert_eq!(2, deck.cards.len());
        let first = &deck.cards[0];
        assert_eq!("hund", first.front);
        assert_eq!(vec!["dog"], first.back);
        assert_eq!(None, first.only_note());
        assert!(first.context.is_empty(), "an untitled table has no context");
        assert_eq!(Some(CONTAINER), first.token.as_deref());
        assert_eq!(Some("4k2x9w"), first.row.as_deref());
        assert_eq!(Some(format!("{CONTAINER}-t4k2x9w")), first.id());
        assert_eq!(3, first.line);
        let second = &deck.cards[1];
        assert_eq!("katze", second.front);
        assert_eq!(Some(format!("{CONTAINER}-t7m3p5q")), second.id());
        assert_eq!(4, second.line);
    }

    #[test]
    fn a_three_column_table_carries_the_note_and_context() {
        let text = format!(
            "| word | meaning | note |\n|---|---|---|\n| a | b | care | <!-- r:4k2x9w -->\n| c | d | | <!-- r:7m3p5q -->\n<!-- cards -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert_eq!(2, deck.cards.len());
        assert_eq!(Some("care"), deck.cards[0].only_note());
        assert_eq!(None, deck.cards[1].only_note());
        assert!(deck.cards[0].context.is_empty());
    }

    #[test]
    fn an_unstamped_row_stays_id_less_even_under_a_container() {
        let text = format!(
            "| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n| p | q |\n<!-- cards -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert_eq!(Some(format!("{CONTAINER}-t4k2x9w")), deck.cards[0].id());
        assert_eq!(None, deck.cards[1].token);
        assert_eq!(None, deck.cards[1].row);
        assert_eq!(None, deck.cards[1].id());
    }

    #[test]
    fn table_directives_apply_to_every_row_card() {
        let text = format!(
            "| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n| p | q | <!-- r:7m3p5q -->\n<!-- cards -->\n<!-- direction: both -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert_eq!(2, deck.cards.len());
        for card in &deck.cards {
            assert_eq!(Some(Direction::Both), card.direction);
        }
    }

    #[test]
    fn a_pipe_line_without_a_delimiter_stays_answer_content() {
        let deck = parse("## q\n---\n| a | b |\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["| a | b |"], deck.cards[0].back);
    }

    #[test]
    fn a_bare_heading_directly_above_a_table_titles_it_instead_of_erroring() {
        let deck = parse("## q\n| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["q"], deck.cards[0].context);
    }

    #[test]
    fn a_table_after_a_complete_card_is_its_own_block() {
        let deck = parse("## q\n---\nanswer\n\n| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n");
        assert_eq!(2, deck.cards.len());
        assert_eq!(vec!["answer"], deck.cards[0].back);
        assert_eq!("x", deck.cards[1].front);
    }

    #[test]
    fn a_table_inside_a_fence_is_literal_content() {
        let deck = parse("## q\n---\n```\n| a | b |\n|---|---|\n```\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(
            vec!["```", "| a | b |", "|---|---|", "```"],
            deck.cards[0].back
        );
    }

    #[test]
    fn a_table_line_without_a_closing_pipe_is_malformed() {
        assert_eq!(
            ParseError::TableLineMalformed(1),
            err("| a | b\n|---|---|\n<!-- cards -->\n")
        );
        assert_eq!(
            ParseError::TableLineMalformed(3),
            err("| a | b |\n|---|---|\n| x | y\n<!-- cards -->\n")
        );
        assert_eq!(
            ParseError::TableLineMalformed(1),
            err("|\n|---|\n<!-- cards -->\n")
        );
    }

    #[test]
    fn a_table_needs_two_or_three_columns() {
        assert_eq!(
            ParseError::TableColumns { line: 1, found: 1 },
            err("| a |\n|---|\n| x |\n<!-- cards -->\n")
        );
        assert_eq!(
            ParseError::TableColumns { line: 1, found: 4 },
            err("| a | b | c | d |\n|---|---|---|---|\n<!-- cards -->\n")
        );
    }

    #[test]
    fn an_empty_table_ends_on_its_delimiter() {
        let deck = parse("---\ntable: cards\n---\n| a | b |\n|---|---|\n");
        assert_eq!(5, deck.tables[0].end_line);
    }

    #[test]
    fn every_table_line_matches_the_header_width() {
        assert_eq!(
            ParseError::TableDelimiterWidth {
                line: 2,
                found: 1,
                expected: 2
            },
            err("| a | b |\n|---|\n<!-- cards -->\n")
        );
        assert_eq!(
            ParseError::TableRowWidth {
                line: 3,
                found: 3,
                expected: 2
            },
            err("| a | b |\n|---|---|\n| x | y | z |\n<!-- cards -->\n")
        );
    }

    #[test]
    fn an_image_in_a_cell_is_refused() {
        assert_eq!(
            ParseError::TableCellImage(3),
            err("| a | b |\n|---|---|\n| ![alt](x.png) | y |\n<!-- cards -->\n")
        );
    }

    #[test]
    fn escaped_images_in_cells_stay_legal_and_a_real_one_after_them_still_refuses() {
        let deck = parse("| a | b |\n|---|---|\n| \\![x] \\![y] | z |\n<!-- cards -->\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!("\\![x] \\![y]", deck.cards[0].front);

        assert_eq!(
            ParseError::TableCellImage(3),
            err("| a | b |\n|---|---|\n| \\![x] ![y](p.png) | z |\n<!-- cards -->\n")
        );
    }

    #[test]
    fn an_invalid_or_duplicate_row_stamp_is_refused() {
        assert_eq!(
            ParseError::TableRowStamp {
                line: 3,
                value: "xyz".into()
            },
            err("| a | b |\n|---|---|\n| x | y | <!-- r:xyz -->\n<!-- cards -->\n")
        );
        assert_eq!(
            ParseError::TableDuplicateStamp {
                line: 4,
                value: "4k2x9w".into()
            },
            err(
                "| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n| p | q | <!-- r:4k2x9w -->\n<!-- cards -->\n"
            )
        );
    }

    #[test]
    fn only_comment_machinery_may_follow_a_table() {
        assert_eq!(
            ParseError::LeadingInvocation {
                line: 7,
                word: "cards".into()
            },
            err("# S\n\n| a | b |\n|---|---|\n| x | y |\nstray prose\n<!-- cards -->\n"),
            "prose between a table and its invocation un-tables the block, so the invocation floats"
        );
        assert_eq!(
            ParseError::TableTrailing(5),
            err("| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n> a note\n")
        );
        assert_eq!(
            ParseError::TableTrailing(6),
            err("| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n\n| z | w |\n")
        );
    }

    #[test]
    fn an_empty_front_or_back_cell_is_refused() {
        assert_eq!(
            ParseError::EmptyFront(3),
            err("| a | b |\n|---|---|\n| | y | <!-- r:4k2x9w -->\n<!-- cards -->\n")
        );
        assert_eq!(
            ParseError::FrontWithoutAnswer(3),
            err("| a | b |\n|---|---|\n| x | |\n<!-- cards -->\n")
        );
    }

    #[test]
    fn an_escaped_pipe_stays_in_the_cell() {
        let deck = parse("| a | b |\n|---|---|\n| x \\| y | z |\n<!-- cards -->\n");
        assert_eq!("x | y", deck.cards[0].front);
    }

    #[test]
    fn a_single_hyphen_delimiter_is_valid_gfm_and_accepted() {
        // GFM defines a delimiter cell as `:?-+:?` with no minimum hyphen
        // count, so `| - |` is a table in every GFM renderer.
        let deck = parse("| front | back |\n| - | -- |\n| question | answer |\n<!-- cards -->\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!("question", deck.cards[0].front);
    }

    #[test]
    fn alignment_colons_in_the_delimiter_are_accepted() {
        let deck = parse("| a | b |\n|:---|---:|\n| x | y |\n<!-- cards -->\n");
        assert_eq!(1, deck.cards.len());
    }

    #[test]
    fn adjacent_tables_split_on_the_second_header() {
        let deck = parse(
            "| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n| c | d |\n|---|---|\n| z | w |\n<!-- cards -->\n",
        );
        assert_eq!(2, deck.cards.len());
        assert!(deck.cards[0].context.is_empty());
        assert!(deck.cards[1].context.is_empty());
    }

    #[test]
    fn an_empty_heading_above_a_table_becomes_its_title() {
        let text = format!(
            "## Verbs of arguing\n| word | meaning |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n<!-- cards -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert_eq!(1, deck.cards.len(), "the heading is a title, not a card");
        assert_eq!(vec!["Verbs of arguing"], deck.cards[0].context);
        assert_eq!(Some(format!("{CONTAINER}-t4k2x9w")), deck.cards[0].id());
    }

    #[test]
    fn a_heading_with_answer_content_before_a_table_stays_a_card() {
        let deck = parse("## q\nanswer\n| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n");
        assert_eq!(2, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(vec!["answer"], deck.cards[0].back);
        assert!(deck.cards[1].context.is_empty(), "the table is untitled");
    }

    #[test]
    fn a_heading_with_only_a_note_keeps_being_a_card_and_fails_loudly() {
        assert_eq!(
            ParseError::FrontWithoutAnswer(1),
            err("## q\n> [!NOTE]\n> a note\n| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n")
        );
    }

    #[test]
    fn a_heading_id_becomes_the_tables_container_id() {
        let text = format!(
            "## Title\n| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n<!-- cards -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert_eq!(Some(format!("{CONTAINER}-t4k2x9w")), deck.cards[0].id());
    }

    #[test]
    fn heading_directives_apply_to_the_titled_tables_rows() {
        let text = format!(
            "## Title\n| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n<!-- direction: both -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert!(
            deck.cards
                .iter()
                .all(|card| card.direction == Some(Direction::Both))
        );
    }

    #[test]
    fn a_blank_line_between_title_and_table_still_titles() {
        let deck = parse("## Title\n\n| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["Title"], deck.cards[0].context);
    }

    #[test]
    fn a_table_id_line_must_hold_a_base_card_id() {
        assert_eq!(
            ParseError::InvalidCardId {
                line: 5,
                value: format!("{CONTAINER}-2"),
            },
            err(&format!(
                "| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n<!-- id: {CONTAINER}-2 -->\n"
            ))
        );
    }

    // ── Image regions (ADR 0034): binding and cross-region rules ──

    #[test]
    fn a_geometric_region_binds_to_the_nearest_preceding_image_on_its_side() {
        let deck = parse(
            "## bones\n![](a.png)\n![](b.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 -->\n\n---\n![](c.png)\n<!-- blank: rect x=3 y=3 width=4 height=4 -->\nanswer\n",
        );
        let card = &deck.cards[0];
        assert_eq!(2, card.images.len(), "both front images survive");
        assert!(
            card.images[0].regions.is_empty(),
            "the farther image gets nothing"
        );
        assert_eq!(
            1,
            card.images[1].regions.len(),
            "the nearest preceding image binds"
        );
        assert_eq!(
            1,
            card.images_back[0].regions.len(),
            "the back region binds on its own side"
        );
        assert!(card.span_regions.is_empty());
    }

    #[test]
    fn a_geometric_region_without_a_media_element_on_its_side_is_rejected() {
        let error =
            err("## q\n<!-- blank: rect x=1 y=1 width=2 height=2 -->\n\n---\n![](a.png)\nanswer\n");
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("preceding media element"), "{message}");
    }

    #[test]
    fn a_span_region_binds_to_the_answer_block_without_any_media() {
        let deck = parse("## q\nanswer with der Artikel\n<!-- blank: span hidden=\"der\" -->\n");
        let card = &deck.cards[0];
        assert_eq!(1, card.span_regions.len());
        assert!(card.images.is_empty() && card.images_back.is_empty());
    }

    #[test]
    fn a_span_enabled_answer_can_start_with_an_entity() {
        let deck =
            parse("## q\n---\n&amp; welcome\n<!-- blank: span hidden=\"welcome\" b:a1b2c3 -->\n");
        assert_eq!(vec!["welcome"], deck.cards[0].back);
        assert_eq!(
            vec!["&amp; ⍰"],
            deck.cards[0].context,
            "the unrelated entity remains authored while the span masks ordinary prose"
        );
    }

    #[test]
    fn a_span_splice_covers_an_entitys_whole_authored_footprint() {
        let deck =
            parse("## q\nTom &amp; Jerry\n<!-- blank: span hidden=\"& Jerry\" b:a1b2c3 -->\n");
        assert_eq!(
            vec!["Tom ⍰"],
            deck.cards[0].context,
            "the splice takes all of `&amp;`, neither the space before it nor leaving `amp;`"
        );
    }

    #[test]
    fn a_nested_fence_records_one_interior_including_the_shorter_delimiter() {
        let deck = parse(
            "## q\n````mermaid\nflowchart LR\n```\n  A[Load] --> B\n````\n<!-- blank: span hidden=\"Load\" -->\n",
        );
        let card = &deck.cards[0];
        assert_eq!(1, card.answer_fences.len(), "{:?}", card.answer_fences);
        assert_eq!(
            "flowchart LR\n```\n  A[Load] --> B",
            card.answer_fences[0].interior.as_ref(),
            "the inner delimiter is interior, not a closer"
        );
    }

    #[test]
    fn a_fence_shaped_line_with_info_is_captured_interior_not_a_closer() {
        let deck = parse("## q\n````mermaid\n````x\ny\n````\n<!-- blank: span hidden=\"y\" -->\n");
        let card = &deck.cards[0];
        assert_eq!(1, card.answer_fences.len(), "{:?}", card.answer_fences);
        assert_eq!(
            "````x\ny",
            card.answer_fences[0].interior.as_ref(),
            "the capture agrees with the units walk: an info line never closes"
        );
    }

    #[test]
    fn a_span_inside_a_mermaid_fence_records_its_interior_range_and_fingerprint() {
        let deck = parse(
            "## q\n```mermaid\nflowchart LR\n  A[Load] --> B\n```\n<!-- blank: span hidden=\"Load\" -->\n",
        );
        let card = &deck.cards[0];
        assert!(card.region.is_some(), "the blank makes a region card");
        assert_eq!(1, card.answer_fences.len());
        let fence = &card.answer_fences[0];
        let interior = "flowchart LR\n  A[Load] --> B";
        assert_eq!(
            interior,
            fence.interior.as_ref(),
            "the unmasked bytes ride the record"
        );
        assert_eq!(crate::diagram::fingerprint(interior), fence.fingerprint);
        assert_eq!(1, fence.spans.len());
        let span = &fence.spans[0];
        assert_eq!(6, span.line, "the span's identity is its directive line");
        assert_eq!(
            "Load",
            &interior[span.start..span.end],
            "the range addresses the hidden text in the unmasked interior"
        );
    }

    #[test]
    fn fence_records_keep_block_order_and_skip_non_mermaid_and_prose_spans() {
        let deck = parse(concat!(
            "## q\n",
            "```mermaid\nflowchart LR\n  A[First] --> B\n```\n",
            "```rust\nlet code = 1;\n```\n",
            "```MERMAID\nflowchart TD\n  C[Second] --> D\n```\n",
            "prose with der Artikel\n",
            "<!-- blank: span hidden=\"First\" -->\n",
            "<!-- blank: span hidden=\"Second\" -->\n",
            "<!-- cover: span hidden=\"der\" -->\n",
        ));
        let card = &deck.cards[0];
        assert_eq!(
            2,
            card.answer_fences.len(),
            "mermaid fences only (case-insensitive), in order"
        );
        let first = &card.answer_fences[0];
        let second = &card.answer_fences[1];
        assert_eq!(
            crate::diagram::fingerprint("flowchart LR\n  A[First] --> B"),
            first.fingerprint
        );
        assert_eq!(
            crate::diagram::fingerprint("flowchart TD\n  C[Second] --> D"),
            second.fingerprint
        );
        assert_eq!(1, first.spans.len(), "each fence sees only its own spans");
        assert_eq!(1, second.spans.len());
        assert_eq!(
            "Second",
            &"flowchart TD\n  C[Second] --> D"[second.spans[0].start..second.spans[0].end]
        );
        let recorded: usize = card.answer_fences.iter().map(|f| f.spans.len()).sum();
        assert_eq!(2, recorded, "the prose cover appears in no fence record");
    }

    #[test]
    fn empty_unclosed_and_indented_fences_follow_the_unit_grammar() {
        let deck = parse("## q\n```mermaid\n```\nanswer\n");
        assert_eq!(
            1,
            deck.cards[0].answer_fences.len(),
            "one record per closed mermaid fence, empty included: a context \
             interior emptied by image-line dropping is indistinguishable \
             from an authored-empty one, so only the uniform rule aligns"
        );
        assert_eq!(
            crate::diagram::fingerprint(""),
            deck.cards[0].answer_fences[0].fingerprint
        );
        let deck = parse("## q\n```mermaid\nflowchart LR\nanswer without a closer\n");
        assert!(
            deck.cards[0].answer_fences.is_empty(),
            "an unclosed fence produces no unit, so it produces no record"
        );
        let deck = parse("## q\n```mermaid\nflowchart LR\n  ```\nanswer\n");
        assert_eq!(
            crate::diagram::fingerprint("flowchart LR"),
            deck.cards[0].answer_fences[0].fingerprint,
            "an indented close line closes the fence in the unit grammar, so \
             the record ends where the unit ends"
        );
        let deck = parse("## q\n  ```mermaid\n  flowchart LR\n  ```\nanswer\n");
        assert_eq!(
            1,
            deck.cards[0].answer_fences.len(),
            "an indented fence is a fence to the unit grammar, so it is recorded"
        );
        assert_eq!(
            crate::diagram::fingerprint("flowchart LR"),
            deck.cards[0].answer_fences[0].fingerprint,
            "the scanner dedents non-fence lines, so in card space the fence \
             is column-0 and its interior is dedented; it can never resolve \
             (freeze only stamps document column-0 fences verbatim)"
        );
    }

    #[test]
    fn a_second_occurrence_span_records_the_second_range_in_the_fence() {
        let deck = parse(concat!(
            "## q\n",
            "```mermaid\nflowchart LR\n  Cache[store] --> B[Cache]\n```\n",
            "<!-- blank: span hidden=\"Cache\" occurrence=2 -->\n",
        ));
        let interior = "flowchart LR\n  Cache[store] --> B[Cache]";
        let fence = &deck.cards[0].answer_fences[0];
        assert_eq!(1, fence.spans.len());
        let span = &fence.spans[0];
        assert_eq!("Cache", &interior[span.start..span.end]);
        assert_eq!(
            interior.rfind("Cache").unwrap(),
            span.start,
            "n=2 addresses the second occurrence, not the first"
        );
    }

    #[test]
    fn a_media_element_takes_at_most_one_crop() {
        let error = err(
            "## q\n![](a.png)\n<!-- crop: rect x=0 y=0 width=9 height=9 -->\n<!-- crop: rect x=1 y=1 width=2 height=2 -->\n\n---\nanswer\n",
        );
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("at most one"), "{message}");
    }

    #[test]
    fn one_media_element_carries_one_unit_across_regions_and_crop() {
        let error = err(
            "## q\n![](a.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 -->\n<!-- blank: rect x=1% y=1% width=2% height=2% -->\n\n---\nanswer\n",
        );
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("same unit"), "{message}");

        let error = err(
            "## q\n![](a.png)\n<!-- crop: rect x=0% y=0% width=9% height=9% -->\n<!-- blank: rect x=1 y=1 width=2 height=2 -->\n\n---\nanswer\n",
        );
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("same unit"), "{message}");
    }

    #[test]
    fn viewport_bounds_reject_an_invisible_blank_and_accept_partial_overlap() {
        // Touching the crop edge is outside: no positive-area intersection.
        let error = err(
            "## q\n![](a.png)\n<!-- crop: rect x=0 y=0 width=100 height=100 -->\n<!-- blank: rect x=100 y=0 width=10 height=10 -->\n\n---\nanswer\n",
        );
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("visible area"), "{message}");

        // Partial overlap is legal and clipped.
        let deck = parse(
            "## q\n![](a.png)\n<!-- crop: rect x=0 y=0 width=100 height=100 -->\n<!-- blank: rect x=90 y=0 width=20 height=20 -->\n\n---\nanswer\n",
        );
        assert_eq!(1, deck.cards[0].images[0].regions.len());

        // The ADR's no-crop percentage example: x=100% width=10% clips to nothing.
        let error = err(
            "## q\n![](a.png)\n<!-- blank: rect x=100% y=0% width=10% height=10% -->\n\n---\nanswer\n",
        );
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("visible area"), "{message}");
    }

    #[test]
    fn an_empty_cover_is_dropped_rather_than_rejected() {
        let deck = parse(
            "## q\n![](a.png)\n<!-- crop: rect x=0 y=0 width=50 height=50 -->\n<!-- cover: rect x=60 y=60 width=5 height=5 -->\n\n---\nanswer\n",
        );
        let image = &deck.cards[0].images[0];
        assert!(
            image.regions.is_empty(),
            "the invisible cover creates nothing and errors nothing"
        );
        assert!(image.crop.is_some());
    }

    #[test]
    fn a_region_directive_on_a_card_table_is_rejected_not_dropped() {
        let error = err(
            "| a | b |\n|---|---|\n| x | y |\n<!-- cards -->\n<!-- blank: rect x=1 y=1 width=2 height=2 -->\n",
        );
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("card table"), "{message}");
    }

    #[test]
    fn region_and_crop_survive_onto_the_stored_card_image() {
        let deck = parse(
            "## q\n![](a.png)\n<!-- crop: rect x=0 y=0 width=50 height=50 -->\n<!-- blank: rect x=1 y=1 width=2 height=2 b:a1b2c3 -->\n\n---\nanswer\n",
        );
        let image = &deck.cards[0].images[0];
        assert_eq!(1, image.regions.len());
        assert_eq!(Some("a1b2c3"), image.regions[0].stamp.as_deref());
        assert_eq!("50", image.crop.as_ref().unwrap().width.literal);
    }

    #[test]
    fn a_quoted_hidden_value_accepts_ordinary_non_ascii_text_without_panicking() {
        let deck = parse("## German noun\nanswer for Bär\n<!-- blank: span hidden=\"Bär\" -->\n");
        assert_eq!(Some("Bär"), deck.cards[0].span_regions[0].hidden.as_deref());
    }

    #[test]
    fn a_region_immediately_after_the_divider_cannot_bind_to_front_media() {
        let error = err(
            "## q\n![](front.png)\n\n---\n<!-- blank: rect x=1 y=1 width=2 height=2 -->\nanswer\n",
        );
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("preceding media element"), "{message}");
    }

    #[test]
    fn percentage_crop_and_blank_are_clipped_against_the_source_viewport() {
        let error = err(
            "## q\n![](a.png)\n<!-- crop: rect x=100% y=0% width=10% height=10% -->\n<!-- blank: rect x=100% y=0% width=10% height=10% -->\n\n---\nanswer\n",
        );
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("visible area"), "{message}");
    }

    #[test]
    fn a_region_directive_before_any_card_is_rejected_not_silently_dropped() {
        let error = err("<!-- blank: rect x=1 y=1 width=2 height=2 -->\n## q\nanswer\n");
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("media element"), "{message}");
    }

    // ── Region cards (ADR 0034): assembly ──

    const RTOK: &str = "card-regionregionregionregion";

    fn region_deck(regions: &str) -> String {
        format!(
            "## name the parts\n![](hand.png)\n{regions}\n\n---\nthe parts\n<!-- id: {RTOK} -->\n"
        )
    }

    #[test]
    fn a_stamped_blank_yields_a_region_card_with_the_literal_b_id() {
        let deck = parse(&region_deck(
            r#"<!-- blank: rect x=1 y=1 width=2 height=2 hidden="lunate" b:a1b2c3 -->"#,
        ));
        assert_eq!(
            1,
            deck.cards.len(),
            "the template produces only its region card"
        );
        let region_card = &deck.cards[0];
        assert_eq!(Some(format!("{RTOK}-ba1b2c3")), region_card.id());
        assert_eq!(vec!["lunate"], region_card.back);
        assert_eq!("name the parts", region_card.front);
        assert_eq!(
            1,
            region_card.images.len(),
            "the media rides along for masking"
        );
    }

    #[test]
    fn a_named_group_is_one_card_asking_every_member_with_a_derived_id() {
        let two = |a: &str, b: &str| {
            parse(&region_deck(&format!(
                "<!-- blank: rect [carpals] x=1 y=1 width=2 height=2 hidden=\"lunate\" b:{a} -->\n<!-- blank: rect [carpals] x=5 y=5 width=2 height=2 hidden=\"hamate\" b:{b} -->"
            )))
        };
        let deck = two("a1b2c3", "d4e5f6");
        assert_eq!(1, deck.cards.len(), "one group card, not one per member");
        let group = &deck.cards[0];
        assert_eq!(
            Some(format!("{RTOK}-gchsbz14b1a30x")),
            group.id(),
            "the frozen vector, derived live"
        );
        assert_eq!(
            vec!["lunate", "hamate"],
            group.back,
            "the card asks every member"
        );

        let swapped = two("d4e5f6", "a1b2c3");
        assert_eq!(
            deck.cards[0].id(),
            swapped.cards[0].id(),
            "member order in the file never changes the id"
        );
    }

    #[test]
    fn a_mixed_shape_group_keeps_its_answers_in_file_order() {
        let deck = parse(
            "## identify both\n---\nalpha beta\n<!-- blank: span [pair] hidden=\"alpha\" b:a1b2c3 -->\n![](diagram.png)\n<!-- blank: rect [pair] x=1 y=1 width=2 height=2 hidden=\"diagram\" b:d4e5f6 -->\n<!-- id: card-mixed1 -->\n",
        );
        let group = deck
            .cards
            .iter()
            .find(|card| card.region.is_some())
            .unwrap();

        assert_eq!(
            vec!["alpha", "diagram"],
            group.back,
            "a grouped answer follows the author's source order across shapes"
        );
    }

    #[test]
    fn changing_group_membership_changes_the_derived_id() {
        let deck = parse(&region_deck(
            r#"<!-- blank: rect [g] x=1 y=1 width=2 height=2 hidden="a" b:a1b2c3 -->"#,
        ));
        let grown = parse(&region_deck(
            "<!-- blank: rect [g] x=1 y=1 width=2 height=2 hidden=\"a\" b:a1b2c3 -->\n<!-- blank: rect [g] x=5 y=5 width=2 height=2 hidden=\"b\" b:d4e5f6 -->",
        ));
        assert_ne!(deck.cards[0].id(), grown.cards[0].id());
    }

    #[test]
    fn a_cover_produces_no_card_and_a_cover_only_deck_is_undisturbed() {
        let deck = parse(&region_deck(
            r#"<!-- cover: rect x=1 y=1 width=2 height=2 hidden="legend" -->"#,
        ));
        assert_eq!(1, deck.cards.len(), "a cover never asks");
        assert_eq!(
            1,
            deck.cards[0].images[0].regions.len(),
            "the cover still rides the media for drawing"
        );
    }

    #[test]
    fn a_group_mixing_hidden_presence_is_rejected() {
        let error = err(&region_deck(
            "<!-- blank: rect [g] x=1 y=1 width=2 height=2 hidden=\"a\" b:a1b2c3 -->\n<!-- blank: rect [g] x=5 y=5 width=2 height=2 b:d4e5f6 -->",
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("mixes"), "{message}");
    }

    #[test]
    fn an_unstamped_blank_yields_an_idless_region_card() {
        let deck = parse(&region_deck(
            r#"<!-- blank: rect x=1 y=1 width=2 height=2 hidden="lunate" -->"#,
        ));
        let region_card = &deck.cards[0];
        assert_eq!(
            None,
            region_card.id(),
            "no usable id until the stamper reconciles"
        );
        assert!(region_card.region.is_some());
    }

    #[test]
    fn an_unlabelled_blank_asks_with_an_empty_back() {
        let deck = parse(&region_deck(
            "<!-- blank: rect x=1 y=1 width=2 height=2 b:a1b2c3 -->",
        ));
        let region_card = &deck.cards[0];
        assert_eq!(Some(format!("{RTOK}-ba1b2c3")), region_card.id());
        assert!(
            region_card.back.is_empty(),
            "the reveal is visual: unmasking is the answer"
        );
    }
    #[test]
    fn a_blank_bearing_block_is_a_template_producing_only_its_region_cards() {
        let deck = parse(&region_deck(
            "<!-- blank: rect x=1 y=1 width=2 height=2 hidden=\"lunate\" b:a1b2c3 -->\n<!-- blank: rect x=5 y=5 width=2 height=2 hidden=\"hamate\" b:d4e5f6 -->",
        ));
        assert_eq!(2, deck.cards.len(), "two blanks, two region cards");
        assert!(
            deck.cards.iter().all(|card| card.region.is_some()),
            "no plain card exists beside a template's region cards"
        );
    }

    #[test]
    fn notes_ride_every_region_card() {
        let deck = parse(&format!(
            "## name the parts\n![](hand.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 hidden=\"lunate\" b:a1b2c3 -->\n<!-- blank: rect x=5 y=5 width=2 height=2 hidden=\"hamate\" b:d4e5f6 -->\n\n---\nthe parts\n> [!NOTE]\n> the lunate sits center\n<!-- id: {RTOK} -->\n"
        ));
        for card in &deck.cards {
            assert_eq!(
                Some("the lunate sits center"),
                card.only_note(),
                "the block's note rides every region card, as cloze notes do"
            );
        }
    }

    #[test]
    fn a_task_list_answer_plus_a_blank_region_is_a_composition_error() {
        let error = err(&format!(
            "## pick\n---\n- [x] alpha\n- [ ] beta\n<!-- choices-single -->\n<!-- blank: span hidden=\"alpha\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("task-list"), "{message}");
    }

    #[test]
    fn an_incomplete_task_list_plus_a_blank_region_is_still_a_composition_error() {
        let error = err(&format!(
            "## pick\n---\n- [ ] alpha\n- [ ] beta\n<!-- choices-single -->\n<!-- blank: span hidden=\"alpha\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("task-list"), "{message}");
    }

    #[test]
    fn covers_and_crops_stay_legal_beside_blanks_and_task_lists() {
        let cloze = parse(&format!(
            "## q\n![](a.png)\n<!-- cover: rect x=1 y=1 width=2 height=2 -->\n<!-- crop: rect x=0 y=0 width=9 height=9 -->\n\n---\nw z y\n<!-- blank: span hidden=\"z\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert!(
            cloze.cards[0].is_blank_card(),
            "a cover is a display transform, not a template: the span cards stand"
        );
        assert_eq!(1, cloze.cards[0].images[0].regions.len());

        let choice = parse(&format!(
            "## pick\n![](a.png)\n<!-- cover: rect x=1 y=1 width=2 height=2 -->\n\n---\n- [x] alpha\n- [ ] beta\n<!-- choices-single -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(1, choice.cards.len());
        assert!(!choice.cards[0].authored_distractors.is_empty());
    }

    #[test]
    fn a_span_binds_into_styled_contents_over_the_stream() {
        let deck = parse(&format!(
            "## q\n---\nthe **lunate** bone\n<!-- blank: span hidden=\"lunate\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["lunate"], deck.cards[0].back);
    }

    #[test]
    fn a_span_crossing_a_style_boundary_is_rejected_naming_it() {
        let error = err(&format!(
            "## q\n---\n**New** York is big\n<!-- blank: span hidden=\"New York\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("style boundary"), "{message}");
    }

    #[test]
    fn a_span_cannot_bind_to_a_markdown_link_destination() {
        let error = err(&format!(
            "## q\n---\nread [the guide](https://secret.example/path)\n<!-- blank: span hidden=\"secret.example\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("occurs 0 time(s)"), "{message}");
    }

    #[test]
    fn a_span_cannot_bind_to_a_balanced_link_destination_suffix() {
        let error = err(&format!(
            "## q\n---\nread [the article](https://example.test/a(part)suffix) now\n<!-- blank: span hidden=\"suffix\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("occurs 0 time(s)"), "{message}");
    }

    #[test]
    fn a_whole_math_formula_is_a_legal_span_blank() {
        let deck = parse(&format!(
            "## q\n---\nsum $x+y$ here\n<!-- blank: span hidden=\"x+y\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(1, deck.cards.len());
        assert_eq!(
            vec!["sum $\u{2370}$ here"],
            deck.cards[0].context,
            "the whole-piece carve-out masks the formula to the blank marker"
        );
    }

    #[test]
    fn a_partial_match_containing_a_structural_token_is_rejected() {
        for hidden in ["x^", "^2"] {
            let error = err(&format!(
                "## q\n---\n$x^2$\n<!-- blank: span hidden=\"{hidden}\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
            ));
            let ParseError::InvalidRegion { message, .. } = error else {
                panic!("expected InvalidRegion for {hidden}, got {error:?}");
            };
            assert!(
                message.contains("structural token `^`"),
                "{hidden}: {message}"
            );
        }
    }

    #[test]
    fn a_matrix_column_separator_is_never_inside_a_span_match() {
        let line = r"$\begin{pmatrix}a & b\end{pmatrix}$";
        let error = err(&format!(
            "## q\n---\n{line}\n<!-- blank: span hidden=\"a &\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("structural token `&`"), "{message}");
    }

    #[test]
    fn a_span_endpoint_inside_a_control_word_is_rejected_naming_it() {
        let error = err(&format!(
            "## q\n---\n$x \\leq y$\n<!-- blank: span hidden=\"q\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("\\leq"), "{message}");
    }

    #[test]
    fn escaped_braces_are_legal_inside_and_as_a_span_match() {
        let line = r"$A=\{x\}$";
        let hidden = r"\\{x\\}";
        let deck = parse(&format!(
            "## q\n---\n{line}\n<!-- blank: span hidden=\"{hidden}\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["$A=\u{2370}$"], deck.cards[0].context);
    }

    #[test]
    fn a_matrix_cell_is_a_complete_structural_unit() {
        let line = r"$\begin{pmatrix}a & b\end{pmatrix}$";
        let deck = parse(&format!(
            "## q\n---\n{line}\n<!-- blank: span hidden=\"a\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(
            vec!["$\\begin{pmatrix}\u{2370} & b\\end{pmatrix}$"],
            deck.cards[0].context
        );
    }

    #[test]
    fn a_group_interior_binds_at_equal_depth_and_a_group_split_is_rejected() {
        let line = r"$\frac{ab}{cd}$";
        let deck = parse(&format!(
            "## q\n---\n{line}\n<!-- blank: span hidden=\"ab\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(
            vec!["$\\frac{\u{2370}}{cd}$"],
            deck.cards[0].context,
            "the equal-depth interior of one group is a unit"
        );

        let hidden = r"ab}{cd";
        let error = err(&format!(
            "## q\n---\n{line}\n<!-- blank: span hidden=\"{hidden}\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("brace group"), "{message}");
    }

    #[test]
    fn a_partial_match_containing_a_command_is_rejected_in_v1() {
        let error = err(&format!(
            "## q\n---\n$x + \\gamma + y$\n<!-- blank: span hidden=\"\\\\gamma\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("control sequence `\\gamma`"), "{message}");
    }

    #[test]
    fn left_right_and_begin_end_pairs_are_never_split() {
        for (line, hidden) in [
            (r"$\left( x \right)$", r"\\left( x"),
            (r"$\begin{pmatrix}a\end{pmatrix}$", r"\\begin{pmatrix}a"),
        ] {
            let error = err(&format!(
                "## q\n---\n{line}\n<!-- blank: span hidden=\"{hidden}\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
            ));
            let ParseError::InvalidRegion { message, .. } = error else {
                panic!("expected InvalidRegion for {hidden}, got {error:?}");
            };
            assert!(message.contains("control sequence"), "{hidden}: {message}");
        }
    }

    #[test]
    fn a_whole_formula_blank_bypasses_the_token_rules() {
        for (line, hidden) in [
            (r"$x^2$", r"x^2"),
            (
                r"$\begin{pmatrix}a & b\end{pmatrix}$",
                r"\\begin{pmatrix}a & b\\end{pmatrix}",
            ),
        ] {
            let deck = parse(&format!(
                "## q\n---\n{line}\n<!-- blank: span hidden=\"{hidden}\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
            ));
            assert_eq!(1, deck.cards.len(), "{hidden}");
            assert_eq!(vec!["$\u{2370}$"], deck.cards[0].context, "{hidden}");
        }
    }

    #[test]
    fn space_padded_dollars_are_prose_so_the_span_binds_as_text() {
        let deck = parse(&format!(
            "## q\n---\n$ \\gamma $\n<!-- blank: span hidden=\"\\\\gamma\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(
            vec!["$ \u{2370} $"],
            deck.cards[0].context,
            "whitespace-adjacent dollars never open math, so the token rules do not apply"
        );
    }

    #[test]
    fn a_math_comment_does_not_consume_the_first_visible_occurrence() {
        let deck = parse(&format!(
            "## q\n---\n$a % target$ target\n<!-- blank: span hidden=\"target\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));

        assert_eq!(
            vec!["$a % target$ ⍰"],
            deck.cards[0].context,
            "Ratex does not render source after `%`, so that occurrence is not learner-visible"
        );
    }

    #[test]
    fn a_span_inside_phantom_is_rejected_as_not_learner_visible() {
        let error = err(&format!(
            "## q\n---\n$\\phantom{{target}}$\n<!-- blank: span hidden=\"target\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert!(
            matches!(error, ParseError::InvalidRegion { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_span_inside_math_verb_is_rejected_because_the_blank_stays_literal() {
        let error = err(&format!(
            "## q\n---\n$\\verb|target|$\n<!-- blank: span hidden=\"target\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert!(
            matches!(error, ParseError::InvalidRegion { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn an_unterminated_verb_after_a_span_is_a_loud_binding_error() {
        let error = err(&format!(
            "## q\n---\n$target + \\verb|abc$\n<!-- blank: span hidden=\"target\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert!(
            matches!(error, ParseError::InvalidRegion { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn ordinary_atoms_bind_in_inline_and_display_math() {
        for (line, hidden, masked) in [
            (r"$x \leq y$", "y", "$x \\leq \u{2370}$"),
            (r"$$a + b$$", "b", "$$a + \u{2370}$$"),
        ] {
            let deck = parse(&format!(
                "## q\n---\n{line}\n<!-- blank: span hidden=\"{hidden}\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
            ));
            assert_eq!(vec![masked.to_string()], deck.cards[0].context, "{line}");
        }
    }

    #[test]
    fn an_authored_malformed_formula_under_a_span_is_a_loud_binding_error() {
        let line = r"$x^{2$";
        let error = err(&format!(
            "## q\n---\n{line}\n<!-- blank: span hidden=\"x\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("does not parse"), "{message}");
    }

    #[test]
    fn a_structurally_legal_mask_that_fails_the_renderer_is_a_loud_binding_error() {
        let error = err(&format!(
            "## q\n---\n$x^2$\n<!-- blank: span hidden=\"2\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(
            message.contains("masking") && message.contains("does not parse"),
            "{message}"
        );
    }

    #[test]
    fn a_math_span_pinned_to_typing_a_command_is_linted_untypable() {
        let deck = parse(&format!(
            "## q\n---\n$\\gamma$\n<!-- input: type -->\n<!-- blank: span hidden=\"\\\\gamma\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(
            vec![LintKind::UntypableSpan {
                answer: r"\gamma".to_string()
            }],
            deck.lints
                .iter()
                .map(|l| l.kind.clone())
                .collect::<Vec<_>>(),
            "keyboard pinned, command answer"
        );

        let contains = parse(&format!(
            "## q\n---\n$x \\leq y$\n<!-- input: type -->\n<!-- blank: span hidden=\"x \\\\leq y\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(
            1,
            contains
                .lints
                .iter()
                .filter(|l| matches!(l.kind, LintKind::UntypableSpan { .. }))
                .count(),
            "a command anywhere in the hidden text is untypable: {:?}",
            contains.lints
        );

        let drawn = parse(&format!(
            "## q\n---\n$\\gamma$\n<!-- blank: span hidden=\"\\\\gamma\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert!(
            drawn.lints.is_empty(),
            "unpinned math spans draw by default: {:?}",
            drawn.lints
        );
    }

    #[test]
    fn image_syntax_is_invisible_to_span_binding() {
        let error = err(&format!(
            "## q\n---\nprose here\n![lunate](lunate.png)\n<!-- blank: span hidden=\"lunate\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(
            message.contains("occurs 0 time(s)"),
            "the alt text and destination are not learner prose: {message}"
        );
    }

    #[test]
    fn the_mask_codepoints_are_rejected_in_authored_text_anywhere() {
        assert_eq!(ParseError::ReservedMarker(2), err("## q\na ⍰ b\n"));
        assert_eq!(ParseError::ReservedMarker(1), err("## q ⬚\nanswer\n"));
    }

    #[test]
    fn literal_underscores_and_bracket_dots_are_ordinary_prose_beside_spans() {
        let deck = parse(&format!(
            "## q\n---\nfill the ____ gap in prose\n<!-- blank: span hidden=\"prose\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(
            vec!["fill the ____ gap in ⍰"],
            deck.cards[0].context,
            "an authored marker lookalike is text; only the reserved codepoint masks"
        );
    }

    #[test]
    fn an_image_sharing_a_line_with_prose_is_rejected_on_either_side() {
        let back = err("## q\nthe parts ![x](hand.png) are labeled\n");
        assert_eq!(ParseError::MixedImageLine(2), back);

        let front = err("## q\npress ![gear](gear.png) now\n\n---\nanswer\n");
        assert_eq!(ParseError::MixedImageLine(2), front);

        let indented = parse("## q\n  ![x](hand.png)\nanswer\n");
        assert_eq!(
            1,
            indented.cards[0].images_back.len(),
            "whitespace around an own-line image is not prose"
        );
    }

    #[test]
    fn an_image_sharing_the_card_heading_with_prose_is_rejected() {
        assert_eq!(
            ParseError::MixedImageLine(1),
            err("## identify ![car](car.png)\nanswer\n")
        );
    }

    #[test]
    fn a_malformed_image_in_the_card_heading_is_linted() {
        let deck = parse("## identify ![car](car.png\nanswer\n");
        assert_eq!(
            vec![Lint {
                line: 1,
                kind: LintKind::ImageMalformed,
            }],
            deck.lints
        );
    }

    #[test]
    fn occurrence_counting_skips_link_destinations() {
        let beside_link = err(&format!(
            "## q\n---\nsee [the RFC](https://rfc.example/22) for port 22\n<!-- blank: span hidden=\"22\" occurrence=2 b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = beside_link else {
            panic!("expected InvalidRegion, got {beside_link:?}");
        };
        assert!(
            message.contains("occurs 1 time(s)"),
            "the destination is invisible, so only the visible 22 counts: {message}"
        );
    }

    #[test]
    fn prose_beside_links_stays_blankable() {
        let beside_cover = parse(&format!(
            "## ports\n---\nSSH is 22; HTTPS is 22/tcp\n<!-- cover: span hidden=\"22\" -->\n<!-- id: {RTOK} -->\n"
        ));
        assert!(
            beside_cover.cards.iter().all(|card| card.region.is_none()),
            "a cover makes no card; the plain card stands"
        );

        let beside_link = parse(&format!(
            "## q\n---\nsee [the RFC](https://x) for port 22\n<!-- blank: span hidden=\"port\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(vec!["port"], beside_link.cards[0].back);

        let label = parse(&format!(
            "## q\n---\nsee [the RFC](https://x) for port 22\n<!-- blank: span hidden=\"RFC\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(
            vec!["RFC"],
            label.cards[0].back,
            "the label is matchable text"
        );
    }

    #[test]
    fn a_match_crossing_a_link_label_edge_is_a_style_boundary_error() {
        let error = err(&format!(
            "## q\n---\nsee [the RFC](https://x) now\n<!-- blank: span hidden=\"see the\" boundary=char b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("style boundary"), "{message}");
    }

    #[test]
    fn a_span_cards_context_masks_own_as_blank_and_siblings_as_hidden() {
        let deck = parse(&format!(
            "## anatomy\n---\nthe lunate sits beside the hamate bone\n<!-- blank: span hidden=\"lunate\" b:a1b2c3 -->\n<!-- blank: span hidden=\"hamate\" b:d4e5f6 -->\n<!-- cover: span hidden=\"bone\" -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(2, deck.cards.len());
        assert_eq!(
            vec!["the ⍰ sits beside the ⬚ ⬚"],
            deck.cards[0].context,
            "own blank asked, sibling and cover hidden"
        );
        assert!(deck.cards[0].context_leads);
        assert_eq!(
            vec!["the ⬚ sits beside the ⍰ ⬚"],
            deck.cards[1].context,
            "each card asks its own span"
        );
    }

    #[test]
    fn a_rect_cards_context_masks_every_span_as_hidden() {
        let deck = parse(&format!(
            "## anatomy\n![](hand.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 b:a1b2c3 -->\n\n---\nthe lunate sits center\n<!-- blank: span hidden=\"lunate\" b:d4e5f6 -->\n<!-- id: {RTOK} -->\n"
        ));
        let rect = deck
            .cards
            .iter()
            .find(|card| card.back.is_empty())
            .expect("the unlabelled rect card");
        assert_eq!(
            vec!["the ⬚ sits center"],
            rect.context,
            "prose is context on a rect card: every span is hidden"
        );
    }

    #[test]
    fn masking_splices_authored_bytes_so_styling_survives() {
        let deck = parse(&format!(
            "## q\n---\n**New** York is big\n<!-- blank: span hidden=\"New\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(
            vec!["**⍰** York is big"],
            deck.cards[0].context,
            "the splice lands inside the markers, so the bold survives"
        );
    }

    #[test]
    fn a_group_addressed_note_replaces_the_shared_note_on_its_card() {
        let deck = parse(&format!(
            "## q\n---\nalpha then beta\n> [!NOTE]\n> shared note\n> g: only for g\n<!-- blank: span [g] hidden=\"alpha\" b:a1b2c3 -->\n<!-- blank: span [h] hidden=\"beta\" b:d4e5f6 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(2, deck.cards.len(), "two one-member group cards");
        let g = deck
            .cards
            .iter()
            .find(|card| card.back == ["alpha"])
            .expect("group g's card");
        let h = deck
            .cards
            .iter()
            .find(|card| card.back == ["beta"])
            .expect("group h's card");
        assert_eq!(
            vec!["only for g"],
            g.notes.iter().map(|n| n.body.as_str()).collect::<Vec<_>>(),
            "the address REPLACES the shared note for g"
        );
        assert_eq!(
            vec!["shared note"],
            h.notes.iter().map(|n| n.body.as_str()).collect::<Vec<_>>(),
            "the unaddressed card keeps only the shared block"
        );
    }

    #[test]
    fn an_append_address_extends_the_shared_note_on_its_card() {
        let deck = parse(&format!(
            "## q\n---\nalpha then beta\n> [!NOTE]\n> shared note\n> g+: extra for g\n<!-- blank: span [g] hidden=\"alpha\" b:a1b2c3 -->\n<!-- blank: span [h] hidden=\"beta\" b:d4e5f6 -->\n<!-- id: {RTOK} -->\n"
        ));
        let g = deck
            .cards
            .iter()
            .find(|card| card.back == ["alpha"])
            .expect("group g's card");
        assert_eq!(
            vec!["shared note\nextra for g"],
            g.notes.iter().map(|n| n.body.as_str()).collect::<Vec<_>>(),
            "the append address extends the shared block for g"
        );
        let h = deck
            .cards
            .iter()
            .find(|card| card.back == ["beta"])
            .expect("group h's card");
        assert_eq!(
            vec!["shared note"],
            h.notes.iter().map(|n| n.body.as_str()).collect::<Vec<_>>(),
            "the unaddressed card keeps only the shared block"
        );
    }

    #[test]
    fn a_group_card_blanks_every_member_in_context() {
        let deck = parse(&format!(
            "## q\n---\nalpha then beta\n<!-- blank: span [pair] hidden=\"alpha\" b:a1b2c3 -->\n<!-- blank: span [pair] hidden=\"beta\" b:d4e5f6 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(1, deck.cards.len(), "one group card");
        assert_eq!(vec!["⍰ then ⍰"], deck.cards[0].context);
    }

    #[test]
    fn moving_a_span_to_another_occurrence_changes_the_fingerprint() {
        let one = parse(&format!(
            "## q\n---\nport 22 forwards to port 22\n<!-- blank: span hidden=\"22\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let two = parse(&format!(
            "## q\n---\nport 22 forwards to port 22\n<!-- blank: span hidden=\"22\" occurrence=2 b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_ne!(
            one.cards[0].content_fingerprint, two.cards[0].content_fingerprint,
            "the same word in a different sentence position is a different question"
        );
        assert_ne!(
            one.cards[0].context, two.cards[0].context,
            "the mask sits on a different occurrence"
        );
    }

    #[test]
    fn image_only_lines_stay_out_of_region_context() {
        let deck = parse(&format!(
            "## q\n---\nthe lunate sits center\n![](hand.png)\n<!-- blank: span hidden=\"lunate\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(
            vec!["the ⍰ sits center"],
            deck.cards[0].context,
            "the image rides images_back, never the prose context"
        );
        assert_eq!(1, deck.cards[0].images_back.len());
    }

    #[test]
    fn overlapping_span_ranges_are_rejected_before_masking() {
        let error = err(&format!(
            "## q\n---\nNew York City Hall\n<!-- blank: span hidden=\"New York City Hall\" boundary=char b:a1b2c3 -->\n<!-- blank: span hidden=\"York City Hall\" boundary=char b:d4e5f6 -->\n<!-- id: {RTOK} -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("overlap"), "{message}");
    }

    #[test]
    fn a_cover_span_masks_answer_giving_prose_in_blank_context() {
        let deck = parse(&format!(
            "## q\n---\nthe legend says alpha; fill alpha\n<!-- cover: span hidden=\"alpha\" -->\n<!-- blank: span hidden=\"alpha\" occurrence=2 b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_eq!(1, deck.cards.len());
        assert!(deck.cards[0].is_blank_card());
        assert_eq!(vec!["the legend says ⬚; fill ⍰"], deck.cards[0].context);
    }

    #[test]
    fn moving_a_cover_span_changes_the_blank_cards_fingerprint() {
        let first = parse(&format!(
            "## q\n---\nalpha then alpha; fill x\n<!-- cover: span hidden=\"alpha\" -->\n<!-- blank: span hidden=\"x\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        let second = parse(&format!(
            "## q\n---\nalpha then alpha; fill x\n<!-- cover: span hidden=\"alpha\" occurrence=2 -->\n<!-- blank: span hidden=\"x\" b:a1b2c3 -->\n<!-- id: {RTOK} -->\n"
        ));
        assert_ne!(
            first.cards[0].content_fingerprint,
            second.cards[0].content_fingerprint
        );
    }

    #[test]
    fn each_blank_card_fingerprints_its_effective_masked_question() {
        let deck = parse(
            "## q\n---\nfirst alpha; second beta\n<!-- blank: span hidden=\"alpha\" -->\n<!-- blank: span hidden=\"beta\" -->\n",
        );
        assert_eq!(2, deck.cards.len());
        for card in &deck.cards {
            let mut effective_question = card.context.clone();
            effective_question.extend(card.back.iter().cloned());
            assert_eq!(
                content_fingerprint(&card.front, &effective_question),
                card.content_fingerprint,
                "card back {:?} must hash front, its masked context, and its answer",
                card.back
            );
        }
    }

    #[test]
    fn editing_a_hidden_sibling_preserves_the_unchanged_blank_cards_fingerprint() {
        let before = parse(
            "## q\n---\nfirst alpha; second beta\n<!-- blank: span hidden=\"alpha\" -->\n<!-- blank: span hidden=\"beta\" -->\n",
        );
        let after = parse(
            "## q\n---\nfirst alpha; second gamma\n<!-- blank: span hidden=\"alpha\" -->\n<!-- blank: span hidden=\"gamma\" -->\n",
        );
        let alpha_before = before
            .cards
            .iter()
            .find(|card| card.back == ["alpha"])
            .unwrap();
        let alpha_after = after
            .cards
            .iter()
            .find(|card| card.back == ["alpha"])
            .unwrap();
        assert_eq!(alpha_before.context, alpha_after.context);
        assert_eq!(alpha_before.back, alpha_after.back);
        assert_eq!(
            alpha_before.content_fingerprint, alpha_after.content_fingerprint,
            "a hidden sibling edit did not change this card's effective question"
        );
    }

    #[test]
    fn removing_the_last_blank_directive_re_exposes_the_plain_card_with_its_token() {
        let template = parse(&region_deck(
            r#"<!-- blank: rect x=1 y=1 width=2 height=2 hidden="lunate" b:a1b2c3 -->"#,
        ));
        assert_eq!(Some(format!("{RTOK}-ba1b2c3")), template.cards[0].id());

        let plain = parse(&region_deck(""));
        assert_eq!(1, plain.cards.len());
        assert!(plain.cards[0].region.is_none());
        assert_eq!(
            Some(RTOK.to_string()),
            plain.cards[0].id(),
            "the block token is the plain card's identity, so its history resumes"
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig { cases: 64, ..proptest::prelude::ProptestConfig::default() })]

        #[test]
        fn the_parser_never_panics_on_arbitrary_block_text(
            lines in proptest::collection::vec(
                proptest::string::string_regex(
                    "[a-zA-Zäö✓ *_`$\\\\\\[\\]()!{}#>|.0-9-]{0,24}"
                )
                .expect("a valid regex literal"),
                1..5,
            ),
            spans in proptest::collection::vec(
                (
                    proptest::sample::select(vec!["a", "b", "ab", "ba"]),
                    1..3u32,
                    proptest::bool::ANY,
                ),
                0..3,
            ),
        ) {
            let mut deck = String::from("## q <!-- id: card-proptok -->\n---\n");
            for line in &lines {
                deck.push_str(line);
                deck.push('\n');
            }
            for (hidden, occurrence, cover) in &spans {
                let keyword = if *cover { "cover" } else { "blank" };
                deck.push_str(&format!(
                    "<!-- {keyword}: span hidden=\"{hidden}\" occurrence={occurrence} boundary=char -->\n"
                ));
            }
            // Ok or Err are both fine; only a panic fails the property.
            let _ = super::parse("deck.md", &deck);
        }
    }
    #[test]
    fn a_diagram_stamp_parses_onto_the_card() {
        let object = |ext: &str| format!("sha256-{}.{ext}", "a".repeat(64));
        let text = format!(
            "## q\n```mermaid\nflowchart LR\n A-->B\n```\n<!-- diagram: fingerprint: xxh64-0011223344556677 asset: {} geometry: {} -->\nanswer\n",
            object("png"),
            object("json"),
        );
        let deck = parse(&text);
        assert_eq!(1, deck.cards[0].diagrams.len());
        let stamp = &deck.cards[0].diagrams[0];
        assert_eq!("xxh64-0011223344556677", stamp.fingerprint);
        assert_eq!(object("png"), stamp.asset);
        assert_eq!(object("json"), stamp.geometry);
        assert!(
            !deck.cards[0]
                .back
                .iter()
                .any(|line| line.contains("diagram:")),
            "the stamp is a directive, never answer content"
        );
    }

    #[test]
    fn malformed_diagram_stamps_fail_as_invalid_input() {
        let png = format!("sha256-{}.png", "a".repeat(64));
        let json = format!("sha256-{}.json", "a".repeat(64));
        let cases = [
            (
                "fingerprint: xxh64-00112233 asset: a geometry: b".to_string(),
                "short fingerprint hex",
            ),
            (
                format!("fingerprint: 0011223344556677 asset: {png} geometry: {json}"),
                "missing xxh64 prefix",
            ),
            (
                format!("fingerprint: xxh64-0011223344556677 asset: nope.png geometry: {json}"),
                "asset is not an object name",
            ),
            (
                format!("fingerprint: xxh64-0011223344556677 asset: {json} geometry: {png}"),
                "swapped artifact roles: a json asset or png geometry is a later decode failure",
            ),
            (
                format!("fingerprint: xxh64-0011223344556677 asset: {png} geometry: {png}"),
                "the geometry role requires a json object",
            ),
            (
                format!("fingerprint: xxh64-0011223344556677 geometry: {json} asset: {png}"),
                "fields out of order",
            ),
            (
                format!(
                    "fingerprint: xxh64-0011223344556677 asset: {png} geometry: {json} extra: 1"
                ),
                "trailing junk",
            ),
        ];
        for (stamp, why) in cases {
            let text = format!("## q\na\n<!-- diagram: {stamp} -->\n");
            assert!(
                super::parse("deck.md", &text).is_err(),
                "{why}: `{stamp}` must be rejected as invalid input"
            );
        }
    }
}
