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

pub use canonical::{canonical_content, content_fingerprint};
pub use cloze::{BLANK, HIDDEN};
use cloze::{Region, Seg, hash_repr, hole_fingerprints, scan_markers, seg_display};
pub use frontmatter::{DECK_FORMAT_VERSION, Frontmatter, parse_sampling, yaml_quote};
use frontmatter::{bad_value, closes_frontmatter, parse_frontmatter, parse_reveal};

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
    UnknownKey { key: String },
    BadValue { key: String, value: String },
    EmptyValue { key: String },
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
    #[error("line {0}: card front is empty")]
    EmptyFront(usize),
    #[error("line {0}: card front without an answer")]
    FrontWithoutAnswer(usize),
    #[error(
        "line {line}: `at:` is not a named-field locator (`at: <src>:<lines> fingerprint: xxh64-<hex> asset: <object>`): {message}; fields are `at:`, `fingerprint:`, `asset:`, in that order"
    )]
    InvalidLocator { line: usize, message: String },
    #[error("line {0}: `\\blank[` is reserved for a future per-hole pin; write `\\blank{{...}}`")]
    ClozeBracketReserved(usize),
    #[error("line {0}: unclosed cloze hole (missing the closing `}}`)")]
    UnclosedHole(usize),
    #[error("line {0}: empty cloze hole")]
    EmptyHole(usize),
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
            RawBlock::Card(raw) => build_card(&subject, &deck_id, raw, &mut cards, &mut lints)?,
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
            cursor = alt_end + 1;
            continue;
        };
        let Some((source, after)) = cloze::scan_src(paren) else {
            cursor = alt_end + 1;
            continue;
        };
        let consumed = paren.len() - after.len();
        let paren_start = alt_end + 2;
        let destination = if paren.starts_with('<') {
            paren_start + 1..paren_start + consumed.saturating_sub(2)
        } else {
            paren_start..paren_start + consumed.saturating_sub(1)
        };
        if destination.start <= destination.end && destination.end <= line.len() {
            out.push(ImageReference {
                source,
                destination: offset + destination.start..offset + destination.end,
            });
        }
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
    note: Option<String>,
    directives: CardDirectives,
}

struct RawTable {
    line: usize,
    title: Option<String>,
    columns: usize,
    header: Vec<String>,
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
            .find(|c| (*c as u32) < 0x20 && !WHITESPACE.contains(c))
        {
            return Err(ParseError::ControlChar {
                line: idx + 1,
                found: format!("U+{:04X}", ch as u32),
            });
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
                header,
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
                if let Some((key, value)) = directive(body)
                    && let Some(card) = current.as_mut()
                {
                    apply_directive(&mut card.directives, &key, value, lineno, lints)?;
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

fn build_table_cards(
    subject: &Arc<str>,
    deck_id: &Arc<str>,
    raw: RawTable,
    cards: &mut Vec<Card>,
) -> Result<(), ParseError> {
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
        card.context = raw.header.clone();
        if let Some(title) = &raw.title {
            card.context.insert(0, title.clone());
        }
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
                Some((_, None, None, false))
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
        }),
        Seg::Text(_) | Seg::Hole(_) => None,
    })
}

// The empty guard is load-bearing: an all-image line drops, but a blank content line (a fence's
// blank, which yields no segments) must stay.
fn image_only(segments: &[Seg]) -> bool {
    !segments.is_empty() && segments.iter().all(|s| matches!(s, Seg::Image { .. }))
}

fn build_card(
    subject: &Arc<str>,
    deck_id: &Arc<str>,
    raw: RawCard,
    cards: &mut Vec<Card>,
    lints: &mut Vec<Lint>,
) -> Result<(), ParseError> {
    let RawCard {
        line,
        front: heading,
        front_extra,
        back,
        divided,
        note,
        directives,
    } = raw;
    let mut images: Vec<CardImage> = Vec::new();
    let (front, answer) = if divided {
        let mut front_lines = vec![heading];
        for (lineno, text) in &front_extra {
            let segments = scan_markers(text, *lineno, Region::Front, lints)?;
            images.extend(card_images(&segments));
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
        parsed.push(scan_markers(text, *lineno, Region::Answer, lints)?);
    }
    let mut images_back: Vec<CardImage> = Vec::new();
    for segments in &parsed {
        images_back.extend(card_images(segments));
    }

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
                card.citations = directives.citations;
                card.givens = directives.givens;
                card.authored_distractors = distractors;
                cards.push(card);
                return Ok(());
            }
        }
    }

    let holes: Vec<(usize, usize, &str)> = parsed
        .iter()
        .enumerate()
        .flat_map(|(li, segments)| {
            segments
                .iter()
                .enumerate()
                .filter_map(move |(si, segment)| match segment {
                    Seg::Hole(h) => Some((li, si, h.as_str())),
                    Seg::Text(_) | Seg::Image { .. } => None,
                })
        })
        .collect();

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
        card.citations = directives.citations;
        card.givens = directives.givens;
        cards.push(card);
        return Ok(());
    }

    // A cloze card. `reveal:` is retired here: the holes are the trigger.
    if directives.reveal.is_some() {
        lints.push(Lint {
            line: directives.reveal_line.unwrap_or(line),
            kind: LintKind::RevealOnCloze,
        });
    }
    let token: Option<Arc<str>> = directives.token.as_deref().map(Arc::from);
    let structure: Vec<String> = parsed.iter().map(|segments| hash_repr(segments)).collect();
    // Raw (unmasked) answer lines, so `\blank{...}` markers count as literal
    // text and this can't collide with a plain card repeating the hidden text.
    let raw_answer: Vec<String> = answer.iter().map(|(_, text)| text.clone()).collect();
    let block_fingerprint = content_fingerprint(&front, &raw_answer);
    let block_holes = hole_fingerprints(&parsed, &holes);
    for (n, (hole_line, hole_seg, answer_text)) in holes.iter().enumerate() {
        let context: Vec<String> = parsed
            .iter()
            .enumerate()
            .filter(|(_, segments)| !image_only(segments))
            .map(|(li, segments)| {
                let mut rendered = String::new();
                for (si, segment) in segments.iter().enumerate() {
                    match segment {
                        Seg::Text(text) => rendered.push_str(text),
                        Seg::Hole(_) if li == *hole_line && si == *hole_seg => {
                            rendered.push_str(BLANK);
                        }
                        Seg::Hole(_) => rendered.push_str(HIDDEN),
                        Seg::Image { .. } => {}
                    }
                }
                rendered
            })
            .collect();
        let mut hash_lines = structure.clone();
        hash_lines.push(format!("#cloze:{n}"));
        let mut card = Card::plain(
            Arc::clone(subject),
            front.clone(),
            vec![answer_text.to_string()],
            note.clone(),
            line,
        );
        card.deck_id = Arc::clone(deck_id);
        card.context = context;
        card.hash_lines = Some(hash_lines);
        card.token = token.clone();
        card.hole = Some(n as u32);
        card.block_holes = block_holes.clone();
        card.images = images.clone();
        card.images_back = images_back.clone();
        card.content_fingerprint = block_fingerprint;
        // A cloze sub-card never reverses and keeps no direction: only the
        // per-card `input:` still applies here.
        card.input = directives.input;
        cards.push(card);
    }
    Ok(())
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
        assert_eq!(vec!["```", "let x = ____;", "```"], deck.cards[0].context);
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
        assert_eq!(vec!["the ____ fox", "jumps […]"], deck.cards[0].context);

        assert_eq!(Some(1), deck.cards[1].hole);
        assert_eq!(vec!["over"], deck.cards[1].back);
        assert_eq!(vec!["the […] fox", "jumps ____"], deck.cards[1].context);
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
    fn cloze_bracket_is_a_reserved_parse_error() {
        assert_eq!(
            ParseError::ClozeBracketReserved(3),
            err("## q\n---\na \\blank[x]{y} b\n")
        );
    }

    #[test]
    fn escaped_braces_inside_a_hole_are_stripped_and_do_not_count() {
        let deck = parse("## q\n---\nw \\blank{a \\{b\\} c} z\n");
        assert_eq!(1, deck.cards.len());
        assert_eq!(vec!["a {b} c"], deck.cards[0].back);
        assert_eq!(vec!["w ____ z"], deck.cards[0].context);

        let deck = parse("## q\n---\nw \\blank{a \\{b} c\n");
        assert_eq!(vec!["a {b"], deck.cards[0].back);
        assert_eq!(vec!["w ____ c"], deck.cards[0].context);
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
        let text =
            "---\ntitle: x\n---\n## q\n![one](images/Moon.PNG)\n![two](<with space/a.png>)\n";
        let references = image_references(text);

        assert_eq!(2, references.len());
        assert_eq!("images/Moon.PNG", references[0].source);
        assert_eq!("images/Moon.PNG", &text[references[0].destination.clone()]);
        assert_eq!("with space/a.png", references[1].source);
        assert_eq!("with space/a.png", &text[references[1].destination.clone()]);
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
        assert_eq!(vec!["word", "meaning"], first.context);
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
        assert_eq!(vec!["word", "meaning", "note"], deck.cards[0].context);
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
        assert_eq!(vec!["q", "a", "b"], deck.cards[0].context);
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
        assert_eq!(vec!["a", "b"], deck.cards[0].context);
        assert_eq!(vec!["c", "d"], deck.cards[1].context);
    }

    #[test]
    fn an_empty_heading_above_a_table_becomes_its_title() {
        let text = format!(
            "## Verbs of arguing\n| word | meaning |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n<!-- id: {CONTAINER} -->\n"
        );
        let deck = parse(&text);
        assert_eq!(1, deck.cards.len(), "the heading is a title, not a card");
        assert_eq!(
            vec!["Verbs of arguing", "word", "meaning"],
            deck.cards[0].context
        );
        assert_eq!(Some(format!("{CONTAINER}-t4k2x9w")), deck.cards[0].id());
    }

    #[test]
    fn a_heading_with_answer_content_before_a_table_stays_a_card() {
        let deck = parse("## q\nanswer\n| a | b |\n|---|---|\n| x | y |\n");
        assert_eq!(2, deck.cards.len());
        assert_eq!("q", deck.cards[0].front);
        assert_eq!(vec!["answer"], deck.cards[0].back);
        assert_eq!(
            vec!["a", "b"],
            deck.cards[1].context,
            "the table is untitled"
        );
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
        assert_eq!(vec!["Title", "a", "b"], deck.cards[0].context);
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
}
