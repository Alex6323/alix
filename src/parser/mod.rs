use std::{collections::HashSet, ops::Range, path::PathBuf, sync::Arc};

use thiserror::Error;

use crate::{
    answer::Input,
    card::{Card, CardImage, Direction},
    depth::Reveal,
    token,
};

mod canonical;
pub(crate) mod checklist;
mod cloze;
mod frontmatter;
mod mathspan;
pub mod region;
mod sidecar;
mod stream;

pub use canonical::{canonical_content, content_fingerprint};
pub use cloze::{BLANK, HIDDEN};
use cloze::{Hole, Seg, Side, hash_repr, hole_fingerprints, scan_markers, seg_display};
pub use frontmatter::{
    DECK_FORMAT_VERSION, Frontmatter, PERSONAL_PARENT_KEY, parse_sampling, yaml_quote,
};
use frontmatter::{bad_value, closes_frontmatter, parse_frontmatter, parse_reveal};
pub use sidecar::{SidecarNote, notes, without_notes};

// Deliberately not Unicode whitespace; anything outside this set is content.
const WHITESPACE: [char; 6] = ['\t', '\n', '\x0B', '\x0C', '\r', ' '];

const ESCAPABLE: [&str; 6] = ["##", ">", "---", "<!--", "```", "~~~"];

pub type LineSpan = (usize, usize);

#[derive(Debug)]
pub struct ParsedDeck {
    pub deck_token: Option<String>,
    pub title: Option<String>,
    pub preamble: Option<String>,
    pub frontmatter: Frontmatter,
    pub cards: Vec<Card>,
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
    RevealOnCloze,
    IndentedH2,
    ClozeInHole,
    UnclosedComment,
    UnclosedFence,
    ImageMalformed,
    ChoiceAnswerMixed,
    ChoiceNeedsBothSides,
    DuplicateChoiceOption,
    ChoiceMultiCorrectUnsupported,
    UntypableHole {
        answer: String,
    },
    /// A block note that spells out one hole's answer, which every other
    /// hole of the block also shows. `hole` is 1-based, as an author counts.
    NoteContainsHoleAnswer {
        hole: usize,
        answer: String,
    },
    NoteNamesNoHole {
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
    #[error("line {0}: an initialized deck must declare `format-version: 1`")]
    MissingDeckVersion(usize),
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
    #[error("line {0}: card front is empty")]
    EmptyFront(usize),
    #[error("line {0}: card front without an answer")]
    FrontWithoutAnswer(usize),
    #[error(
        "line {line}: `at:` is not a named-field locator (`at: <src>:<lines> fingerprint: xxh64-<hex> asset: <object>`): {message}; fields are `at:`, `fingerprint:`, `asset:`, in that order"
    )]
    InvalidLocator { line: usize, message: String },
    #[error("line {line}: {message}")]
    InvalidRegion { line: usize, message: String },
    #[error(
        "line {0}: a hole name is one or more of `a-z`, `A-Z`, `0-9`, `_` or `-`, closed by `]` and followed by `{{answer}}`: `\\blank[base]{{Unit}}`"
    )]
    InvalidHoleName(usize),
    #[error("line {0}: unclosed cloze hole (missing the closing `}}`)")]
    UnclosedHole(usize),
    #[error("line {0}: empty cloze hole")]
    EmptyHole(usize),
    #[error(
        "line {0}: an image shares its line with prose; give the image its own line (inline images are a roadmap item, not silently torn from the sentence)"
    )]
    MixedImageLine(usize),
    #[error("line {0}: a table line must start and end with `|`")]
    TableLineMalformed(usize),
    #[error(
        "line {line}: a card table has 2 or 3 columns (front | back | note), this line has {found}"
    )]
    TableColumns { line: usize, found: usize },
    #[error("line {line}: this table line has {found} cells but the header has {expected}")]
    TableRowWidth {
        line: usize,
        found: usize,
        expected: usize,
    },
    #[error(
        "line {0}: `\\blank{{...}}` in a table cell is not supported; write that row as a `##` card"
    )]
    TableCellHole(usize),
    #[error("line {0}: an image in a table cell is not supported; write that row as a `##` card")]
    TableCellImage(usize),
    #[error("line {line}: row stamp `{value}` is not 6 base32 chars")]
    TableRowStamp { line: usize, value: String },
    #[error("line {line}: row stamp `{value}` appears twice in one table")]
    TableDuplicateStamp { line: usize, value: String },
    #[error("line {0}: only directive comments may follow a card table before the next `## ` card")]
    TableTrailing(usize),
}

impl ParseError {
    pub(crate) fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::UnclosedFrontmatter(_) => "unclosed_frontmatter",
            Self::FrontmatterSyntax { .. } => "frontmatter_syntax",
            Self::NonStringId { .. } => "non_string_id",
            Self::InvalidDeckId { .. } => "invalid_deck_id",
            Self::InvalidCardId { .. } => "invalid_card_id",
            Self::MissingDeckVersion(_) => "missing_deck_version",
            Self::UnsupportedDeckVersion { .. } => "unsupported_deck_version",
            Self::NonIntegerVersion { .. } => "non_integer_version",
            Self::ControlChar { .. } => "control_character",
            Self::ReservedMarker(_) => "reserved_marker",
            Self::EmptyFront(_) => "empty_front",
            Self::FrontWithoutAnswer(_) => "front_without_answer",
            Self::InvalidLocator { .. } => "invalid_locator",
            Self::InvalidRegion { .. } => "invalid_region",
            Self::InvalidHoleName(_) => "invalid_hole_name",
            Self::UnclosedHole(_) => "unclosed_hole",
            Self::EmptyHole(_) => "empty_hole",
            Self::MixedImageLine(_) => "mixed_image_line",
            Self::TableLineMalformed(_) => "table_line_malformed",
            Self::TableColumns { .. } => "table_columns",
            Self::TableRowWidth { .. } => "table_row_width",
            Self::TableCellHole(_) => "table_cell_hole",
            Self::TableCellImage(_) => "table_cell_image",
            Self::TableRowStamp { .. } => "table_row_stamp",
            Self::TableDuplicateStamp { .. } => "table_duplicate_stamp",
            Self::TableTrailing(_) => "table_trailing",
        }
    }

    pub(crate) fn line(&self) -> usize {
        match self {
            Self::UnclosedFrontmatter(line)
            | Self::MissingDeckVersion(line)
            | Self::EmptyFront(line)
            | Self::FrontWithoutAnswer(line)
            | Self::InvalidHoleName(line)
            | Self::UnclosedHole(line)
            | Self::EmptyHole(line)
            | Self::MixedImageLine(line)
            | Self::ReservedMarker(line)
            | Self::TableLineMalformed(line)
            | Self::TableCellHole(line)
            | Self::TableCellImage(line)
            | Self::TableTrailing(line) => *line,
            Self::FrontmatterSyntax { line, .. }
            | Self::NonStringId { line, .. }
            | Self::InvalidDeckId { line, .. }
            | Self::InvalidCardId { line, .. }
            | Self::UnsupportedDeckVersion { line, .. }
            | Self::NonIntegerVersion { line, .. }
            | Self::ControlChar { line, .. }
            | Self::InvalidLocator { line, .. }
            | Self::InvalidRegion { line, .. }
            | Self::TableColumns { line, .. }
            | Self::TableRowWidth { line, .. }
            | Self::TableRowStamp { line, .. }
            | Self::TableDuplicateStamp { line, .. } => *line,
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
                let prose = build_card(&subject, &deck_id, raw, &mut cards, &mut lints)?;
                build_region_cards(block_start, &mut cards, prose.as_ref())?;
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
    Ok(ParsedDeck {
        deck_token: document.frontmatter.id.clone(),
        title: document.title,
        preamble: document.preamble,
        frontmatter: document.frontmatter,
        cards,
        lints,
        frontmatter_span: document.frontmatter_span,
        tables,
    })
}

pub fn parse_str(subject: &str, text: &str) -> Result<Vec<Card>, ParseError> {
    Ok(parse(subject, text)?.cards)
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

pub fn is_deck_content(text: &str) -> bool {
    match parse("deck.md", text) {
        Ok(deck) => !deck.cards.is_empty() || deck.frontmatter_span.is_some(),
        // A parse failure counts as deck content too: a broken deck should
        // surface to doctor rather than silently vanish from the listing.
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
            if line == "---" {
                frontmatter = true;
                offset += segment.len();
                continue;
            }
        }
        if frontmatter {
            if closes_frontmatter(line) {
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
    title: Option<String>,
    preamble: Option<String>,
    blocks: Vec<RawBlock>,
    lints: Vec<Lint>,
    frontmatter_span: Option<LineSpan>,
}

enum RawBlock {
    Card(RawCard),
    Table(RawTable),
}

struct RawCard {
    line: usize,
    front: String,
    front_extra: Vec<(usize, String)>,
    back: Vec<(usize, String)>,
    divided: bool,
    /// The `---` line's number when divided: the side boundary for region
    /// binding, which first-answer-content would misplace for a directive
    /// sitting between the divider and the first content line.
    divider_line: Option<usize>,
    note: Option<String>,
    directives: CardDirectives,
}

struct RawTable {
    line: usize,
    title: Option<String>,
    columns: usize,
    rows: Vec<RawRow>,
    directives: CardDirectives,
    rows_done: bool,
    end_line: usize,
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
    givens: Vec<String>,
}

fn parse_document(text: &str) -> Result<Document, ParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = prepare(text)?;
    let mut lints = Vec::new();
    let (frontmatter, body_start, frontmatter_span) = parse_frontmatter(&lines, &mut lints)?;
    let (title, preamble, blocks) = scan(&lines, body_start, &mut lints)?;
    Ok(Document {
        frontmatter,
        title,
        preamble,
        blocks,
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

fn collapse(s: &str) -> String {
    s.split(&WHITESPACE[..])
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn fence_opener(line: &str) -> Option<char> {
    if line.starts_with("```") {
        Some('`')
    } else if line.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

pub(crate) fn closes_fence(line: &str, ch: char) -> bool {
    let run = line.chars().take_while(|c| *c == ch).count();
    run >= 3 && line.chars().skip(run).all(|c| WHITESPACE.contains(&c))
}

// ── The line scanner ──

// `(title, preamble, blocks)` from the body above the first card.
type ScannedBody = (Option<String>, Option<String>, Vec<RawBlock>);

fn scan(lines: &[&str], start: usize, lints: &mut Vec<Lint>) -> Result<ScannedBody, ParseError> {
    let mut title: Option<String> = None;
    let mut preamble_lines: Vec<String> = Vec::new();
    let mut blocks: Vec<RawBlock> = Vec::new();
    let mut current: Option<RawCard> = None;
    let mut table: Option<RawTable> = None;
    let mut skip_delimiter = false;
    let mut fence: Option<(char, usize)> = None;
    let mut prev_blank = false;
    let mut prev_heading = false;

    for (idx, raw) in lines.iter().enumerate().skip(start) {
        let lineno = idx + 1;
        let raw = *raw;

        if skip_delimiter {
            skip_delimiter = false;
            continue;
        }

        // A fence never opens while a table is active (every non-table line
        // inside a table's scope is either a flush or a loud error).
        if let Some(tbl) = table.as_mut() {
            let next = lines.get(idx + 1).copied();
            if table_line(tbl, raw, lineno, next, lints)? {
                prev_blank = trim_ws(raw).is_empty();
                prev_heading = false;
                continue;
            }
            if let Some(tbl) = table.take() {
                blocks.push(RawBlock::Table(tbl));
            }
        }

        if let Some((ch, _)) = fence {
            if closes_fence(raw, ch) {
                fence = None;
            }
            push_content(&mut current, lineno, raw.to_string());
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        if let Some(ch) = fence_opener(raw) {
            fence = Some((ch, lineno));
            push_content(&mut current, lineno, raw.to_string());
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        if raw.starts_with('|')
            && let Some(next) = lines.get(idx + 1)
            && next.starts_with('|')
            && is_delimiter_row(next)
        {
            // An empty-bodied heading directly above the table is its TITLE,
            // not a card; any content or note keeps it a card.
            let mut title = None;
            let mut block_line = lineno;
            let mut directives = CardDirectives::default();
            if let Some(card) = current.take() {
                if card.front_extra.is_empty()
                    && card.back.is_empty()
                    && card.note.is_none()
                    && !card.divided
                {
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
                return Err(ParseError::TableRowWidth {
                    line: lineno + 1,
                    found: delimiter.len(),
                    expected: header.len(),
                });
            }
            table = Some(RawTable {
                line: block_line,
                title,
                columns: header.len(),
                rows: Vec::new(),
                directives,
                rows_done: false,
                end_line: lineno + 1,
            });
            skip_delimiter = true;
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        if let Some(rest) = raw.strip_prefix("## ") {
            if let Some(card) = current.take() {
                blocks.push(RawBlock::Card(card));
            }
            let (front, directives) = heading(rest, lineno, lints)?;
            if front.is_empty() {
                return Err(ParseError::EmptyFront(lineno));
            }
            current = Some(RawCard {
                line: lineno,
                front,
                front_extra: Vec::new(),
                back: Vec::new(),
                divided: false,
                divider_line: None,
                note: None,
                directives,
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

        if let Some(rest) = t.strip_prefix('\\')
            && ESCAPABLE.iter().any(|marker| rest.starts_with(marker))
        {
            push_content(&mut current, lineno, rest.to_string());
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        if t == "---" {
            let divides =
                current.as_ref().is_some_and(|card| !card.divided) && (prev_blank || prev_heading);
            if divides && let Some(card) = current.as_mut() {
                card.divided = true;
                card.divider_line = Some(lineno);
            } else {
                push_content(&mut current, lineno, "---".to_string());
            }
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        if let Some(rest) = t.strip_prefix('>') {
            if let Some(card) = current.as_mut() {
                let text = rest.strip_prefix(' ').unwrap_or(rest);
                append_note(card, text);
            }
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        if t.starts_with("<!--") {
            if let Some(body) = t.strip_prefix("<!--").and_then(|s| s.strip_suffix("-->")) {
                if let Some((key, value)) = directive(body) {
                    match current.as_mut() {
                        Some(card) => {
                            apply_directive(&mut card.directives, &key, value, lineno, lints)?;
                        }
                        // A region outside any card has nothing to bind to and
                        // would otherwise vanish silently; other directives
                        // keep their historical tolerance here.
                        None if matches!(key.as_str(), "blank" | "cover" | "crop") => {
                            return Err(ParseError::InvalidRegion {
                                line: lineno,
                                message: format!(
                                    "`{key}:` appears before any card, so no media element or answer block can bind it"
                                ),
                            });
                        }
                        None => {}
                    }
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

        if current.is_none() {
            if title.is_none()
                && let Some(rest) = raw.strip_prefix("# ")
            {
                title = Some(strip_trailing_hashes(trim_ws(rest)).to_string());
            } else {
                preamble_lines.push(t.to_string());
            }
            prev_blank = false;
            prev_heading = false;
            continue;
        }

        push_content(&mut current, lineno, t.to_string());
        prev_blank = false;
        prev_heading = false;
    }

    if let Some((_, open_line)) = fence {
        lints.push(Lint {
            line: open_line,
            kind: LintKind::UnclosedFence,
        });
    }
    if let Some(tbl) = table.take() {
        blocks.push(RawBlock::Table(tbl));
    }
    if let Some(card) = current.take() {
        blocks.push(RawBlock::Card(card));
    }
    let preamble = (!preamble_lines.is_empty()).then(|| preamble_lines.join(" "));
    Ok((title, preamble, blocks))
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
    if raw.strip_prefix("## ").is_some() {
        return Ok(false);
    }
    if let Some(body) = t.strip_prefix("<!--").and_then(|s| s.strip_suffix("-->")) {
        tbl.rows_done = true;
        tbl.end_line = lineno;
        if let Some((key, value)) = directive(body) {
            apply_directive(&mut tbl.directives, &key, value, lineno, lints)?;
        }
        return Ok(true);
    }
    Err(ParseError::TableTrailing(lineno))
}

fn split_cells(line: &str) -> Option<Vec<String>> {
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

fn check_cells(cells: &[String], lineno: usize) -> Result<(), ParseError> {
    for cell in cells {
        if cell.contains("\\blank{") || cell.contains("\\blank[") {
            return Err(ParseError::TableCellHole(lineno));
        }
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

/// Synthesizes the region cards a block's blanks ask (ADR 0034): a named
/// group is one card asking every member, an ungrouped blank one card each,
/// a cover no card. A blank-bearing block is a template: its region cards
/// REPLACE the cards `build_card` pushed, so no plain card exists beside
/// them; cover/crop-only blocks keep theirs.
fn build_region_cards(
    block_start: usize,
    cards: &mut Vec<Card>,
    prose: Option<&BlockProse>,
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
    let region_card = |slot: RegionSlot, back: Vec<String>| {
        let mut card = template.clone();
        card.back = back;
        card.display_back = None;
        // card.line stays the authored block line: card_front_lines exposes
        // every distinct line as a Markdown block boundary, and a directive
        // line there once let removal truncate the parent's answer.
        let own: Vec<usize> = match &slot {
            RegionSlot::Single { line, .. } => vec![*line],
            RegionSlot::Group { members, .. } => members.iter().map(|m| m.line).collect(),
        };
        card.context = masked_context(&own);
        card.context_leads = !card.context.is_empty();
        card.region = Some(slot);
        card.hole = None;
        card.hole_name = None;
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
        // The all-or-none rule for a group's answers, exactly as text holes
        // have it: mixed presence would leave the card half-answerable.
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
        let mut card = Card::plain(Arc::clone(subject), front, vec![back], note, row.line);
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
        card.givens = raw.directives.givens.clone();
        cards.push(card);
    }
    Ok(())
}

fn push_content(current: &mut Option<RawCard>, lineno: usize, text: String) {
    if let Some(card) = current.as_mut() {
        if card.divided {
            card.back.push((lineno, text));
        } else {
            card.front_extra.push((lineno, text));
        }
    }
}

fn append_note(card: &mut RawCard, text: &str) {
    match &mut card.note {
        Some(note) => {
            note.push('\n');
            note.push_str(text);
        }
        slot => *slot = Some(text.to_string()),
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
            apply_directive(&mut directives, &key, value, lineno, lints)?;
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
                Some((_, None, None, false, None))
            ) {
                return Err(ParseError::InvalidCardId { line, value });
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
        Seg::Text(_) | Seg::Hole { .. } => None,
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
            Seg::Hole { .. } => true,
            Seg::Text(text) => !text.trim().is_empty(),
        })
}

fn build_card(
    subject: &Arc<str>,
    deck_id: &Arc<str>,
    raw: RawCard,
    cards: &mut Vec<Card>,
    lints: &mut Vec<Lint>,
) -> Result<Option<BlockProse>, ParseError> {
    let RawCard {
        line,
        front: heading,
        front_extra,
        back,
        divided,
        divider_line,
        note,
        directives,
    } = raw;
    let mut front_media: Vec<(usize, CardImage)> = Vec::new();
    {
        let segments = scan_markers(&heading, line, Side::Front, lints)?;
        if segments.iter().any(|s| matches!(s, Seg::Image { .. })) {
            return Err(ParseError::MixedImageLine(line));
        }
    }
    let (front, answer) = if divided {
        let mut front_lines = vec![heading];
        for (lineno, text) in &front_extra {
            let segments = scan_markers(text, *lineno, Side::Front, lints)?;
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
        let segments = scan_markers(text, *lineno, Side::Answer, lints)?;
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
    let span_regions = bind_regions(
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
    if !span_regions.is_empty() {
        let stream = stream::maskable_stream(&answer, &parsed);
        for span in &span_regions {
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
                let accepted = unit && (!whole_word || stream.word_bounded(&range));
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
                        kind: LintKind::UntypableHole {
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
                answer_index,
                range: (range.start, range.end),
            });
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
        if let Some(ch) = fence {
            if closes_fence(text, ch) {
                fence = None;
            }
            has_other = true;
            continue;
        }
        if let Some(ch) = fence_opener(text) {
            fence = Some(ch);
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
    if !task_lines.is_empty()
        && let Some(line) = first_blank_line
    {
        return Err(ParseError::InvalidRegion {
            line,
            message: "a `blank:` region cannot share a block with a task-list answer".into(),
        });
    }
    if !task_lines.is_empty() && has_other {
        lints.push(Lint {
            line: task_lines[0].0,
            kind: LintKind::ChoiceAnswerMixed,
        });
    } else if !task_lines.is_empty() {
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
        if checked_count > 1 {
            lints.push(Lint {
                line: choice_line,
                kind: LintKind::ChoiceMultiCorrectUnsupported,
            });
        } else {
            let distractors: Vec<String> = options
                .iter()
                .filter(|(checked, _)| !checked)
                .map(|(_, text)| text.clone())
                .collect();
            if checked_count == 0 || distractors.is_empty() {
                lints.push(Lint {
                    line: choice_line,
                    kind: LintKind::ChoiceNeedsBothSides,
                });
            } else if let Some((_, correct)) = options.into_iter().find(|(checked, _)| *checked) {
                let mut card = Card::plain(Arc::clone(subject), front, vec![correct], note, line);
                card.deck_id = Arc::clone(deck_id);
                card.token = directives.token.as_deref().map(Arc::from);
                card.images = images;
                card.images_back = images_back;
                card.span_regions = span_regions;
                card.citations = directives.citations;
                card.givens = directives.givens;
                card.authored_distractors = distractors;
                cards.push(card);
                return Ok(None);
            }
        }
    }

    /// Whether the whole answer is one LaTeX command: `\pm` yes, `2a` and
    /// `x^2` no, and `\frac{1}{2}` yes, since none of them can be typed as
    /// what they render to.
    fn is_control_sequence(answer: &str) -> bool {
        answer.trim_start().starts_with('\\')
    }

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

    /// A `>` line addressed to one hole of this block: `name: text` replaces
    /// the block note for it, `name+: text` keeps the block note above it.
    fn address(text: &str) -> Option<(&str, bool, &str)> {
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
        cloze::is_hole_name(name).then_some((name, append, payload))
    }

    fn split_note(
        note: Option<&str>,
        names: &HashSet<&str>,
        line: usize,
        lints: &mut Vec<Lint>,
    ) -> (Option<String>, Vec<(String, bool, String)>) {
        let Some(note) = note else {
            return (None, Vec::new());
        };
        let mut block: Vec<&str> = Vec::new();
        let mut addressed = Vec::new();
        for text in note.lines() {
            match address(text) {
                Some((name, append, payload)) if names.contains(name) => {
                    addressed.push((name.to_string(), append, payload.to_string()));
                }
                // Only a card that names a hole can be addressing one, so a
                // note beginning `2:` on any other card is prose.
                Some((name, ..)) if !names.is_empty() => {
                    lints.push(Lint {
                        line,
                        kind: LintKind::NoteNamesNoHole {
                            name: name.to_string(),
                        },
                    });
                    block.push(text);
                }
                _ => block.push(text),
            }
        }
        ((!block.is_empty()).then(|| block.join("\n")), addressed)
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

    fn hole_sits_in_math(segments: &[Seg], hole_seg: usize) -> bool {
        let mut line = String::new();
        for (si, segment) in segments.iter().enumerate() {
            match segment {
                Seg::Text(text) => line.push_str(text),
                Seg::Hole { .. } if si == hole_seg => line.push_str(BLANK),
                Seg::Hole { .. } => line.push_str(HIDDEN),
                Seg::Image { .. } => {}
            }
        }
        crate::inline::math_encloses(&line, BLANK)
    }

    let holes: Vec<Hole<'_>> = parsed
        .iter()
        .enumerate()
        .flat_map(|(li, segments)| {
            segments
                .iter()
                .enumerate()
                .filter_map(move |(si, segment)| match segment {
                    Seg::Hole { text, name } => Some(Hole {
                        line: li,
                        seg: si,
                        text: text.as_str(),
                        name: name.as_deref(),
                    }),
                    Seg::Text(_) | Seg::Image { .. } => None,
                })
        })
        .collect();

    let mut named = HashSet::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (h, hole) in holes.iter().enumerate() {
        let joined = hole.name.and_then(|name| {
            groups
                .iter_mut()
                .find(|group| holes[group[0]].name == Some(name))
                .map(|group| group.push(h))
        });
        if joined.is_none() {
            groups.push(vec![h]);
        }
        named.extend(hole.name);
    }

    if holes.is_empty() {
        let back_lines: Vec<String> = parsed
            .iter()
            .filter(|segments| !image_only(segments))
            .map(|segments| seg_display(segments))
            .collect();
        let mut card = Card::plain(Arc::clone(subject), front, back_lines, note, line);
        card.deck_id = Arc::clone(deck_id);
        card.token = directives.token.as_deref().map(Arc::from);
        card.reveal = directives.reveal;
        card.input = directives.input;
        card.direction = directives.direction;
        card.sampling = directives.sampling;
        card.images = images;
        card.images_back = images_back;
        card.span_regions = span_regions;
        card.citations = directives.citations;
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
        return Ok(prose);
    }

    if let Some(line) = first_blank_line {
        return Err(ParseError::InvalidRegion {
            line,
            message: "a `blank:` region cannot share a block with `\\blank{}` text holes".into(),
        });
    }

    // A cloze card. `reveal:` is retired here: the holes are the trigger.
    if directives.reveal.is_some() {
        lints.push(Lint {
            line: directives.reveal_line.unwrap_or(line),
            kind: LintKind::RevealOnCloze,
        });
    }
    let (block_note, addressed) = split_note(note.as_deref(), &named, line, lints);
    let notes: Vec<Option<String>> = groups
        .iter()
        .map(|group| resolve_note(block_note.as_deref(), &addressed, holes[group[0]].name))
        .collect();
    for (n, group) in groups.iter().enumerate() {
        for hole in group.iter().map(|h| &holes[*h]) {
            let shown_elsewhere = notes.iter().enumerate().any(|(other, note)| {
                other != n
                    && note
                        .as_deref()
                        .is_some_and(|note| names_answer(note, hole.text))
            });
            if shown_elsewhere {
                lints.push(Lint {
                    line,
                    kind: LintKind::NoteContainsHoleAnswer {
                        hole: n + 1,
                        answer: hole.text.to_string(),
                    },
                });
            }
        }
    }
    let token: Option<Arc<str>> = directives.token.as_deref().map(Arc::from);
    let structure: Vec<String> = parsed.iter().map(|segments| hash_repr(segments)).collect();
    let block_holes = hole_fingerprints(&parsed, &holes, &groups);
    for (n, group) in groups.iter().enumerate() {
        let asked: Vec<&Hole<'_>> = group.iter().map(|h| &holes[*h]).collect();
        // Cover-free blocks keep the seg rendering verbatim (escapes display
        // unescaped), so no existing deck's context changes; a cover forces
        // the raw-splice path, where covers and hole footprints cut the
        // authored line together and the leak the cover exists for dies.
        let context: Vec<String> = if splices.is_empty() {
            parsed
                .iter()
                .enumerate()
                .filter(|(_, segments)| !image_only(segments))
                .map(|(li, segments)| {
                    let mut rendered = String::new();
                    for (si, segment) in segments.iter().enumerate() {
                        let asked_here = asked.iter().any(|hole| hole.line == li && hole.seg == si);
                        match segment {
                            Seg::Text(text) => rendered.push_str(text),
                            Seg::Hole { .. } if asked_here => rendered.push_str(BLANK),
                            Seg::Hole { .. } => rendered.push_str(HIDDEN),
                            Seg::Image { .. } => {}
                        }
                    }
                    rendered
                })
                .collect()
        } else {
            answer
                .iter()
                .zip(&parsed)
                .enumerate()
                .filter(|(_, (_, segments))| !image_only(segments))
                .map(|(li, ((_, raw), _))| {
                    let line_holes: Vec<&Hole<'_>> =
                        holes.iter().filter(|hole| hole.line == li).collect();
                    let mut cuts: Vec<(usize, usize, &str)> = splices
                        .iter()
                        .filter(|splice| splice.answer_index == li)
                        .map(|splice| (splice.range.0, splice.range.1, HIDDEN))
                        .collect();
                    for (footprint, hole) in cloze::hole_footprints(raw).into_iter().zip(line_holes)
                    {
                        let asked_here = asked
                            .iter()
                            .any(|it| it.line == hole.line && it.seg == hole.seg);
                        cuts.push((
                            footprint.start,
                            footprint.end,
                            if asked_here { BLANK } else { HIDDEN },
                        ));
                    }
                    cuts.sort_by_key(|(start, ..)| std::cmp::Reverse(*start));
                    let mut rendered = raw.clone();
                    for (start, end, marker) in cuts {
                        rendered.replace_range(start..end, marker);
                    }
                    rendered
                })
                .collect()
        };
        let mut hash_lines = structure.clone();
        hash_lines.push(format!("#cloze:{n}"));
        let mut card = Card::plain(
            Arc::clone(subject),
            front.clone(),
            asked.iter().map(|hole| hole.text.to_string()).collect(),
            notes[n].clone(),
            line,
        );
        card.deck_id = Arc::clone(deck_id);
        card.context = context;
        card.context_leads = true;
        card.hash_lines = Some(hash_lines);
        card.token = token.clone();
        card.hole = Some(n as u32);
        card.hole_name = asked[0].name.map(str::to_string);
        let in_math: Vec<bool> = asked
            .iter()
            .map(|hole| hole_sits_in_math(&parsed[hole.line], hole.seg))
            .collect();
        if in_math.iter().any(|it| *it) {
            card.display_back = Some(
                asked
                    .iter()
                    .zip(&in_math)
                    .map(|(hole, math)| match math {
                        true => format!("${}$", hole.text),
                        false => hole.text.to_string(),
                    })
                    .collect(),
            );
            card.math_hole = true;
        }
        // A hole that stays typed and holds a control sequence asks for the
        // command's spelling. In a formula the input rule draws it instead,
        // unless the author pinned the keyboard back.
        for (hole, math) in asked.iter().zip(&in_math) {
            let typed =
                directives.input != Some(Input::Draw) && (!math || directives.input.is_some());
            if typed && is_control_sequence(hole.text) {
                lints.push(Lint {
                    line,
                    kind: LintKind::UntypableHole {
                        answer: hole.text.to_string(),
                    },
                });
            }
        }
        card.block_holes = block_holes.clone();
        card.images = images.clone();
        card.images_back = images_back.clone();
        card.span_regions = span_regions.clone();
        // The effective question (ADR 0034): this card's own masked context
        // and back, so editing a hidden sibling never stales this card's
        // cached AI artifacts; the block key above addresses the block.
        let mut question = card.context.clone();
        question.extend(card.back.iter().cloned());
        card.content_fingerprint = content_fingerprint(&front, &question);
        card.block_fingerprint = block_key;
        // A cloze sub-card never reverses and keeps no direction: only the
        // per-card `input:` still applies here.
        card.input = directives.input;
        cards.push(card);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Order;

    fn parse(text: &str) -> ParsedDeck {
        super::parse("deck.md", text).unwrap()
    }

    fn err(text: &str) -> ParseError {
        super::parse("deck.md", text).unwrap_err()
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

        let deck = parse("intro prose\n---\nid: nope\n---\n## q\na\n");
        assert_eq!(Frontmatter::default(), deck.frontmatter);
        assert_eq!(None, deck.deck_token);
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
    fn an_initialized_deck_without_a_version_is_a_hard_error() {
        assert_eq!(
            ParseError::MissingDeckVersion(2),
            err("---\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n")
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
        assert_eq!(Some(1), deck.frontmatter.format_version);
    }

    #[test]
    fn deck_metadata_keys_parse_as_single_values_or_lists() {
        let deck = parse(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\nauthors: Alex\nlicense: CC-BY-4.0\ntags: [rust, memory]\n---\n## q\na\n",
        );
        assert_eq!(vec!["Alex".to_string()], deck.frontmatter.authors);
        assert_eq!(Some("CC-BY-4.0"), deck.frontmatter.license.as_deref());
        assert_eq!(
            vec!["rust".to_string(), "memory".to_string()],
            deck.frontmatter.tags
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
    fn an_uninitialized_deck_needs_no_version() {
        let deck = parse("## q\na\n");
        assert_eq!(None, deck.frontmatter.id);
        assert_eq!(None, deck.frontmatter.format_version);
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
            "---\ntags: [x, y]\nlicense: MIT\nauthors: me\nlanguage: de\nrevision: 3\n\
             created-at: 2026-07-19\nfnord: 7\n---\n## q\na\n",
        );
        assert_eq!(vec![unknown(8, "fnord")], deck.lints);
    }

    #[test]
    fn invalid_frontmatter_yaml_is_a_hard_error() {
        let e = err("---\nid: [unclosed\n---\n## q\na\n");
        assert!(matches!(e, ParseError::FrontmatterSyntax { .. }), "{e:?}");
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

    #[test]
    fn a_file_with_no_h2_fronts_is_a_zero_card_deck() {
        let deck = parse("# Title\njust prose\n");
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
    fn preamble_prose_and_h1_title_precede_the_first_card() {
        let deck = parse("# My Deck\nsome intro prose\n\n## q\n---\na\n");
        assert_eq!(Some("My Deck"), deck.title.as_deref());
        assert_eq!(Some("some intro prose"), deck.preamble.as_deref());
        assert_eq!(1, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(vec!["a"], deck.cards[0].back);
    }

    #[test]
    fn preamble_joins_multiple_lines_and_stops_at_the_first_card() {
        let deck = parse("# T\nline one\nline two\n\n## q\na\n");
        assert_eq!(Some("line one line two"), deck.preamble.as_deref());
    }

    #[test]
    fn a_deck_without_preamble_prose_has_none() {
        let deck = parse("# T\n\n## q\na\n");
        assert_eq!(None, deck.preamble);
    }

    #[test]
    fn preamble_is_captured_even_without_a_title() {
        let deck = parse("just an intro\n\n## q\na\n");
        assert_eq!(None, deck.title);
        assert_eq!(Some("just an intro"), deck.preamble.as_deref());
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
    fn a_cloze_hole_on_a_fenced_line_is_still_a_hole() {
        let deck = parse("## q\n---\n```\nlet x = \\blank{5};\n```\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(0), deck.cards[0].hole);
        assert_eq!(vec!["5"], deck.cards[0].back);
        assert_eq!(vec!["```", "let x = ⍰;", "```"], deck.cards[0].context);
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
        assert_eq!(ParseError::FrontWithoutAnswer(1), err("## q\n---\n"));
    }

    // ── Divider, answer, notes ──

    #[test]
    fn the_first_bare_divider_splits_front_from_answer() {
        let deck = parse("## Q\nmore question\n\n---\nthe answer\n");
        assert_eq!("Q\nmore question", deck.cards[0].front);
        assert_eq!(vec!["the answer"], deck.cards[0].back);
    }

    #[test]
    fn a_divider_needs_a_blank_line_or_the_heading_before_it() {
        let deck = parse("## Q\ntext\n---\nanswer\n");
        assert_eq!("Q", deck.cards[0].front);
        assert_eq!(vec!["text", "---", "answer"], deck.cards[0].back);

        let deck = parse("## Q\n---\nanswer\n");
        assert_eq!(vec!["answer"], deck.cards[0].back);
    }

    #[test]
    fn later_dividers_and_four_dashes_are_content() {
        let deck = parse("## Q\n\n---\na\n\n---\n----\nb\n");
        assert_eq!(vec!["a", "---", "----", "b"], deck.cards[0].back);
    }

    #[test]
    fn consecutive_quote_lines_concatenate_into_the_note() {
        let deck = parse("## q\n---\nans\n> one\n> two\n");
        assert_eq!(Some("one\ntwo".to_string()), deck.cards[0].note);
    }

    #[test]
    fn an_all_task_list_answer_is_a_single_correct_checkbox_card() {
        let deck = parse("## Which is prime?\n- [ ] 4\n- [x] 5\n- [ ] 6\n");
        let card = &deck.cards[0];
        assert_eq!(vec!["5"], card.back);
        assert_eq!(
            vec!["4".to_string(), "6".to_string()],
            card.authored_distractors
        );
        assert!(card.hole.is_none());
        assert!(deck.lints.is_empty(), "{:?}", deck.lints);
    }

    #[test]
    fn blank_and_image_only_lines_do_not_turn_authored_choices_into_prose() {
        let deck = parse("## Which is prime?\n\n- [ ] 4\n![](number-line.png)\n- [x] 5\n");
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
        let deck = parse("## Pick one\nsome stimulus\n\n---\n- [x] yes\n- [ ] no\n");
        let card = &deck.cards[0];
        assert_eq!("Pick one\nsome stimulus", card.front);
        assert_eq!(vec!["yes"], card.back);
        assert_eq!(vec!["no".to_string()], card.authored_distractors);
    }

    #[test]
    fn a_mix_of_task_list_and_prose_is_a_plain_card_and_lints() {
        let deck = parse("## q\n- [x] a\nnot an option\n");
        assert!(deck.cards[0].authored_distractors.is_empty());
        assert_eq!(vec!["- [x] a", "not an option"], deck.cards[0].back);
        assert!(
            deck.lints
                .iter()
                .any(|lint| lint.kind == LintKind::ChoiceAnswerMixed)
        );
    }

    #[test]
    fn all_checked_or_no_distractor_lints_needs_both_sides_and_is_plain() {
        let deck = parse("## q\n- [x] a\n- [x] b\n");
        assert!(deck.cards[0].authored_distractors.is_empty());
        assert!(
            deck.lints
                .iter()
                .any(|lint| lint.kind == LintKind::ChoiceMultiCorrectUnsupported)
        );

        let deck = parse("## q\n- [ ] a\n- [ ] b\n");
        assert!(deck.cards[0].authored_distractors.is_empty());
        assert!(
            deck.lints
                .iter()
                .any(|lint| lint.kind == LintKind::ChoiceNeedsBothSides)
        );
    }

    #[test]
    fn a_duplicate_option_lints_and_keeps_first() {
        let deck = parse("## q\n- [x] a\n- [ ] b\n- [ ] b\n");
        assert_eq!(vec!["b".to_string()], deck.cards[0].authored_distractors);
        assert!(
            deck.lints
                .iter()
                .any(|lint| lint.kind == LintKind::DuplicateChoiceOption)
        );
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
        let deck = parse("## q\n- [x] **Paris**\n- [ ] London\n");
        assert_eq!(vec!["**Paris**"], deck.cards[0].back);
        assert_eq!("Paris", crate::inline::strip_inline(&deck.cards[0].back[0]));
        assert_eq!(
            vec!["London".to_string()],
            deck.cards[0].authored_distractors
        );
    }

    #[test]
    fn math_checkbox_options_preserve_authored_source() {
        let deck = parse("## q\n- [x] $x^2$\n- [ ] $x^3$\n");
        assert_eq!(vec!["$x^2$"], deck.cards[0].back);
        assert_eq!(
            vec!["$x^3$".to_string()],
            deck.cards[0].authored_distractors
        );
        assert_eq!("x^2", crate::inline::strip_inline(&deck.cards[0].back[0]));
    }

    #[test]
    fn formatted_and_plain_checkbox_options_are_content_duplicates() {
        let deck = parse("## q\n- [x] $x$\n- [ ] x\n- [ ] y\n");
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
        let before =
            parse("## q <!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n- [x] right\n- [ ] wrong\n");
        let after = parse(
            "## q <!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n- [x] right\n- [ ] different\n",
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
            "## q <!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n---\na\n## r\n---\nb\n\
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
        let deck = parse("## q\n---\n\\## x\n\\> y\n\\---\n\\<!-- z -->\n\\```\n> real note\n");
        assert_eq!(
            vec!["## x", "> y", "---", "<!-- z -->", "```"],
            deck.cards[0].back
        );
        assert_eq!(Some("real note".to_string()), deck.cards[0].note);
    }

    #[test]
    fn a_backslash_before_anything_else_is_literal() {
        let deck = parse("## q\n---\n\\d is a digit class\n\\# x\n");
        assert_eq!(vec!["\\d is a digit class", "\\# x"], deck.cards[0].back);
    }

    #[test]
    fn one_leading_bom_is_stripped() {
        let deck = parse("\u{feff}## q\n---\na\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);

        assert!(parse("\u{feff}\u{feff}## q\n---\na\n").cards.is_empty());
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
        assert_eq!(None, deck.cards[0].note);
        assert!(deck.lints.is_empty());
    }

    // ── Cloze ──

    #[test]
    fn a_cloze_marker_makes_the_card_cloze_and_numbers_holes_in_document_order() {
        let deck = parse("## fill\n---\nthe \\blank{quick} fox\njumps \\blank{over}\n");
        assert_eq!(2, deck.cards.len());

        assert_eq!("fill", deck.cards[0].front);
        assert_eq!(Some(0), deck.cards[0].hole);
        assert_eq!(vec!["quick"], deck.cards[0].back);
        assert_eq!(vec!["the ⍰ fox", "jumps ⬚"], deck.cards[0].context);

        assert_eq!(Some(1), deck.cards[1].hole);
        assert_eq!(vec!["over"], deck.cards[1].back);
        assert_eq!(vec!["the ⬚ fox", "jumps ⍰"], deck.cards[1].context);
    }

    /// A hole cut out of a formula is a piece of that formula, so it has to
    /// be shown as one. `back` is what the learner types and what identifies
    /// the card, so the math form goes to `display_back` alone: revealing
    /// `\pm` as the characters `\pm` shows source code as an answer.
    /// A hole's content is the expected answer, so a hole holding a LaTeX
    /// command asks the learner to spell `\pm`. Inside a formula the input
    /// rule already turns that into a sketch, so what is left to warn about
    /// is a hole that stays typed: one outside any formula, or one the
    /// author pinned back to the keyboard.
    #[test]
    fn a_hole_that_asks_for_a_typed_latex_command_is_linted() {
        let deck = parse("## q\n---\nthe sign is \\blank{\\pm} here\n");
        assert_eq!(
            vec![LintKind::UntypableHole {
                answer: "\\pm".to_string()
            }],
            deck.lints
                .iter()
                .map(|l| l.kind.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_hole_inside_a_formula_is_not_linted_because_it_is_drawn() {
        let deck = parse("## q\n---\n$x = -b \\blank{\\pm} \\sqrt{d}$\n");
        assert!(deck.lints.is_empty(), "{:?}", deck.lints);
    }

    #[test]
    fn a_formula_hole_pinned_back_to_typing_is_linted() {
        let deck = parse("## q <!-- input: type -->\n---\n$x = \\blank{\\pm} y$\n");
        assert_eq!(
            vec![LintKind::UntypableHole {
                answer: "\\pm".to_string()
            }],
            deck.lints
                .iter()
                .map(|l| l.kind.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_ordinary_hole_is_never_linted_as_untypable() {
        let deck =
            parse("## q\n---\n$x = \\frac{-b}{\\blank{2a}}$\nthe value is \\blank{dropped}\n");
        assert!(deck.lints.is_empty(), "{:?}", deck.lints);
    }

    #[test]
    fn a_hole_inside_math_reveals_as_math_but_is_still_typed_as_written() {
        let deck = parse("## q\n---\n$x = -b \\blank{\\pm} \\sqrt{d}$\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["\\pm"], deck.cards[0].back);
        assert_eq!(["$\\pm$"], *deck.cards[0].back_for_display());
    }

    #[test]
    fn a_hole_in_display_math_reveals_as_math_too() {
        let deck = parse("## q\n---\n$$a^2 - b^2 = \\blank{(a-b)}(a+b)$$\n");
        assert_eq!(["$(a-b)$"], *deck.cards[0].back_for_display());
    }

    #[test]
    fn a_hole_in_prose_reveals_exactly_as_written() {
        let deck = parse("## q\n---\nthe value is \\blank{dropped}\n");
        assert_eq!(None, deck.cards[0].display_back);
        assert_eq!(["dropped"], *deck.cards[0].back_for_display());
    }

    /// Two holes on one line, only one of them inside the formula.
    #[test]
    fn only_the_hole_inside_the_formula_reveals_as_math() {
        let deck = parse("## q\n---\nthe \\blank{sign} in $x = \\blank{\\pm} y$\n");
        assert_eq!(2, deck.cards.len());
        assert_eq!(None, deck.cards[0].display_back);
        assert_eq!(["$\\pm$"], *deck.cards[1].back_for_display());
    }

    #[test]
    fn bare_cloze_without_a_brace_is_literal() {
        let deck = parse("## q\n---\na \\blank marker\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(None, deck.cards[0].hole);
        assert_eq!(vec!["a \\blank marker"], deck.cards[0].back);
    }

    #[test]
    fn double_backslash_cloze_is_a_literal_marker() {
        let deck = parse("## q\n---\na \\\\blank{x} b\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(None, deck.cards[0].hole);
        assert_eq!(vec!["a \\blank{x} b"], deck.cards[0].back);
    }

    #[test]
    fn a_named_hole_parses_and_the_sub_card_carries_the_name() {
        let deck = parse("## fill\n---\nthe \\blank[speed]{quick} \\blank{fox}\n");
        assert_eq!(2, deck.cards.len());
        assert_eq!(Some("speed"), deck.cards[0].hole_name.as_deref());
        assert_eq!(vec!["quick"], deck.cards[0].back);
        assert_eq!(None, deck.cards[1].hole_name.as_deref());
        assert_eq!(vec!["fox"], deck.cards[1].back);
        assert_eq!(vec!["the ⍰ ⬚"], deck.cards[0].context);
    }

    /// A name is an address, never an identity (ADR 0032): naming a hole an
    /// author has already drilled must not reset its schedule, and
    /// `store::realign_holes` decides that from these fingerprints alone.
    #[test]
    fn naming_a_hole_leaves_the_card_it_addresses_untouched() {
        let unnamed = parse("## q\n---\n\\blank{Unit}, \\blank{integration}\n");
        let named = parse("## q\n---\n\\blank[base]{Unit}, \\blank[middle]{integration}\n");
        assert_eq!(2, named.cards.len());
        for (n, (plain, addressed)) in unnamed.cards.iter().zip(&named.cards).enumerate() {
            assert_eq!(plain.block_holes, addressed.block_holes, "hole {n} holes");
            assert_eq!(
                plain.hash_lines, addressed.hash_lines,
                "hole {n} hash lines"
            );
            assert_eq!(plain.back, addressed.back, "hole {n} answer");
            assert_eq!(plain.context, addressed.context, "hole {n} context");
        }
    }

    #[test]
    fn a_hole_name_may_carry_an_underscore_or_a_hyphen() {
        let deck = parse("## q\n---\n\\blank[base_two]{x} \\blank[base-three]{y}\n");
        assert_eq!(2, deck.cards.len());
        assert_eq!(Some("base_two"), deck.cards[0].hole_name.as_deref());
        assert_eq!(Some("base-three"), deck.cards[1].hole_name.as_deref());
    }

    /// A hole's line fingerprint masks that hole and renders every other one
    /// as its text, so the siblings' wording is part of the context this hole
    /// is identified by.
    #[test]
    fn a_holes_line_fingerprint_reads_the_other_holes_text() {
        let one = parse("## q\n---\n\\blank{a}, \\blank{alpha}\n");
        let two = parse("## q\n---\n\\blank{a}, \\blank{omega}\n");
        assert_ne!(
            one.cards[0].block_holes[0].line_fp, two.cards[0].block_holes[0].line_fp,
            "the sibling's text sits in this hole's line"
        );
        assert_eq!(
            one.cards[0].block_holes[0].text_fp, two.cards[0].block_holes[0].text_fp,
            "what this hole asks for did not change"
        );
    }

    #[test]
    fn a_malformed_hole_name_is_a_parse_error() {
        for spelling in [
            "\\blank[]{x}",
            "\\blank[a b]{x}",
            "\\blank[a.b]{x}",
            "\\blank[a{x}",
            "\\blank[name]",
            "\\blank[name] {x}",
        ] {
            assert_eq!(
                ParseError::InvalidHoleName(3),
                err(&format!("## q\n---\n{spelling}\n")),
                "for `{spelling}`"
            );
        }
    }

    #[test]
    fn two_holes_sharing_a_name_are_drilled_as_one_card_asking_both_spans() {
        let deck = parse("## q\n---\n\\blank[hs]{SYN}, \\blank[hs]{SYN-ACK}, \\blank{ACK}\n");
        assert_eq!(2, deck.cards.len(), "the group is one card, `ACK` another");
        assert_eq!(vec!["SYN", "SYN-ACK"], deck.cards[0].back);
        assert_eq!(Some("hs"), deck.cards[0].hole_name.as_deref());
        assert_eq!(vec!["⍰, ⍰, ⬚"], deck.cards[0].context);
        assert_eq!(vec!["ACK"], deck.cards[1].back);
        assert_eq!(vec!["⬚, ⬚, ⍰"], deck.cards[1].context);
        assert_eq!(Some(0), deck.cards[0].hole);
        assert_eq!(Some(1), deck.cards[1].hole);
    }

    #[test]
    fn a_group_may_span_lines() {
        let deck = parse("## q\n---\n\\blank[c]{Berlin} is the capital\nof \\blank[c]{Germany}\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["Berlin", "Germany"], deck.cards[0].back);
        assert_eq!(vec!["⍰ is the capital", "of ⍰"], deck.cards[0].context);
    }

    #[test]
    fn a_group_of_three_is_one_card() {
        let deck = parse("## q\n---\n\\blank[a]{x} \\blank[a]{y} \\blank[a]{z}\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["x", "y", "z"], deck.cards[0].back);
    }

    /// A merged card asks two spans, which are exact answers rather than the
    /// key points a multi-line plain answer holds, so it stays typed.
    #[test]
    fn a_merged_card_is_typed_at_reconstruct_not_self_graded() {
        let deck = parse("## q\n---\n\\blank[hs]{SYN}, \\blank[hs]{SYN-ACK}\n");
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
            "## q\n\\blank[hs]{SYN}, \\blank[hs]{SYN-ACK}, \\blank{ACK}\n\
             > Shared.\n> hs: Both halves of the opening.\n",
        );
        assert_eq!(
            Some("Both halves of the opening."),
            deck.cards[0].note.as_deref()
        );
        assert_eq!(Some("Shared."), deck.cards[1].note.as_deref());
    }

    /// D4: the merged card inherits nothing, and the hole it did not touch
    /// rides the positional shift on its fingerprint.
    #[test]
    fn grouping_resets_the_merged_card_alone() {
        let before = parse("## q\n---\n\\blank{SYN}, \\blank{SYN-ACK}, \\blank{ACK}\n");
        let after = parse("## q\n---\n\\blank[hs]{SYN}, \\blank[hs]{SYN-ACK}, \\blank{ACK}\n");
        let outcome =
            crate::store::realign_holes(&before.cards[0].block_holes, &after.cards[0].block_holes);
        assert_eq!(vec![(2, 1)], outcome.remap, "`ACK` moves from -2 to -1");
        assert_eq!(vec![0], outcome.fresh, "the merged card starts fresh");
        assert_eq!(vec![0, 1], outcome.orphaned, "neither half is inherited");
    }

    /// `line_fp` is the context half of a hole's identity, so it must not
    /// move when only the hidden text does. A group hides several spans, and
    /// every one of them is masked out of its own line.
    #[test]
    fn a_groups_line_fingerprint_ignores_the_text_it_hides() {
        let one = parse("## q\n---\n\\blank[hs]{SYN}, \\blank[hs]{SYN-ACK}\n");
        let two = parse("## q\n---\n\\blank[hs]{SYN}, \\blank[hs]{SYNACK}\n");
        let (one, two) = (&one.cards[0].block_holes[0], &two.cards[0].block_holes[0]);
        assert_eq!(one.line_fp, two.line_fp, "the line around the spans is one");
        assert_ne!(one.text_fp, two.text_fp, "what it asks for did change");
    }

    #[test]
    fn naming_a_hole_without_grouping_it_keeps_every_fingerprint() {
        let plain = parse("## q\n---\n\\blank{SYN}, \\blank{SYN-ACK}\n");
        let named = parse("## q\n---\n\\blank[a]{SYN}, \\blank[b]{SYN-ACK}\n");
        assert_eq!(plain.cards[0].block_holes, named.cards[0].block_holes);
    }

    #[test]
    fn two_cards_may_each_carry_a_hole_of_the_same_name() {
        let deck = parse("## a\n---\n\\blank[base]{Unit}\n\n## b\n---\n\\blank[base]{atom}\n");
        assert_eq!(2, deck.cards.len());
        assert_eq!(Some("base"), deck.cards[0].hole_name.as_deref());
        assert_eq!(Some("base"), deck.cards[1].hole_name.as_deref());
    }

    #[test]
    fn escaped_braces_inside_a_hole_are_stripped_and_do_not_count() {
        let deck = parse("## q\n---\nw \\blank{a \\{b\\} c} z\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["a {b} c"], deck.cards[0].back);
        assert_eq!(vec!["w ⍰ z"], deck.cards[0].context);

        let deck = parse("## q\n---\nw \\blank{a \\{b} c\n");
        assert_eq!(vec!["a {b"], deck.cards[0].back);
        assert_eq!(vec!["w ⍰ c"], deck.cards[0].context);
    }

    #[test]
    fn backslash_backslash_inside_a_hole_is_a_literal_backslash() {
        let deck = parse("## q\n---\nw \\blank{a\\\\b} z\n");
        assert_eq!(vec!["a\\b"], deck.cards[0].back);
    }

    #[test]
    fn an_unclosed_hole_is_a_line_numbered_error() {
        assert_eq!(
            ParseError::UnclosedHole(3),
            err("## q\n---\nw \\blank{oops\n")
        );
    }

    #[test]
    fn an_empty_hole_is_an_error() {
        assert_eq!(ParseError::EmptyHole(3), err("## q\n---\nw \\blank{} z\n"));
        assert_eq!(
            ParseError::EmptyHole(3),
            err("## q\n---\nw \\blank{  } z\n")
        );
    }

    #[test]
    fn hole_content_is_not_rescanned() {
        let deck = parse("## q\n---\nw \\blank{x \\blank{y}} z\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["x \\blank{y}"], deck.cards[0].back);
        assert_eq!(
            vec![Lint {
                line: 3,
                kind: LintKind::ClozeInHole
            }],
            deck.lints
        );
    }

    #[test]
    fn a_block_note_naming_a_holes_answer_is_reported_per_hole() {
        // The spec's motivating fixture: reviewing a later hole shows a note
        // that spells out the first hole's answer.
        let deck = parse(
            "## The test pyramid, bottom to top\n\
             \\blank{Unit}, \\blank{integration}, \\blank{end-to-end}\n\
             > Unit tests sit at the base because they are fastest and most numerous.\n",
        );
        assert_eq!(
            vec![Lint {
                line: 1,
                kind: LintKind::NoteContainsHoleAnswer {
                    hole: 1,
                    answer: "Unit".to_string()
                }
            }],
            deck.lints,
            "only the hole whose answer appears is named, and 1-based"
        );
    }

    #[test]
    fn an_addressed_note_replaces_the_block_note_for_its_hole_alone() {
        let deck = parse(
            "## q\n\\blank[base]{Unit}, \\blank{integration}\n\
             > Shared.\n> base: Fastest and most numerous.\n",
        );
        assert_eq!(
            Some("Fastest and most numerous."),
            deck.cards[0].note.as_deref()
        );
        assert_eq!(Some("Shared."), deck.cards[1].note.as_deref());
    }

    #[test]
    fn a_plus_addressed_note_keeps_the_block_note_above_it() {
        let deck = parse(
            "## q\n\\blank[base]{Unit}, \\blank{integration}\n\
             > Shared.\n> base+: And fastest.\n",
        );
        assert_eq!(Some("Shared.\nAnd fastest."), deck.cards[0].note.as_deref());
        assert_eq!(Some("Shared."), deck.cards[1].note.as_deref());
    }

    #[test]
    fn an_addressed_note_with_no_block_note_stands_alone() {
        let deck = parse("## q\n\\blank[base]{Unit}, \\blank{integration}\n> base+: Fastest.\n");
        assert_eq!(Some("Fastest."), deck.cards[0].note.as_deref());
        assert_eq!(None, deck.cards[1].note.as_deref());
    }

    #[test]
    fn two_lines_addressed_to_one_hole_join_in_written_order() {
        let deck = parse(
            "## q\n\\blank[base]{Unit}, \\blank{integration}\n\
             > base: First.\n> base: Second.\n",
        );
        assert_eq!(Some("First.\nSecond."), deck.cards[0].note.as_deref());
    }

    /// A card with no named hole cannot be addressing anything, so a note
    /// beginning `2:` is prose and stays prose.
    #[test]
    fn a_note_that_looks_addressed_is_prose_where_no_hole_is_named() {
        let deck = parse("## q\n\\blank{Unit}, \\blank{integration}\n> 2: the second one.\n");
        assert_eq!(Vec::<Lint>::new(), deck.lints);
        for card in &deck.cards {
            assert_eq!(Some("2: the second one."), card.note.as_deref());
        }
    }

    #[test]
    fn an_address_is_separated_from_its_text_by_a_space() {
        let deck = parse("## q\n\\blank[base]{Unit}, \\blank{integration}\n> base:no space.\n");
        assert_eq!(Vec::<Lint>::new(), deck.lints);
        for card in &deck.cards {
            assert_eq!(Some("base:no space."), card.note.as_deref());
        }
    }

    #[test]
    fn an_address_naming_no_hole_of_this_card_is_reported_and_kept() {
        let deck = parse("## q\n\\blank[base]{Unit}, \\blank{integration}\n> bass: typo.\n");
        assert_eq!(
            vec![Lint {
                line: 1,
                kind: LintKind::NoteNamesNoHole {
                    name: "bass".to_string()
                }
            }],
            deck.lints
        );
        for card in &deck.cards {
            assert_eq!(
                Some("bass: typo."),
                card.note.as_deref(),
                "the line is still shown rather than lost"
            );
        }
    }

    #[test]
    fn the_pyramid_stops_leaking_once_its_note_is_addressed() {
        let deck = parse(
            "## The test pyramid, bottom to top\n\
             \\blank[base]{Unit}, \\blank{integration}, \\blank{end-to-end}\n\
             > base: Unit tests sit at the base because they are fastest and most numerous.\n",
        );
        assert_eq!(
            Vec::<Lint>::new(),
            deck.lints,
            "no other hole shows the note that names `Unit`"
        );
        assert!(
            deck.cards[0]
                .note
                .as_deref()
                .is_some_and(|note| note.starts_with("Unit tests sit at the base"))
        );
        assert_eq!(None, deck.cards[1].note.as_deref());
        assert_eq!(None, deck.cards[2].note.as_deref());
    }

    #[test]
    fn a_note_naming_no_hole_answer_is_silent() {
        let deck = parse("## q\n\\blank{Unit}, \\blank{integration}\n> Fastest at the base.\n");
        assert_eq!(Vec::<Lint>::new(), deck.lints);
    }

    #[test]
    fn a_single_hole_block_is_never_reported_however_the_note_reads() {
        // With one hole there is no other card to leak to: the note is on the
        // card whose answer it names, which is the ordinary way to write one.
        let deck = parse("## q\n\\blank{Unit} tests\n> Unit tests are fastest.\n");
        assert_eq!(Vec::<Lint>::new(), deck.lints);
    }

    #[test]
    fn a_hole_answer_inside_a_longer_word_is_not_a_match() {
        for note in ["Reunites the suites.", "Unitary tests are narrow."] {
            let deck = parse(&format!(
                "## q\n\\blank{{unit}}, \\blank{{integration}}\n> {note}\n"
            ));
            assert_eq!(
                Vec::<Lint>::new(),
                deck.lints,
                "`unit` inside {note:?} is not the answer appearing"
            );
        }
    }

    #[test]
    fn a_short_hole_answer_is_below_the_reporting_floor() {
        let deck = parse("## q\n\\blank{TCP}, \\blank{integration}\n> TCP is a protocol.\n");
        assert_eq!(
            Vec::<Lint>::new(),
            deck.lints,
            "three characters match too much prose to be worth reporting"
        );
    }

    #[test]
    fn nested_balanced_braces_stay_inside_the_hole() {
        let deck = parse("## q\n---\nw \\blank{f{g}h} z\n");
        assert_eq!(vec!["f{g}h"], deck.cards[0].back);
    }

    #[test]
    fn a_reveal_directive_on_a_cloze_card_is_linted_not_obeyed() {
        let deck = parse("## q\n---\na \\blank{b} c\n<!-- reveal: line -->\n");
        assert_eq!(None, deck.cards[0].reveal);
        assert_eq!(
            vec![Lint {
                line: 4,
                kind: LintKind::RevealOnCloze
            }],
            deck.lints
        );
    }

    #[test]
    fn cloze_cards_never_produce_a_reversed_twin() {
        let deck = parse(
            "---\ndirection: both\n---\n## q\n---\na \\blank{b} c\n<!-- direction: both -->\n",
        );
        assert_eq!(Some(Direction::Both), deck.frontmatter.direction);
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(0), deck.cards[0].hole);
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
tags: [a, b]
license: MIT
authors: someone
language: de
revision: 3
created-at: 2026-07-19
---
# The Title

## The question <!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->

---
the answer
<!-- reveal: flip -->
<!-- input: type -->
<!-- direction: reverse -->
<!-- at: src/caching.rs:46-66 fingerprint: xxh64-0123456789abcdef asset: sha256-abc123.rs -->
<!-- given: state - the parser position -->
<!-- given: partial - the card -->
"#;
        let document = parse_document(text).unwrap();
        assert_eq!(
            Frontmatter {
                id: Some("deck-9w2c7x4k1m8q3z5t0v6b2n4d8f".into()),
                format_version: Some(1),
                authors: vec!["someone".into()],
                created_at: Some("2026-07-19".into()),
                license: Some("MIT".into()),
                tags: vec!["a".into(), "b".into()],
                source: vec!["https://example.org/book".into(), "notes.md".into()],
                requires: vec!["basics".into()],
                link: vec!["https://docs.rs/tokio".into()],
                trace: Some("how a keypress becomes a grade".into()),
                reveal: Some(Reveal::Line),
                order: Some(Order::Sequential),
                input: Some(Input::Draw),
                direction: Some(Direction::Both),
                sampling: None,
                unspliceable: false,
                personal_for: None,
            },
            document.frontmatter
        );
        assert_eq!(Some("The Title"), document.title.as_deref());
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
                reveal_line: Some(29),
                input: Some(Input::Type),
                direction: Some(Direction::Reverse),
                sampling: None,
                citations: vec![crate::card::SourceCitation {
                    locator: "src/caching.rs:46-66".into(),
                    fingerprint: Some(0x0123456789abcdef),
                    asset: Some("sha256-abc123.rs".into()),
                    line: 32,
                }],
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
                line: 32,
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
    fn without_a_blank_line_the_divider_is_content_and_the_image_lands_on_the_back() {
        let deck = parse("## q\n![](x.png)\n---\nWaxing\n");
        let card = &deck.cards[0];
        assert!(card.images.is_empty());
        assert_eq!(vec![PathBuf::from("x.png")], img_srcs(&card.images_back));
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
        assert!(card.hole.is_none());
    }

    #[test]
    fn a_cloze_card_carries_front_and_back_images() {
        let deck = parse("## front\n![](f.png)\n\n---\nthe \\blank{answer} here\n![](b.png)\n");
        assert_eq!(1, deck.cards.len());
        let card = &deck.cards[0];
        assert_eq!(Some(0), card.hole);
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
        let base = parse("## q <!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\nWaxing\n");
        let with =
            parse("## q <!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\nWaxing\n![](moon.png)\n");
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
            "| word | meaning |\n|---|---|\n| hund | dog | <!-- r:4k2x9w -->\n| katze | cat | <!-- r:7m3p5q -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert_eq!(2, deck.cards.len());
        let first = &deck.cards[0];
        assert_eq!("hund", first.front);
        assert_eq!(vec!["dog"], first.back);
        assert_eq!(None, first.note);
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
            "| word | meaning | note |\n|---|---|---|\n| a | b | care | <!-- r:4k2x9w -->\n| c | d | | <!-- r:7m3p5q -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert_eq!(2, deck.cards.len());
        assert_eq!(Some("care"), deck.cards[0].note.as_deref());
        assert_eq!(None, deck.cards[1].note);
        assert!(deck.cards[0].context.is_empty());
    }

    #[test]
    fn an_unstamped_row_stays_id_less_even_under_a_container() {
        let text = format!(
            "| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n| p | q |\n<!-- id: {CONTAINER} -->\n"
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
            "| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n| p | q | <!-- r:7m3p5q -->\n<!-- direction: both -->\n<!-- id: {CONTAINER} -->\n"
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
        let deck = parse("## q\n| a | b |\n|---|---|\n| x | y |\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["q"], deck.cards[0].context);
    }

    #[test]
    fn a_table_after_a_complete_card_is_its_own_block() {
        let deck = parse("## q\n---\nanswer\n\n| a | b |\n|---|---|\n| x | y |\n");
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
            err("| a | b\n|---|---|\n")
        );
        assert_eq!(
            ParseError::TableLineMalformed(3),
            err("| a | b |\n|---|---|\n| x | y\n")
        );
        assert_eq!(ParseError::TableLineMalformed(1), err("|\n|---|\n"));
    }

    #[test]
    fn a_table_needs_two_or_three_columns() {
        assert_eq!(
            ParseError::TableColumns { line: 1, found: 1 },
            err("| a |\n|---|\n| x |\n")
        );
        assert_eq!(
            ParseError::TableColumns { line: 1, found: 4 },
            err("| a | b | c | d |\n|---|---|---|---|\n")
        );
    }

    #[test]
    fn an_empty_table_ends_on_its_delimiter() {
        let deck = parse("| a | b |\n|---|---|\n");
        assert_eq!(2, deck.tables[0].end_line);
    }

    #[test]
    fn every_table_line_matches_the_header_width() {
        assert_eq!(
            ParseError::TableRowWidth {
                line: 2,
                found: 1,
                expected: 2
            },
            err("| a | b |\n|---|\n")
        );
        assert_eq!(
            ParseError::TableRowWidth {
                line: 3,
                found: 3,
                expected: 2
            },
            err("| a | b |\n|---|---|\n| x | y | z |\n")
        );
    }

    #[test]
    fn a_blank_marker_or_image_in_a_cell_is_refused() {
        assert_eq!(
            ParseError::TableCellHole(3),
            err("| a | b |\n|---|---|\n| x | \\blank{y} |\n")
        );
        assert_eq!(
            ParseError::TableCellImage(3),
            err("| a | b |\n|---|---|\n| ![alt](x.png) | y |\n")
        );
    }

    #[test]
    fn escaped_images_in_cells_stay_legal_and_a_real_one_after_them_still_refuses() {
        let deck = parse("| a | b |\n|---|---|\n| \\![x] \\![y] | z |\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!("\\![x] \\![y]", deck.cards[0].front);

        assert_eq!(
            ParseError::TableCellImage(3),
            err("| a | b |\n|---|---|\n| \\![x] ![y](p.png) | z |\n")
        );
    }

    #[test]
    fn an_invalid_or_duplicate_row_stamp_is_refused() {
        assert_eq!(
            ParseError::TableRowStamp {
                line: 3,
                value: "xyz".into()
            },
            err("| a | b |\n|---|---|\n| x | y | <!-- r:xyz -->\n")
        );
        assert_eq!(
            ParseError::TableDuplicateStamp {
                line: 4,
                value: "4k2x9w".into()
            },
            err("| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n| p | q | <!-- r:4k2x9w -->\n")
        );
    }

    #[test]
    fn only_directive_comments_may_follow_a_table() {
        assert_eq!(
            ParseError::TableTrailing(4),
            err("| a | b |\n|---|---|\n| x | y |\nstray prose\n")
        );
        assert_eq!(
            ParseError::TableTrailing(4),
            err("| a | b |\n|---|---|\n| x | y |\n> a note\n")
        );
        assert_eq!(
            ParseError::TableTrailing(5),
            err("| a | b |\n|---|---|\n| x | y |\n\n| z | w |\n")
        );
    }

    #[test]
    fn an_empty_front_or_back_cell_is_refused() {
        assert_eq!(
            ParseError::EmptyFront(3),
            err("| a | b |\n|---|---|\n| | y | <!-- r:4k2x9w -->\n")
        );
        assert_eq!(
            ParseError::FrontWithoutAnswer(3),
            err("| a | b |\n|---|---|\n| x | |\n")
        );
    }

    #[test]
    fn an_escaped_pipe_stays_in_the_cell() {
        let deck = parse("| a | b |\n|---|---|\n| x \\| y | z |\n");
        assert_eq!("x | y", deck.cards[0].front);
    }

    #[test]
    fn a_single_hyphen_delimiter_is_valid_gfm_and_accepted() {
        // GFM defines a delimiter cell as `:?-+:?` with no minimum hyphen
        // count, so `| - |` is a table in every GFM renderer.
        let deck = parse("| front | back |\n| - | -- |\n| question | answer |\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!("question", deck.cards[0].front);
    }

    #[test]
    fn alignment_colons_in_the_delimiter_are_accepted() {
        let deck = parse("| a | b |\n|:---|---:|\n| x | y |\n");
        assert_eq!(1, deck.cards.len());
    }

    #[test]
    fn adjacent_tables_split_on_the_second_header() {
        let deck = parse("| a | b |\n|---|---|\n| x | y |\n| c | d |\n|---|---|\n| z | w |\n");
        assert_eq!(2, deck.cards.len());
        assert!(deck.cards[0].context.is_empty());
        assert!(deck.cards[1].context.is_empty());
    }

    #[test]
    fn an_empty_heading_above_a_table_becomes_its_title() {
        let text = format!(
            "## Verbs of arguing\n| word | meaning |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert_eq!(1, deck.cards.len(), "the heading is a title, not a card");
        assert_eq!(vec!["Verbs of arguing"], deck.cards[0].context);
        assert_eq!(Some(format!("{CONTAINER}-t4k2x9w")), deck.cards[0].id());
    }

    #[test]
    fn a_heading_with_answer_content_before_a_table_stays_a_card() {
        let deck = parse("## q\nanswer\n| a | b |\n|---|---|\n| x | y |\n");
        assert_eq!(2, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(vec!["answer"], deck.cards[0].back);
        assert!(deck.cards[1].context.is_empty(), "the table is untitled");
    }

    #[test]
    fn a_heading_with_only_a_note_keeps_being_a_card_and_fails_loudly() {
        assert_eq!(
            ParseError::FrontWithoutAnswer(1),
            err("## q\n> a note\n| a | b |\n|---|---|\n| x | y |\n")
        );
    }

    #[test]
    fn a_heading_id_becomes_the_tables_container_id() {
        let text = format!(
            "## Title <!-- id: {CONTAINER} -->\n| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n"
        );
        let deck = parse(&text);
        assert_eq!(Some(format!("{CONTAINER}-t4k2x9w")), deck.cards[0].id());
    }

    #[test]
    fn heading_directives_apply_to_the_titled_tables_rows() {
        let text = format!(
            "## Title <!-- direction: both -->\n| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n<!-- id: {CONTAINER} -->\n"
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
        let deck = parse("## Title\n\n| a | b |\n|---|---|\n| x | y |\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["Title"], deck.cards[0].context);
    }

    #[test]
    fn a_table_id_line_must_hold_a_base_card_id() {
        assert_eq!(
            ParseError::InvalidCardId {
                line: 4,
                value: format!("{CONTAINER}-2"),
            },
            err(&format!(
                "| a | b |\n|---|---|\n| x | y |\n<!-- id: {CONTAINER}-2 -->\n"
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
    fn a_media_element_takes_at_most_one_crop() {
        let error = err(
            "## q\n![](a.png)\n<!-- crop: rect x=0 y=0 width=9 height=9 -->\n<!-- crop: rect x=1 y=1 width=2 height=2 -->\n---\nanswer\n",
        );
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("at most one"), "{message}");
    }

    #[test]
    fn one_media_element_carries_one_unit_across_regions_and_crop() {
        let error = err(
            "## q\n![](a.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 -->\n<!-- blank: rect x=1% y=1% width=2% height=2% -->\n---\nanswer\n",
        );
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("same unit"), "{message}");

        let error = err(
            "## q\n![](a.png)\n<!-- crop: rect x=0% y=0% width=9% height=9% -->\n<!-- blank: rect x=1 y=1 width=2 height=2 -->\n---\nanswer\n",
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
        let error =
            err("| a | b |\n|---|---|\n| x | y |\n<!-- blank: rect x=1 y=1 width=2 height=2 -->\n");
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
            "## name the parts <!-- id: {RTOK} -->\n![](hand.png)\n{regions}\n\n---\nthe parts\n"
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
            "## identify both <!-- id: card-mixed1 -->\n---\nalpha beta\n<!-- blank: span [pair] hidden=\"alpha\" b:a1b2c3 -->\n![](diagram.png)\n<!-- blank: rect [pair] x=1 y=1 width=2 height=2 hidden=\"diagram\" b:d4e5f6 -->\n",
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
            "## name the parts <!-- id: {RTOK} -->\n![](hand.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 hidden=\"lunate\" b:a1b2c3 -->\n<!-- blank: rect x=5 y=5 width=2 height=2 hidden=\"hamate\" b:d4e5f6 -->\n\n---\nthe parts\n> the lunate sits center\n"
        ));
        for card in &deck.cards {
            assert_eq!(
                Some("the lunate sits center"),
                card.note.as_deref(),
                "the block's note rides every region card, as cloze notes do"
            );
        }
    }

    #[test]
    fn a_text_hole_plus_a_blank_span_is_a_composition_error() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\nalpha \\blank{{beta}}\n<!-- blank: span hidden=\"alpha\" b:a1b2c3 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("text holes"), "{message}");
    }

    #[test]
    fn a_text_hole_plus_a_blank_rect_is_a_composition_error() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n![](hand.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 b:a1b2c3 -->\n\n---\nalpha \\blank{{beta}}\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("text holes"), "{message}");
    }

    #[test]
    fn a_task_list_answer_plus_a_blank_region_is_a_composition_error() {
        let error = err(&format!(
            "## pick <!-- id: {RTOK} -->\n---\n- [x] alpha\n- [ ] beta\n<!-- blank: span hidden=\"alpha\" b:a1b2c3 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("task-list"), "{message}");
    }

    #[test]
    fn an_incomplete_task_list_plus_a_blank_region_is_still_a_composition_error() {
        let error = err(&format!(
            "## pick <!-- id: {RTOK} -->\n---\n- [ ] alpha\n- [ ] beta\n<!-- blank: span hidden=\"alpha\" b:a1b2c3 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("task-list"), "{message}");
    }

    #[test]
    fn covers_and_crops_stay_legal_beside_holes_and_task_lists() {
        let cloze = parse(&format!(
            "## q <!-- id: {RTOK} -->\n![](a.png)\n<!-- cover: rect x=1 y=1 width=2 height=2 -->\n<!-- crop: rect x=0 y=0 width=9 height=9 -->\n\n---\nw \\blank{{z}} y\n"
        ));
        assert!(
            cloze.cards[0].hole.is_some(),
            "a cover is a display transform, not a template: the hole cards stand"
        );
        assert_eq!(1, cloze.cards[0].images[0].regions.len());

        let choice = parse(&format!(
            "## pick <!-- id: {RTOK} -->\n![](a.png)\n<!-- cover: rect x=1 y=1 width=2 height=2 -->\n\n---\n- [x] alpha\n- [ ] beta\n"
        ));
        assert_eq!(1, choice.cards.len());
        assert!(!choice.cards[0].authored_distractors.is_empty());
    }

    #[test]
    fn a_cover_span_cannot_bind_to_text_inside_a_cloze_hole() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\nthe first is \\blank{{alpha}}\n<!-- cover: span hidden=\"alpha\" -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("occurs 0 time(s)"), "{message}");
    }

    #[test]
    fn a_cloze_gap_is_a_word_boundary_for_an_adjacent_cover_span() {
        let deck = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nleft\\blank{{middle}}right\n<!-- cover: span hidden=\"right\" -->\n"
        ));
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(0), deck.cards[0].hole);
    }

    #[test]
    fn a_span_binds_into_styled_contents_over_the_stream() {
        let deck = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nthe **lunate** bone\n<!-- blank: span hidden=\"lunate\" b:a1b2c3 -->\n"
        ));
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["lunate"], deck.cards[0].back);
    }

    #[test]
    fn a_span_crossing_a_style_boundary_is_rejected_naming_it() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\n**New** York is big\n<!-- blank: span hidden=\"New York\" b:a1b2c3 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("style boundary"), "{message}");
    }

    #[test]
    fn a_span_cannot_bind_to_a_markdown_link_destination() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\nread [the guide](https://secret.example/path)\n<!-- blank: span hidden=\"secret.example\" b:a1b2c3 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("occurs 0 time(s)"), "{message}");
    }

    #[test]
    fn a_span_cannot_bind_to_a_balanced_link_destination_suffix() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\nread [the article](https://example.test/a(part)suffix) now\n<!-- blank: span hidden=\"suffix\" boundary=char b:a1b2c3 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("occurs 0 time(s)"), "{message}");
    }

    #[test]
    fn a_crossing_candidate_does_not_consume_an_overlapping_matchable_occurrence() {
        let deck = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nba\\blank{{x}}nana\n<!-- cover: span hidden=\"ana\" boundary=char -->\n"
        ));
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(0), deck.cards[0].hole);
    }

    #[test]
    fn a_whole_math_formula_is_a_legal_span_blank() {
        let deck = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nsum $x+y$ here\n<!-- blank: span hidden=\"x+y\" b:a1b2c3 -->\n"
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
                "## q <!-- id: {RTOK} -->\n---\n$x^2$\n<!-- blank: span hidden=\"{hidden}\" boundary=char b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\n{line}\n<!-- blank: span hidden=\"a &\" boundary=char b:a1b2c3 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("structural token `&`"), "{message}");
    }

    #[test]
    fn a_span_endpoint_inside_a_control_word_is_rejected_naming_it() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\n$x \\leq y$\n<!-- blank: span hidden=\"q\" boundary=char b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\n{line}\n<!-- blank: span hidden=\"{hidden}\" boundary=char b:a1b2c3 -->\n"
        ));
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["$A=\u{2370}$"], deck.cards[0].context);
    }

    #[test]
    fn a_matrix_cell_is_a_complete_structural_unit() {
        let line = r"$\begin{pmatrix}a & b\end{pmatrix}$";
        let deck = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\n{line}\n<!-- blank: span hidden=\"a\" b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\n{line}\n<!-- blank: span hidden=\"ab\" b:a1b2c3 -->\n"
        ));
        assert_eq!(
            vec!["$\\frac{\u{2370}}{cd}$"],
            deck.cards[0].context,
            "the equal-depth interior of one group is a unit"
        );

        let hidden = r"ab}{cd";
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\n{line}\n<!-- blank: span hidden=\"{hidden}\" boundary=char b:a1b2c3 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("brace group"), "{message}");
    }

    #[test]
    fn a_partial_match_containing_a_command_is_rejected_in_v1() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\n$x + \\gamma + y$\n<!-- blank: span hidden=\"\\\\gamma\" boundary=char b:a1b2c3 -->\n"
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
                "## q <!-- id: {RTOK} -->\n---\n{line}\n<!-- blank: span hidden=\"{hidden}\" boundary=char b:a1b2c3 -->\n"
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
                "## q <!-- id: {RTOK} -->\n---\n{line}\n<!-- blank: span hidden=\"{hidden}\" boundary=char b:a1b2c3 -->\n"
            ));
            assert_eq!(1, deck.cards.len(), "{hidden}");
            assert_eq!(vec!["$\u{2370}$"], deck.cards[0].context, "{hidden}");
        }
    }

    #[test]
    fn space_padded_dollars_are_prose_so_the_span_binds_as_text() {
        let deck = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\n$ \\gamma $\n<!-- blank: span hidden=\"\\\\gamma\" b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\n$a % target$ target\n<!-- blank: span hidden=\"target\" b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\n$\\phantom{{target}}$\n<!-- blank: span hidden=\"target\" b:a1b2c3 -->\n"
        ));
        assert!(
            matches!(error, ParseError::InvalidRegion { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_span_inside_math_verb_is_rejected_because_the_blank_stays_literal() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\n$\\verb|target|$\n<!-- blank: span hidden=\"target\" b:a1b2c3 -->\n"
        ));
        assert!(
            matches!(error, ParseError::InvalidRegion { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn an_unterminated_verb_after_a_span_is_a_loud_binding_error() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\n$target + \\verb|abc$\n<!-- blank: span hidden=\"target\" b:a1b2c3 -->\n"
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
                "## q <!-- id: {RTOK} -->\n---\n{line}\n<!-- blank: span hidden=\"{hidden}\" b:a1b2c3 -->\n"
            ));
            assert_eq!(vec![masked.to_string()], deck.cards[0].context, "{line}");
        }
    }

    #[test]
    fn an_authored_malformed_formula_under_a_span_is_a_loud_binding_error() {
        let line = r"$x^{2$";
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\n{line}\n<!-- blank: span hidden=\"x\" boundary=char b:a1b2c3 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("does not parse"), "{message}");
    }

    #[test]
    fn a_structurally_legal_mask_that_fails_the_renderer_is_a_loud_binding_error() {
        let error = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\n$x^2$\n<!-- blank: span hidden=\"2\" b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\n$\\gamma$\n<!-- input: type -->\n<!-- blank: span hidden=\"\\\\gamma\" b:a1b2c3 -->\n"
        ));
        assert_eq!(
            vec![LintKind::UntypableHole {
                answer: r"\gamma".to_string()
            }],
            deck.lints
                .iter()
                .map(|l| l.kind.clone())
                .collect::<Vec<_>>(),
            "keyboard pinned, command answer"
        );

        let contains = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\n$x \\leq y$\n<!-- input: type -->\n<!-- blank: span hidden=\"x \\\\leq y\" b:a1b2c3 -->\n"
        ));
        assert_eq!(
            1,
            contains
                .lints
                .iter()
                .filter(|l| matches!(l.kind, LintKind::UntypableHole { .. }))
                .count(),
            "a command anywhere in the hidden text is untypable: {:?}",
            contains.lints
        );

        let drawn = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\n$\\gamma$\n<!-- blank: span hidden=\"\\\\gamma\" b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\nprose here\n![lunate](lunate.png)\n<!-- blank: span hidden=\"lunate\" b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\nfill the ____ gap in prose\n<!-- blank: span hidden=\"prose\" b:a1b2c3 -->\n"
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

        let front = err("## q\npress ![gear](gear.png) now\n---\nanswer\n");
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
    fn occurrence_counting_skips_hole_answers_and_link_destinations() {
        let beside_hole = err(&format!(
            "## ports <!-- id: {RTOK} -->\n---\nSSH is \\blank{{22}}; HTTPS is 22/tcp\n<!-- cover: span hidden=\"22\" occurrence=2 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = beside_hole else {
            panic!("expected InvalidRegion, got {beside_hole:?}");
        };
        assert!(
            message.contains("occurs 1 time(s)"),
            "the hole's answer is invisible, so only the visible 22 counts: {message}"
        );

        let beside_link = err(&format!(
            "## q <!-- id: {RTOK} -->\n---\nsee [the RFC](https://rfc.example/22) for port 22\n<!-- blank: span hidden=\"22\" occurrence=2 b:a1b2c3 -->\n"
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
    fn prose_beside_holes_and_links_stays_blankable() {
        let beside_hole = parse(&format!(
            "## ports <!-- id: {RTOK} -->\n---\nSSH is \\blank{{22}}; HTTPS is 22/tcp\n<!-- cover: span hidden=\"22\" -->\n"
        ));
        assert!(
            beside_hole.cards.iter().all(|card| card.region.is_none()),
            "a cover makes no card; the cloze cards stand"
        );

        let beside_link = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nsee [the RFC](https://x) for port 22\n<!-- blank: span hidden=\"port\" b:a1b2c3 -->\n"
        ));
        assert_eq!(vec!["port"], beside_link.cards[0].back);

        let label = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nsee [the RFC](https://x) for port 22\n<!-- blank: span hidden=\"RFC\" b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\nsee [the RFC](https://x) now\n<!-- blank: span hidden=\"see the\" boundary=char b:a1b2c3 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("style boundary"), "{message}");
    }

    #[test]
    fn a_span_cards_context_masks_own_as_blank_and_siblings_as_hidden() {
        let deck = parse(&format!(
            "## anatomy <!-- id: {RTOK} -->\n---\nthe lunate sits beside the hamate bone\n<!-- blank: span hidden=\"lunate\" b:a1b2c3 -->\n<!-- blank: span hidden=\"hamate\" b:d4e5f6 -->\n<!-- cover: span hidden=\"bone\" -->\n"
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
            "## anatomy <!-- id: {RTOK} -->\n![](hand.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 b:a1b2c3 -->\n\n---\nthe lunate sits center\n<!-- blank: span hidden=\"lunate\" b:d4e5f6 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\n**New** York is big\n<!-- blank: span hidden=\"New\" b:a1b2c3 -->\n"
        ));
        assert_eq!(
            vec!["**⍰** York is big"],
            deck.cards[0].context,
            "the splice lands inside the markers, so the bold survives"
        );
    }

    #[test]
    fn a_group_card_blanks_every_member_in_context() {
        let deck = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nalpha then beta\n<!-- blank: span [pair] hidden=\"alpha\" b:a1b2c3 -->\n<!-- blank: span [pair] hidden=\"beta\" b:d4e5f6 -->\n"
        ));
        assert_eq!(1, deck.cards.len(), "one group card");
        assert_eq!(vec!["⍰ then ⍰"], deck.cards[0].context);
    }

    #[test]
    fn moving_a_span_to_another_occurrence_changes_the_fingerprint() {
        let one = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nport 22 forwards to port 22\n<!-- blank: span hidden=\"22\" b:a1b2c3 -->\n"
        ));
        let two = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nport 22 forwards to port 22\n<!-- blank: span hidden=\"22\" occurrence=2 b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\nthe lunate sits center\n![](hand.png)\n<!-- blank: span hidden=\"lunate\" b:a1b2c3 -->\n"
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
            "## q <!-- id: {RTOK} -->\n---\nNew York City Hall\n<!-- blank: span hidden=\"New York City Hall\" boundary=char b:a1b2c3 -->\n<!-- blank: span hidden=\"York City Hall\" boundary=char b:d4e5f6 -->\n"
        ));
        let ParseError::InvalidRegion { message, .. } = error else {
            panic!("expected InvalidRegion, got {error:?}");
        };
        assert!(message.contains("overlap"), "{message}");
    }

    #[test]
    fn a_cover_span_masks_answer_giving_prose_in_cloze_context() {
        let deck = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nthe legend says alpha; fill \\blank{{alpha}}\n<!-- cover: span hidden=\"alpha\" -->\n"
        ));
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(0), deck.cards[0].hole);
        assert_eq!(vec!["the legend says ⬚; fill ⍰"], deck.cards[0].context);
    }

    #[test]
    fn moving_a_cloze_cover_span_changes_the_fingerprint() {
        let first = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nalpha then alpha; fill \\blank{{x}}\n<!-- cover: span hidden=\"alpha\" -->\n"
        ));
        let second = parse(&format!(
            "## q <!-- id: {RTOK} -->\n---\nalpha then alpha; fill \\blank{{x}}\n<!-- cover: span hidden=\"alpha\" occurrence=2 -->\n"
        ));
        assert_ne!(
            first.cards[0].content_fingerprint,
            second.cards[0].content_fingerprint
        );
    }

    #[test]
    fn each_cloze_card_fingerprints_its_effective_masked_question() {
        let deck = parse("## q\n---\nfirst \\blank{alpha}; second \\blank{beta}\n");
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
    fn editing_a_hidden_sibling_preserves_the_unchanged_cloze_cards_fingerprint() {
        let before = parse("## q\n---\nfirst \\blank{alpha}; second \\blank{beta}\n");
        let after = parse("## q\n---\nfirst \\blank{alpha}; second \\blank{gamma}\n");
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
}
