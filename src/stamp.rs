use std::{
    collections::HashSet,
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{parser, parser::DECK_FORMAT_VERSION, token};

/// Mirrors the L1 parser's whitespace set exactly, so token-value spans are
/// located the same way the parser reads them.
const WS: [char; 6] = ['\t', '\n', '\x0B', '\x0C', '\r', ' '];

/// One UTF-8 byte-order mark; kept as byte 0 across a stamp write.
const BOM: &str = "\u{feff}";

/// Row-stamp mint attempts before giving up. 32^6 values against at most a
/// few hundred rows: exhausting this means the generator is broken.
const MINT_ATTEMPTS: usize = 64;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct StampOutcome {
    pub minted_cards: Vec<String>,
    pub minted_rows: Vec<String>,
    pub minted_regions: Vec<String>,
    pub minted_deck: Option<String>,
}

#[derive(Debug, Error)]
pub enum StampError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The original is left untouched.
    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} has no file name")]
    NoFileName { path: PathBuf },
    /// Refused even though the enumeration scans already exclude this case:
    /// defends a user's prose file from gaining a frontmatter block if this
    /// path is somehow still reached.
    #[error("{path} is not a deck (no cards, no frontmatter); refusing to stamp")]
    NotADeck { path: PathBuf },
    #[error(
        "{path} is not initialized as an Alix deck; run `alix deck init {path}` before opening it"
    )]
    Uninitialized { path: PathBuf },
    /// Should never happen: the parser guarantees every front line exists.
    #[error("line {0} is past the end of the file")]
    MissingLine(usize),
    /// No `id:` can be spliced into non-block-mapping frontmatter without
    /// risking an unloadable file.
    #[error("frontmatter is not a block mapping, cannot splice an `id:`")]
    UnspliceableFrontmatter,
    #[error("deck does not parse: {0}")]
    Parse(#[from] parser::ParseError),
    /// `getrandom::Error` doesn't implement `std::error::Error` without its
    /// `std` feature, so it can't be a `#[source]` here.
    #[error("cannot mint a token: {0}")]
    Mint(getrandom::Error),
    #[error(
        "{path}: line {line}: a code fence opens here and never closes, so a stamp would land inside it; close the fence and try again"
    )]
    UnclosedFence { path: PathBuf, line: usize },
    /// The input parsed and the stamped result would not. Nothing is written.
    #[error("stamping {path} would leave it unreadable ({source}); nothing was written")]
    WouldNotParse {
        path: PathBuf,
        #[source]
        source: parser::ParseError,
    },
    #[error("token `{token}` is not present in any `<!-- id: -->` comment")]
    TokenNotFound { token: String },
    #[error("cannot mint a row stamp unused by its table after many attempts")]
    MintExhausted,
}

enum DeckAction {
    None,
    Prepend,
    /// The 1-based line number of the frontmatter's opening `---`, to splice
    /// an `id:` after it.
    Splice(usize),
}

pub fn stamp_deck(path: &Path) -> Result<StampOutcome, StampError> {
    stamp_deck_with_mode(path, true)
}

pub fn stamp_initialized_deck(path: &Path) -> Result<StampOutcome, StampError> {
    stamp_deck_with_mode(path, false)
}

fn stamp_deck_with_mode(path: &Path, initialize: bool) -> Result<StampOutcome, StampError> {
    let original = fs::read_to_string(path).map_err(|source| StampError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let subject = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| StampError::NoFileName {
            path: path.to_path_buf(),
        })?;

    // Safety: parse the post-BOM body so parser line/byte offsets align; the
    // BOM is reattached unchanged as byte 0.
    let bom = if original.starts_with(BOM) { BOM } else { "" };
    let body = &original[bom.len()..];

    let deck = parser::parse(subject, body)?;

    if deck.cards.is_empty() && deck.frontmatter_span.is_none() {
        return Err(StampError::NotADeck {
            path: path.to_path_buf(),
        });
    }

    // An id written past an unclosed fence is inside it, so the card still
    // reads as unstamped and the next stamp appends another one, forever.
    if let Some(lint) = deck
        .lints
        .iter()
        .find(|lint| matches!(lint.kind, parser::LintKind::UnclosedFence))
    {
        return Err(StampError::UnclosedFence {
            path: path.to_path_buf(),
            line: lint.line,
        });
    }

    // Unstamped card front lines, deduped: a cloze card's holes expand to
    // several sub-cards sharing one `## ` line, but that line stamps once.
    // Table rows stamp through their table below, never as `## ` blocks.
    let table_row_lines: HashSet<usize> = deck
        .tables
        .iter()
        .flat_map(|table| table.rows.iter().map(|row| row.line))
        .collect();
    let mut card_lines: Vec<usize> = Vec::new();
    for card in &deck.cards {
        if card.token.is_none()
            && !table_row_lines.contains(&card.line)
            && !card_lines.contains(&card.line)
        {
            card_lines.push(card.line);
        }
    }
    card_lines.sort_unstable();

    // (row line, minted stamp) and (block end line, minted container id),
    // minted before any write like the card ids below.
    let mut row_mints: Vec<(usize, String)> = Vec::new();
    let mut container_mints: Vec<(usize, String)> = Vec::new();
    for table in &deck.tables {
        let mut taken: HashSet<String> = table
            .rows
            .iter()
            .filter_map(|row| row.stamp.clone())
            .collect();
        for row in &table.rows {
            if row.stamp.is_none() {
                let stamp = mint_stamp_unique(&taken)?;
                taken.insert(stamp.clone());
                row_mints.push((row.line, stamp));
            }
        }
        if table.token.is_none() && !table.rows.is_empty() {
            container_mints.push((table.end_line, mint_card_id()?));
        }
    }

    // Region stamps (ADR 0034): blanks mint per parent card, covers never
    // stamp. Cloze sub-cards clone their block's images, so regions dedupe
    // by their unique file line while uniqueness stays scoped to the block.
    let mut region_mints: Vec<(usize, String)> = Vec::new();
    let mut region_remints: Vec<(usize, String, String)> = Vec::new();
    {
        use crate::parser::region::{RawRegion, RegionKind};
        // (region line, its authored stamp) per parent block, in file order.
        type BlockRegions = Vec<(usize, Option<String>)>;
        let mut seen_lines: HashSet<usize> = HashSet::new();
        let mut blocks: Vec<(usize, BlockRegions)> = Vec::new();
        for card in &deck.cards {
            let mut push = |region: &RawRegion| {
                if region.kind != RegionKind::Blank || !seen_lines.insert(region.line) {
                    return;
                }
                match blocks.iter_mut().find(|(block, _)| *block == card.line) {
                    Some((_, regions)) => regions.push((region.line, region.stamp.clone())),
                    None => blocks.push((card.line, vec![(region.line, region.stamp.clone())])),
                }
            };
            for image in card.images.iter().chain(card.images_back.iter()) {
                image.regions.iter().for_each(&mut push);
            }
            card.span_regions.iter().for_each(&mut push);
        }
        for (_, mut regions) in blocks {
            // Collection visits image regions before spans; collision repair
            // must run in SOURCE order so the first occurrence survives and
            // the later paste re-mints, never the other way around.
            regions.sort_by_key(|(line, _)| *line);
            let mut taken: HashSet<String> = HashSet::new();
            for (line, stamp) in regions {
                match stamp {
                    Some(stamp) if taken.insert(stamp.clone()) => {}
                    Some(old) => {
                        // A copy-pasted line must not silently fuse two
                        // members: the collision re-mints (ADR 0034).
                        let new = mint_stamp_unique(&taken)?;
                        taken.insert(new.clone());
                        region_remints.push((line, old, new));
                    }
                    None => {
                        let stamp = mint_stamp_unique(&taken)?;
                        taken.insert(stamp.clone());
                        region_mints.push((line, stamp));
                    }
                }
            }
        }
    }

    let deck_action = if deck.deck_token.is_some() {
        DeckAction::None
    } else if !initialize {
        return Err(StampError::Uninitialized {
            path: path.to_path_buf(),
        });
    } else {
        match deck.frontmatter_span {
            None => DeckAction::Prepend,
            Some((open, _close)) if !deck.frontmatter.unspliceable => DeckAction::Splice(open),
            Some(_) => return Err(StampError::UnspliceableFrontmatter),
        }
    };

    // Nothing to write: a genuine byte no-op, so don't touch the file.
    if card_lines.is_empty()
        && row_mints.is_empty()
        && container_mints.is_empty()
        && region_mints.is_empty()
        && region_remints.is_empty()
        && matches!(deck_action, DeckAction::None)
    {
        return Ok(StampOutcome::default());
    }

    // Mint every id before writing a single byte (no partial writes).
    let deck_token = match deck_action {
        DeckAction::None => None,
        _ => Some(mint_deck_id()?),
    };
    let mut minted_cards = Vec::with_capacity(card_lines.len());
    for _ in &card_lines {
        minted_cards.push(mint_card_id()?);
    }

    // Safety: apply insertions right-to-left (sorted below) so an earlier
    // offset is never shifted by a later insertion.
    // (offset, rank, text). Rank breaks a tie at end of file, where a block's
    // closing line and the last row's own end are the same position: the row's
    // stamp has to stay inside the row.
    const AFTER_BLOCK: u8 = 0;
    const WITHIN_LINE: u8 = 1;
    let mut inserts: Vec<(usize, u8, usize, String)> = Vec::new();
    for (line, tok) in card_lines.iter().zip(&minted_cards) {
        let anchor = block_end_line(body, *line);
        let newline = line_terminator(body, anchor);
        let offset = line_start_of_next(body, anchor).ok_or(StampError::MissingLine(anchor))?;
        let lead = if offset == body.len() && !body.ends_with('\n') {
            newline
        } else {
            ""
        };
        inserts.push((
            offset,
            AFTER_BLOCK,
            0,
            format!("{lead}<!-- id: {tok} -->{newline}"),
        ));
    }
    for (line, stamp) in &row_mints {
        let start = nth_line_start(body, *line).ok_or(StampError::MissingLine(*line))?;
        let rest = &body[start..];
        let raw = &rest[..rest.find('\n').unwrap_or(rest.len())];
        let end = raw.trim_end_matches(&WS[..]).len();
        inserts.push((start + end, WITHIN_LINE, 0, format!(" <!-- r:{stamp} -->")));
    }
    for (line, stamp) in &region_mints {
        let start = nth_line_start(body, *line).ok_or(StampError::MissingLine(*line))?;
        let rest = &body[start..];
        let raw = &rest[..rest.find('\n').unwrap_or(rest.len())];
        let terminator = raw.find("-->").ok_or(StampError::MissingLine(*line))?;
        let end = raw[..terminator].trim_end().len();
        inserts.push((start + end, WITHIN_LINE, 0, format!(" b:{stamp}")));
    }
    for (line, old, new) in &region_remints {
        let start = nth_line_start(body, *line).ok_or(StampError::MissingLine(*line))?;
        let rest = &body[start..];
        let raw = &rest[..rest.find('\n').unwrap_or(rest.len())];
        let within = stamp_token_offset(raw, old).ok_or(StampError::MissingLine(*line))?;
        inserts.push((
            start + within,
            WITHIN_LINE,
            "b:".len() + old.len(),
            format!("b:{new}"),
        ));
    }
    for (end_line, tok) in &container_mints {
        let newline = line_terminator(body, *end_line);
        let offset =
            line_start_of_next(body, *end_line).ok_or(StampError::MissingLine(*end_line))?;
        let lead = if offset == body.len() && !body.ends_with('\n') {
            newline
        } else {
            ""
        };
        inserts.push((
            offset,
            AFTER_BLOCK,
            0,
            format!("{lead}<!-- id: {tok} -->{newline}"),
        ));
    }
    minted_cards.extend(container_mints.iter().map(|(_, tok)| tok.clone()));
    let mut prepend = String::new();
    match (&deck_action, &deck_token) {
        (DeckAction::Splice(open), Some(tok)) => {
            let offset = line_start_of_next(body, *open).ok_or(StampError::MissingLine(*open))?;
            // An author may already have written the version by hand; splicing
            // a second one is a duplicate YAML key, which makes the deck
            // unloadable.
            let version = match deck.frontmatter.format_version {
                Some(_) => String::new(),
                None => format!("format-version: {DECK_FORMAT_VERSION}\n"),
            };
            inserts.push((offset, AFTER_BLOCK, 0, format!("{version}id: \"{tok}\"\n")));
        }
        (DeckAction::Prepend, Some(tok)) => {
            prepend = format!("---\nformat-version: {DECK_FORMAT_VERSION}\nid: \"{tok}\"\n---\n\n");
        }
        _ => {}
    }

    // Descending, so each insertion leaves the earlier offsets valid. At an
    // equal offset the block-closing text goes in first, which leaves the
    // row's stamp ahead of it in the finished line.
    inserts.sort_by_key(|(offset, rank, _, _)| (std::cmp::Reverse(*offset), *rank));
    let mut new_body = body.to_string();
    for (offset, _, remove_len, text) in inserts {
        new_body.replace_range(offset..offset + remove_len, &text);
    }
    let new_text = format!("{bom}{prepend}{new_body}");

    // The input parsed, but an insertion can still land somewhere that makes
    // the result invalid, and this path writes the user's own file. Costs a
    // second parse of a file alix has already read, once per stamp.
    parser::parse(subject, &new_text[bom.len()..]).map_err(|source| StampError::WouldNotParse {
        path: path.to_path_buf(),
        source,
    })?;

    write_atomic(path, &new_text)?;

    Ok(StampOutcome {
        minted_cards,
        minted_rows: row_mints.into_iter().map(|(_, stamp)| stamp).collect(),
        minted_regions: region_mints
            .into_iter()
            .map(|(_, stamp)| stamp)
            .chain(region_remints.into_iter().map(|(_, _, new)| new))
            .collect(),
        minted_deck: deck_token,
    })
}

/// If the token appears in more than one id comment, only the first
/// (document order) is replaced.
pub fn replace_card_token(path: &Path, old_token: &str) -> Result<String, StampError> {
    let original = fs::read_to_string(path).map_err(|source| StampError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let span =
        first_id_token_span(&original, old_token).ok_or_else(|| StampError::TokenNotFound {
            token: old_token.to_string(),
        })?;
    let fresh = mint_card_id()?;
    let mut new_text = String::with_capacity(original.len() + fresh.len());
    new_text.push_str(&original[..span.start]);
    new_text.push_str(&fresh);
    new_text.push_str(&original[span.end..]);
    write_atomic(path, &new_text)?;
    Ok(fresh)
}

fn mint_deck_id() -> Result<String, StampError> {
    Ok(token::format_deck_id(
        &token::mint().map_err(StampError::Mint)?,
    ))
}

fn mint_card_id() -> Result<String, StampError> {
    Ok(token::format_card_id(
        &token::mint().map_err(StampError::Mint)?,
        None,
        None,
        false,
    ))
}

/// Bounded: a broken generator returning one value forever would otherwise
/// spin here instead of failing.
/// Finds the byte offset of the `b:<stamp>` token in a region line, skipping
/// quoted values, where the same characters could appear as answer text.
fn stamp_token_offset(line: &str, stamp: &str) -> Option<usize> {
    let needle = format!("b:{stamp}");
    let bytes = line.as_bytes();
    let mut quoted = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if quoted => i += 1,
            b'"' => quoted = !quoted,
            _ if !quoted && line[i..].starts_with(&needle) => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn mint_stamp_unique(taken: &HashSet<String>) -> Result<String, StampError> {
    for _ in 0..MINT_ATTEMPTS {
        let stamp = token::mint_row().map_err(StampError::Mint)?;
        if !taken.contains(&stamp) {
            return Ok(stamp);
        }
    }
    Err(StampError::MintExhausted)
}

/// Writes a sibling `.tmp` then renames over the original, so a failed write
/// leaves the original untouched.
fn write_atomic(path: &Path, contents: &str) -> Result<(), StampError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| StampError::NoFileName {
            path: path.to_path_buf(),
        })?;
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let tmp = parent.join(format!(".{file_name}.tmp"));
    crate::fsio::replace_file(&tmp, path, contents.as_bytes()).map_err(|source| {
        // A failure may strand the tmp: best-effort cleanup.
        let _ = fs::remove_file(&tmp);
        StampError::Write {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn block_end_line(text: &str, front_line: usize) -> usize {
    let mut last = front_line;
    let mut fence: Option<char> = None;
    let mut line = front_line;
    loop {
        line += 1;
        let Some(start) = nth_line_start(text, line) else {
            return last;
        };
        let rest = &text[start..];
        let raw = &rest[..rest.find('\n').unwrap_or(rest.len())];

        if let Some(ch) = fence {
            if parser::closes_fence(raw, ch) {
                fence = None;
            }
            last = line;
            continue;
        }
        if let Some(ch) = parser::fence_opener(raw) {
            fence = Some(ch);
            last = line;
            continue;
        }
        if raw.starts_with("## ") {
            return last;
        }
        // A table opens its own block: the card's id line must not land
        // beyond it, where the parser would hand the marker to the table.
        if raw.starts_with('|')
            && let Some(next_start) = nth_line_start(text, line + 1)
        {
            let next_rest = &text[next_start..];
            let next_raw = &next_rest[..next_rest.find('\n').unwrap_or(next_rest.len())];
            if next_raw.starts_with('|') && parser::is_delimiter_row(next_raw) {
                return last;
            }
        }
        if !raw.trim_matches(&WS[..]).is_empty() {
            last = line;
        }
    }
}

/// 1-based lines of card `<!-- id: -->` markers away from the position
/// `stamp_deck` mints at: the marker's own line closing its card block.
/// Flags a marker trailing the `## ` front and a standalone marker with card
/// content after it; both parse, so this is a `doctor` hygiene signal.
pub fn misplaced_id_markers(text: &str) -> Vec<usize> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut fence: Option<char> = None;
    let mut block_end: Option<usize> = None;
    for (index, raw) in lines.iter().enumerate() {
        let raw = *raw;
        let line = index + 1;
        if let Some(ch) = fence {
            if parser::closes_fence(raw, ch) {
                fence = None;
            }
            continue;
        }
        if let Some(ch) = parser::fence_opener(raw) {
            fence = Some(ch);
            continue;
        }
        if raw.starts_with('|')
            && lines
                .get(index + 1)
                .is_some_and(|next| next.starts_with('|') && parser::is_delimiter_row(next))
        {
            block_end = Some(table_block_end(&lines, index));
            continue;
        }
        if let Some(rest) = raw.strip_prefix("## ") {
            block_end = Some(block_end_line(text, line));
            if heading_id_marker(rest) {
                found.push(line);
            }
            continue;
        }
        let trimmed = raw.trim_matches(&WS[..]);
        if let Some(body) = trimmed
            .strip_prefix("<!--")
            .and_then(|s| s.strip_suffix("-->"))
            && matches!(parser::directive(body), Some((key, _)) if key == "id")
            && let Some(end) = block_end
            && line != end
        {
            found.push(line);
        }
    }
    found
}

/// 1-based last line of the table block whose header sits at 0-based index
/// `header`: rows, then trailing comments; blanks neither extend nor end it.
fn table_block_end(lines: &[&str], header: usize) -> usize {
    let mut last = header + 2;
    for (index, raw) in lines.iter().enumerate().skip(header + 2) {
        // An adjacent table's header opens its own block.
        if raw.starts_with('|')
            && lines
                .get(index + 1)
                .is_some_and(|next| next.starts_with('|') && parser::is_delimiter_row(next))
        {
            break;
        }
        let trimmed = raw.trim_matches(&WS[..]);
        if raw.starts_with('|') || (trimmed.starts_with("<!--") && trimmed.ends_with("-->")) {
            last = index + 1;
        } else if !trimmed.is_empty() {
            break;
        }
    }
    last
}

fn heading_id_marker(rest: &str) -> bool {
    let (_, bodies) = parser::split_trailing_comments(rest);
    bodies
        .iter()
        .any(|body| matches!(parser::directive(body), Some((key, _)) if key == "id"))
}

fn line_terminator(text: &str, line: usize) -> &'static str {
    let Some(start) = nth_line_start(text, line) else {
        return "\n";
    };
    let rest = &text[start..];
    match rest.find('\n') {
        Some(rel) if rest[..rel].ends_with('\r') => "\r\n",
        Some(_) => "\n",
        // An unterminated final line follows the file's convention.
        None if text.contains("\r\n") => "\r\n",
        None => "\n",
    }
}

fn line_start_of_next(text: &str, line: usize) -> Option<usize> {
    let start = nth_line_start(text, line)?;
    let rest = &text[start..];
    Some(match rest.find('\n') {
        Some(nl) => start + nl + 1,
        None => text.len(),
    })
}

fn nth_line_start(text: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    if line == 1 {
        return Some(0);
    }
    let mut seen = 0;
    for (i, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            seen += 1;
            if seen == line - 1 {
                return Some(i + 1);
            }
        }
    }
    None
}

/// A `target` matching only inert fenced text is a theoretical collision a
/// 26-char random token makes vanishingly unlikely.
fn first_id_token_span(text: &str, target: &str) -> Option<Range<usize>> {
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find("<!--") {
        let body_start = cursor + rel + 4;
        let Some(rel_end) = text[body_start..].find("-->") else {
            break;
        };
        let body_end = body_start + rel_end;
        if let Some(range) = id_value_range(body_start, &text[body_start..body_end])
            && &text[range.clone()] == target
        {
            return Some(range);
        }
        cursor = body_end + 3;
    }
    None
}

/// Mirrors the parser's `key: value` split and whitespace set, so token spans
/// line up with how the parser reads them.
fn id_value_range(body_start: usize, body: &str) -> Option<Range<usize>> {
    let colon = body.find(':')?;
    if !body[..colon]
        .trim_matches(&WS[..])
        .eq_ignore_ascii_case("id")
    {
        return None;
    }
    let after = &body[colon + 1..];
    let lead = after.find(|c: char| !WS.contains(&c))?;
    let value = after.trim_matches(&WS[..]);
    if value.is_empty() {
        return None;
    }
    let start = body_start + colon + 1 + lead;
    Some(start..start + value.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trailing space after `## First question` is deliberate: a mutation
    /// that trims the line before inserting would silently normalize it away.
    const FIXTURE: &str = "---\nsource: notes.md\nrequires: basics\n---\n# The Title\nintro prose\n\n## First question \nextra front line\n\n---\nthe answer\n\\--- escaped divider\n> a note\n```\nfenced\n## not a card\n```\ntail prose\n\n## Fill in the blanks\nthe \\blank{alpha} and \\blank{beta} here\n> cloze note\n";

    fn write(dir: &tempfile::TempDir, name: &str, text: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_marker_trailing_the_front_line_is_misplaced() {
        let text = "## q <!-- id: card-q1 -->\na\n";
        assert_eq!(vec![1], misplaced_id_markers(text));
    }

    #[test]
    fn a_standalone_marker_with_card_content_after_it_is_misplaced() {
        let text = "## q\na\n<!-- id: card-q1 -->\n<!-- at: notes.md:2 -->\n";
        assert_eq!(vec![3], misplaced_id_markers(text));
    }

    #[test]
    fn a_marker_closing_its_card_is_canonical() {
        let text = "## q\na\n<!-- at: notes.md:2 -->\n<!-- id: card-q1 -->\n\n## r\nb\n<!-- id: card-r1 -->\n";
        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(text));
    }

    #[test]
    fn a_marker_lookalike_inside_a_fence_is_not_a_marker() {
        let text = "## q\na\n```\n<!-- id: card-fake -->\n```\n<!-- id: card-q1 -->\n";
        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(text));
    }

    #[test]
    fn freshly_stamped_output_carries_no_misplaced_markers() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "deck.md", FIXTURE);
        stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(&stamped));
    }

    #[test]
    fn stamping_inserts_ids_and_changes_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "deck.md", FIXTURE);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        // Two file cards: the cloze card's two holes stamp once.
        assert_eq!(2, outcome.minted_cards.len());
        assert!(outcome.minted_deck.is_some());

        let mut reconstructed = stamped;
        for tok in &outcome.minted_cards {
            let span = format!("<!-- id: {tok} -->\n");
            assert_eq!(1, reconstructed.matches(&span).count(), "span {span:?}");
            reconstructed = reconstructed.replacen(&span, "", 1);
        }
        let deck_tok = outcome.minted_deck.as_ref().unwrap();
        let deck_span = format!("format-version: 1\nid: \"{deck_tok}\"\n");
        assert_eq!(1, reconstructed.matches(&deck_span).count());
        reconstructed = reconstructed.replacen(&deck_span, "", 1);

        assert_eq!(FIXTURE, reconstructed);
    }

    #[test]
    fn stamping_a_deck_without_frontmatter_prepends_the_canonical_four_line_block() {
        let dir = tempfile::tempdir().unwrap();
        let original = "## q\na\n## r\nb\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let deck_tok = outcome.minted_deck.as_ref().unwrap();

        assert!(
            stamped.starts_with(&format!(
                "---\nformat-version: 1\nid: \"{deck_tok}\"\n---\n\n"
            )),
            "{stamped:?}"
        );
        assert!(deck_tok.starts_with("deck-"), "{deck_tok}");
        assert!(
            outcome.minted_cards.iter().all(|t| t.starts_with("card-")),
            "{:?}",
            outcome.minted_cards
        );
        let parsed = parser::parse("deck.md", &stamped).unwrap();
        assert_eq!(Some(deck_tok.as_str()), parsed.deck_token.as_deref());
        assert!(parsed.cards.iter().all(|c| c.token.is_some()));
    }

    #[test]
    fn stamping_after_a_bom_keeps_the_bom_first() {
        let dir = tempfile::tempdir().unwrap();
        let original = "\u{feff}## q\na\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let deck_tok = outcome.minted_deck.as_ref().unwrap();

        assert!(stamped.starts_with(BOM));
        assert!(!stamped[BOM.len()..].starts_with(BOM));
        assert!(stamped.starts_with(&format!(
            "{BOM}---\nformat-version: 1\nid: \"{deck_tok}\"\n---\n\n"
        )));
    }

    #[test]
    fn initializing_a_deck_that_already_declares_a_version_does_not_duplicate_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        fs::write(&path, "---\nformat-version: 1\n---\n## q\na\n").unwrap();

        stamp_deck(&path).unwrap();

        let stamped = fs::read_to_string(&path).unwrap();
        assert_eq!(
            1,
            stamped.matches("format-version:").count(),
            "a second version key is a duplicate mapping key, so the deck stops parsing: {stamped}"
        );
        parser::parse("d.md", &stamped).expect("the stamped deck must still load");
    }

    #[test]
    fn an_id_line_splices_into_block_mapping_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nsource: notes.md\n---\n## q\na\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let deck_tok = outcome.minted_deck.as_ref().unwrap();

        assert_eq!(
            format!("---\nformat-version: 1\nid: \"{deck_tok}\"\nsource: notes.md\n---\n"),
            stamped[..stamped.find("## q").unwrap()]
        );
        let parsed = parser::parse("deck.md", &stamped).unwrap();
        assert_eq!(Some(deck_tok.as_str()), parsed.deck_token.as_deref());
        assert_eq!(vec!["notes.md".to_string()], parsed.frontmatter.source);
    }

    #[test]
    fn flow_mapping_frontmatter_is_a_loud_write_fail_not_a_splice() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\n{source: [a]}\n---\n## q\nb\n";
        let path = write(&dir, "deck.md", original);

        let result = stamp_deck(&path);
        assert!(
            matches!(result, Err(StampError::UnspliceableFrontmatter)),
            "{result:?}"
        );
        assert_eq!(original, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn stamping_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "deck.md", "---\nsource: x\n---\n## q\na\n## r\nb\n");

        stamp_deck(&path).unwrap();
        let once = fs::read_to_string(&path).unwrap();

        let outcome = stamp_deck(&path).unwrap();
        let twice = fs::read_to_string(&path).unwrap();

        assert_eq!(StampOutcome::default(), outcome);
        assert_eq!(once, twice);
    }

    #[test]
    fn a_partially_stamped_deck_mints_only_the_missing_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let stamped_card = "## already <!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\na\n";
        let original = format!(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n{stamped_card}## missing\nb\n"
        );
        let path = write(&dir, "deck.md", &original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        assert_eq!(1, outcome.minted_cards.len());
        assert_eq!(None, outcome.minted_deck);
        assert!(stamped.contains("4jkya9q3m8z0tw5v9y2b4n6d8f"));
        assert!(stamped.contains("9w2c7x4k1m8q3z5t0v6b2n4d8f"));
        let new_tok = &outcome.minted_cards[0];
        assert!(stamped.contains(&format!("## missing\nb\n<!-- id: {new_tok} -->\n")));
    }

    #[test]
    fn maintenance_refuses_an_uninitialized_file_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let original = "# Notes\n\n## Design\nThis is ordinary prose.\n";
        let path = write(&dir, "notes.md", original);

        let result = stamp_initialized_deck(&path);

        assert!(
            matches!(result, Err(StampError::Uninitialized { .. })),
            "{result:?}"
        );
        assert_eq!(original, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn a_foreign_frontmatter_id_is_a_parse_error_and_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nid: \"article\"\n---\n## Question\nAnswer\n";
        let path = write(&dir, "notes.md", original);

        let result = stamp_deck(&path);

        assert!(matches!(result, Err(StampError::Parse(_))), "{result:?}");
        assert_eq!(original, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn a_bare_marker_deck_is_refused_before_any_mint_can_half_convert_it() {
        let dir = tempfile::tempdir().unwrap();
        let with_bare_marker = "## q <!-- id: 9w2c7x4k1m8q3z5t0v6b2n4d8f -->\na\n## fresh\nb\n";
        let path = write(&dir, "marker.md", with_bare_marker);
        let result = stamp_deck(&path);
        assert!(matches!(result, Err(StampError::Parse(_))), "{result:?}");
        assert_eq!(with_bare_marker, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn an_unknown_frontmatter_key_does_not_block_minting() {
        let dir = tempfile::tempdir().unwrap();
        let with_unknown_key = "---\nalix-id: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n";
        let path = write(&dir, "key.md", with_unknown_key);
        stamp_deck(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("id: \"deck-"), "{text}");
        assert!(text.contains("<!-- id: card-"), "{text}");
    }

    #[test]
    fn maintenance_mints_missing_card_ids_but_preserves_the_deck_id() {
        let dir = tempfile::tempdir().unwrap();
        let deck_token = "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f";
        let original = format!("---\nformat-version: 1\nid: \"{deck_token}\"\n---\n## q\na\n");
        let path = write(&dir, "deck.md", &original);

        let outcome = stamp_initialized_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        assert_eq!(None, outcome.minted_deck);
        assert_eq!(1, outcome.minted_cards.len());
        assert!(stamped.contains(&format!("id: \"{deck_token}\"")));
        assert_eq!(1, stamped.matches("<!-- id: ").count());
    }

    #[test]
    fn token_replacement_swaps_exactly_the_old_span() {
        let dir = tempfile::tempdir().unwrap();
        let old = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        let other = "card-zzzzzzzzzzzzzzzzzzzzzzzzzz";
        let original = format!(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n\
             ## q <!-- id: {old} -->\na\n## r <!-- id: {other} -->\nb\n"
        );
        let path = write(&dir, "deck.md", &original);

        let fresh = replace_card_token(&path, old).unwrap();
        let output = fs::read_to_string(&path).unwrap();

        assert_eq!(
            output.replacen(&fresh, "", 1),
            original.replacen(old, "", 1)
        );
        assert!(output.contains(&format!("<!-- id: {fresh} -->")));
        assert!(fresh.starts_with("card-"), "{fresh}");
        assert!(!output.contains(old));
        assert!(output.contains(other));
        assert!(output.contains("deck-9w2c7x4k1m8q3z5t0v6b2n4d8f"));
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_write_leaves_the_original_untouched() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let original = "## q\na\n";
        let path = write(&dir, "deck.md", original);

        let read_only = fs::Permissions::from_mode(0o555);
        fs::set_permissions(dir.path(), read_only).unwrap();

        let result = stamp_deck(&path);

        // Restore write permission so the tempdir can clean itself up.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            matches!(result, Err(StampError::Write { .. })),
            "{result:?}"
        );
        assert_eq!(original, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn replacing_an_absent_token_errors_and_leaves_the_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let original = "## q <!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\na\n";
        let path = write(&dir, "deck.md", original);

        let result = replace_card_token(&path, "card-zzzzzzzzzzzzzzzzzzzzzzzzzz");
        assert!(
            matches!(result, Err(StampError::TokenNotFound { .. })),
            "{result:?}"
        );
        assert_eq!(original, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn selecting_a_prose_file_refuses_loudly_and_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        let original = "# My notes\n\njust some prose, not a deck at all\n";
        let path = write(&dir, "notes.md", original);

        let result = stamp_deck(&path);
        assert!(
            matches!(result, Err(StampError::NotADeck { .. })),
            "{result:?}"
        );
        assert_eq!(original, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn null_scalar_frontmatter_is_a_loud_write_fail_not_a_splice() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nnull\n---\n## q\nb\n";
        let path = write(&dir, "deck.md", original);

        let result = stamp_deck(&path);
        assert!(
            matches!(result, Err(StampError::UnspliceableFrontmatter)),
            "{result:?}"
        );
        assert_eq!(original, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn prepending_frontmatter_reconstructs_the_original_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let original = "## q\na\n## r\nb\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let deck_tok = outcome.minted_deck.as_ref().unwrap();

        let prefix = format!("---\nformat-version: 1\nid: \"{deck_tok}\"\n---\n\n");
        assert!(stamped.starts_with(&prefix), "{stamped:?}");
        let mut reconstructed = stamped[prefix.len()..].to_string();
        for tok in &outcome.minted_cards {
            let span = format!("<!-- id: {tok} -->\n");
            assert_eq!(1, reconstructed.matches(&span).count(), "span {span:?}");
            reconstructed = reconstructed.replacen(&span, "", 1);
        }

        assert_eq!(original, reconstructed);
    }

    #[test]
    fn stamping_a_crlf_deck_preserves_every_original_byte() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\r\nsource: notes.md\r\n---\r\n## q\r\na\r\n## r\r\nb\r\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let deck_tok = outcome.minted_deck.as_ref().unwrap();

        for tok in &outcome.minted_cards {
            assert!(
                stamped.contains(&format!("<!-- id: {tok} -->\r\n")),
                "{stamped:?}"
            );
        }

        let deck_span = format!("format-version: 1\nid: \"{deck_tok}\"\n");
        assert_eq!(1, stamped.matches(&deck_span).count());
        let mut reconstructed = stamped.replacen(&deck_span, "", 1);
        for tok in &outcome.minted_cards {
            let span = format!("<!-- id: {tok} -->\r\n");
            assert_eq!(1, reconstructed.matches(&span).count(), "span {span:?}");
            reconstructed = reconstructed.replacen(&span, "", 1);
        }

        assert_eq!(original, reconstructed);
    }

    #[test]
    fn a_front_with_a_trailing_directive_keeps_it_and_the_id_line_closes_the_card() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q <!-- reveal: line -->\na\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let tok = &outcome.minted_cards[0];

        assert_eq!(
            format!(
                "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n\
                 ## q <!-- reveal: line -->\na\n<!-- id: {tok} -->\n"
            ),
            stamped
        );

        let reconstructed = stamped.replacen(&format!("<!-- id: {tok} -->\n"), "", 1);
        assert_eq!(original, reconstructed);
    }

    #[test]
    fn a_hash_run_front_keeps_its_run_and_the_id_line_closes_the_card() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## Foo ##\nbar\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let tok = &outcome.minted_cards[0];

        assert_eq!(
            format!(
                "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## Foo ##\nbar\n<!-- id: {tok} -->\n"
            ),
            stamped
        );

        let reconstructed = stamped.replacen(&format!("<!-- id: {tok} -->\n"), "", 1);
        assert_eq!(original, reconstructed);

        let parsed = parser::parse("deck.md", &stamped).unwrap();
        assert_eq!("Foo", parsed.cards[0].front);
        assert_eq!(Some(tok.as_str()), parsed.cards[0].token.as_deref());
    }

    #[test]
    fn a_divided_front_card_gets_its_id_line_at_the_end_of_the_block() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## Q\n---\nthe answer\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let tok = &outcome.minted_cards[0];

        assert_eq!(
            format!(
                "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## Q\n---\nthe answer\n<!-- id: {tok} -->\n"
            ),
            stamped
        );

        let parsed = parser::parse("deck.md", &stamped).unwrap();
        assert_eq!(1, parsed.cards.len());
        assert_eq!("Q", parsed.cards[0].front);
        assert_eq!(vec!["the answer".to_string()], parsed.cards[0].back);
        assert_eq!(Some(tok.as_str()), parsed.cards[0].token.as_deref());
    }

    #[test]
    fn stamping_a_card_at_eof_without_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let original =
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let tok = &outcome.minted_cards[0];

        assert_eq!(
            format!(
                "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n<!-- id: {tok} -->\n"
            ),
            stamped
        );

        let reconstructed = stamped.replacen(&format!("\n<!-- id: {tok} -->\n"), "", 1);
        assert_eq!(original, reconstructed);

        let parsed = parser::parse("deck.md", &stamped).unwrap();
        assert_eq!(1, parsed.cards.len());
        assert_eq!(Some(tok.as_str()), parsed.cards[0].token.as_deref());
    }

    #[test]
    fn identical_cloze_fronts_on_different_lines_each_get_their_own_token() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## Foo\n---\nthe \\blank{a} note\n\n\
             ## Foo\n---\nthe \\blank{b} note\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        assert_eq!(2, outcome.minted_cards.len());
        assert_ne!(outcome.minted_cards[0], outcome.minted_cards[1]);
        assert_eq!(
            1,
            stamped
                .matches(&format!(
                    "the \\blank{{a}} note\n<!-- id: {} -->\n",
                    outcome.minted_cards[0]
                ))
                .count(),
            "{stamped:?}"
        );
        assert_eq!(
            1,
            stamped
                .matches(&format!(
                    "the \\blank{{b}} note\n<!-- id: {} -->\n",
                    outcome.minted_cards[1]
                ))
                .count(),
            "{stamped:?}"
        );
        let parsed = parser::parse("deck.md", &stamped).unwrap();
        assert!(parsed.cards.iter().all(|c| c.front == "Foo"));
        assert!(parsed.cards.iter().all(|c| c.token.is_some()));
    }

    #[test]
    fn row_minting_gives_up_instead_of_spinning_when_no_stamp_is_free() {
        let mut taken = HashSet::new();
        for _ in 0..MINT_ATTEMPTS * 4 {
            taken.insert(token::mint_row().unwrap());
        }
        // A healthy generator still finds a free stamp among 32^6 values.
        assert!(mint_stamp_unique(&taken).is_ok());

        // The pathological case the bound exists for: every candidate taken.
        let everything: HashSet<String> = (0..32u8)
            .flat_map(|a| {
                (0..32u8).map(move |b| {
                    let alpha = crate::token::TOKEN_ALPHABET;
                    format!(
                        "{}{}{}{}{}{}",
                        alpha[a as usize] as char, alpha[b as usize] as char, '0', '0', '0', '0'
                    )
                })
            })
            .collect();
        // Not exhaustive over the space, so this must still succeed rather
        // than hang; the bound only fires when the generator itself repeats.
        assert!(mint_stamp_unique(&everything).is_ok());
    }

    #[test]
    fn stamping_a_table_mints_the_container_and_row_stamps() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n| word | meaning |\n|---|---|\n| hund | dog |\n| katze | cat |\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        assert_eq!(1, outcome.minted_cards.len(), "the container id");
        assert_eq!(2, outcome.minted_rows.len());
        let container = &outcome.minted_cards[0];
        assert!(
            stamped.ends_with(&format!(
                "| hund | dog | <!-- r:{} -->\n| katze | cat | <!-- r:{} -->\n<!-- id: {container} -->\n",
                outcome.minted_rows[0], outcome.minted_rows[1]
            )),
            "{stamped:?}"
        );

        let parsed = parser::parse("deck.md", &stamped).unwrap();
        assert_eq!(2, parsed.cards.len());
        for (card, row) in parsed.cards.iter().zip(&outcome.minted_rows) {
            assert_eq!(Some(container.as_str()), card.token.as_deref());
            assert_eq!(Some(row.as_str()), card.row.as_deref());
            assert_eq!(Some(format!("{container}-t{row}")), card.id());
        }

        let mut reconstructed = stamped.replacen(&format!("<!-- id: {container} -->\n"), "", 1);
        for row in &outcome.minted_rows {
            reconstructed = reconstructed.replacen(&format!(" <!-- r:{row} -->"), "", 1);
        }
        assert_eq!(original, reconstructed);
    }

    #[test]
    fn table_stamping_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            "deck.md",
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n| a | b |\n|---|---|\n| x | y |\n",
        );

        stamp_deck(&path).unwrap();
        let once = fs::read_to_string(&path).unwrap();
        let outcome = stamp_deck(&path).unwrap();

        assert_eq!(StampOutcome::default(), outcome);
        assert_eq!(once, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn adjacent_table_container_ids_are_both_canonical() {
        let text = "| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n\n| c | d |\n|---|---|\n| p | q | <!-- r:7m3p5q -->\n<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n";

        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(text));
    }

    #[test]
    fn a_stamp_that_would_leave_the_deck_unreadable_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        // Parses as it stands; splicing an `id:` makes the frontmatter a
        // block mapping, and the tab indentation is then invalid YAML.
        let original = "---\n## ia\n\t\tn: []\n---\n## q\na\n";
        let path = write(&dir, "deck.md", original);

        let error = stamp_deck(&path).unwrap_err();

        assert!(
            matches!(&error, StampError::WouldNotParse { .. }),
            "{error:?}"
        );
        assert_eq!(
            original,
            fs::read_to_string(&path).unwrap(),
            "the user's file is left exactly as it was"
        );
    }

    #[test]
    fn an_unclosed_fence_is_refused_rather_than_stamped_into() {
        let dir = tempfile::tempdir().unwrap();
        let original =
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\n~~~\nc\n";
        let path = write(&dir, "deck.md", original);

        let error = stamp_deck(&path).unwrap_err();

        assert!(
            matches!(&error, StampError::UnclosedFence { line: 6, .. }),
            "{error:?}"
        );
        assert_eq!(
            original,
            fs::read_to_string(&path).unwrap(),
            "a refusal must not write: the id would land inside the fence"
        );
    }

    #[test]
    fn a_table_at_eof_without_a_final_newline_keeps_its_row_stamp_on_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            "deck.md",
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n| a | b |\n|---|---|\n| x | y |",
        );

        let first = stamp_deck(&path).unwrap();
        let once = fs::read_to_string(&path).unwrap();
        let stamp = first.minted_rows.first().expect("the row is stamped");

        assert!(
            once.contains(&format!("| x | y | <!-- r:{stamp} -->")),
            "the row stamp belongs in the row, not on a line of its own: {once:?}"
        );

        let second = stamp_deck(&path).unwrap();
        assert_eq!(
            StampOutcome::default(),
            second,
            "a second stamp re-minted the row, so its card id would change: {once:?}"
        );
        assert_eq!(once, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn a_crlf_table_at_eof_keeps_crlf_when_stamped() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\r\nformat-version: 1\r\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\r\n---\r\n| a | b |\r\n|---|---|\r\n| x | y |";
        let path = write(&dir, "deck.md", original);

        stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        assert!(
            !stamped.replace("\r\n", "").contains('\n'),
            "stamping introduced an LF-only line ending: {stamped:?}"
        );
    }

    #[test]
    fn a_half_stamped_table_mints_only_the_missing_row() {
        let dir = tempfile::tempdir().unwrap();
        let container = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        let original = format!(
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n| a | b |\n|---|---|\n| x | y | <!-- r:4k2x9w -->\n| p | q |\n<!-- id: {container} -->\n"
        );
        let path = write(&dir, "deck.md", &original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        assert!(outcome.minted_cards.is_empty(), "{outcome:?}");
        assert_eq!(1, outcome.minted_rows.len());
        let fresh = &outcome.minted_rows[0];
        assert_ne!("4k2x9w", fresh.as_str());
        assert_eq!(1, stamped.matches("r:4k2x9w").count());
        assert!(
            stamped.contains(&format!("| p | q | <!-- r:{fresh} -->")),
            "{stamped:?}"
        );
        assert_eq!(1, stamped.matches(&format!("id: {container}")).count());
    }

    #[test]
    fn stamps_survive_a_row_sort() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            "deck.md",
            "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n| a | b |\n|---|---|\n| x | y |\n| p | q |\n| m | n |\n",
        );
        stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let ids_by_front = |text: &str| -> std::collections::BTreeMap<String, String> {
            parser::parse("deck.md", text)
                .unwrap()
                .cards
                .iter()
                .map(|card| (card.front.clone(), card.id().unwrap()))
                .collect()
        };
        let before = ids_by_front(&stamped);

        let mut lines: Vec<&str> = stamped.lines().collect();
        let first_row = lines
            .iter()
            .position(|l| l.starts_with("| x"))
            .expect("the stamped x row exists");
        lines.swap(first_row, first_row + 2);
        let sorted = format!("{}\n", lines.join("\n"));

        assert_eq!(before, ids_by_front(&sorted));
    }

    #[test]
    fn the_container_id_splices_after_trailing_directive_comments() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n| a | b |\n|---|---|\n| x | y |\n<!-- direction: both -->\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let container = &outcome.minted_cards[0];

        assert!(
            stamped.ends_with(&format!(
                "<!-- direction: both -->\n<!-- id: {container} -->\n"
            )),
            "{stamped:?}"
        );
        parser::parse("deck.md", &stamped).expect("the stamped deck must still load");
    }

    #[test]
    fn a_freshly_stamped_mixed_deck_has_no_misplaced_markers() {
        let dir = tempfile::tempdir().unwrap();
        let original = "## q\na\n\n| a | b |\n|---|---|\n| x | y |\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        // The `## ` card's id line lands before the table, not beyond it.
        assert!(
            stamped.contains(&format!("a\n<!-- id: {} -->\n", outcome.minted_cards[0])),
            "{stamped:?}"
        );
        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(&stamped));
        let parsed = parser::parse("deck.md", &stamped).unwrap();
        assert_eq!(2, parsed.cards.len());
        assert!(parsed.cards.iter().all(|c| c.token.is_some()));
    }

    #[test]
    fn a_mid_file_table_takes_its_container_id_without_a_stray_blank_line() {
        let dir = tempfile::tempdir().unwrap();
        // No trailing newline: the EOF lead applies only to the final card,
        // never to the mid-file container insert.
        let original = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n| a | b |\n|---|---|\n| x | y |\n\n## q\nanswer";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let container = &outcome.minted_cards[1];

        assert!(
            stamped.contains(&format!(
                "| x | y | <!-- r:{} -->\n<!-- id: {container} -->\n\n## q\n",
                outcome.minted_rows[0]
            )),
            "the container line follows the last row directly: {stamped:?}"
        );
        assert!(
            stamped.ends_with(&format!(
                "answer\n<!-- id: {} -->\n",
                outcome.minted_cards[0]
            )),
            "the EOF card takes the lead newline instead: {stamped:?}"
        );
    }

    #[test]
    fn a_pipe_prose_answer_line_does_not_end_a_cards_block() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n| just | prose |\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        assert!(
            stamped.ends_with(&format!(
                "| just | prose |\n<!-- id: {} -->\n",
                outcome.minted_cards[0]
            )),
            "the id line lands after the whole answer, pipe line included: {stamped:?}"
        );
    }

    #[test]
    fn a_pipe_prose_line_never_becomes_a_marker_hygiene_block() {
        // Two non-pipe lines before the id: the table walk skips the
        // assumed-delimiter position, so one line cannot discriminate.
        let text = "## q\na\n| p | q |\nmore\nlines\n<!-- id: card-q1 -->\n";
        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(text));
    }

    #[test]
    fn a_card_block_before_a_table_keeps_both_ids_canonical() {
        let text = "## q\na\n<!-- id: card-q1 -->\n\n| a | b |\n|---|---|\n| x | y |\n<!-- id: card-t1 -->\n";
        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(text));
    }

    #[test]
    fn a_blank_line_inside_a_tables_directive_window_does_not_end_its_block() {
        let text = "| a | b |\n|---|---|\n| x | y |\n\n<!-- id: card-t1 -->\n";
        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(text));
    }

    #[test]
    fn a_trailing_arrow_prose_line_does_not_extend_a_tables_block() {
        let text = "| a | b |\n|---|---|\n| x | y |\n<!-- id: card-t1 -->\nplain -->\n";
        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(text));
    }

    #[test]
    fn consecutive_pipe_prose_answer_lines_all_stay_in_the_block() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nformat-version: 1\nid: \"deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q\na\n| p1 | x |\n| p2 | y |\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        assert!(
            stamped.ends_with(&format!(
                "| p2 | y |\n<!-- id: {} -->\n",
                outcome.minted_cards[0]
            )),
            "the id line follows the last pipe prose line: {stamped:?}"
        );
    }

    #[test]
    fn an_id_after_a_broken_table_is_misplaced_at_the_blocks_true_end() {
        let text = "## q\na\n<!-- id: card-q1 -->\n\n| a | b |\n|---|---|\nBOOM -->\n<!-- id: card-t2 -->\n";
        assert_eq!(vec![8], misplaced_id_markers(text));
    }

    #[test]
    fn consecutive_pipe_prose_lines_are_no_table_to_the_hygiene_scan() {
        let text = "## q\na\n| p1 |\n| p2 |\nmore\nlines\n<!-- id: card-q1 -->\n";
        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(text));
    }

    #[test]
    fn a_two_row_tables_container_id_is_canonical() {
        let text = "| a | b |\n|---|---|\n| x | y |\n| z | w |\n<!-- id: card-t1 -->\n";
        assert_eq!(Vec::<usize>::new(), misplaced_id_markers(text));
    }

    #[test]
    fn the_id_token_span_lands_exactly_on_the_token() {
        let text = "## q\na\n<!-- note -->\n<!-- id: card-q1 -->\n";
        let range = first_id_token_span(text, "card-q1").unwrap();
        assert_eq!("card-q1", &text[range]);

        // The scan resumes exactly past a comment's terminator: a body whose
        // tail bytes spell a fresh `<!--` must not swallow the next comment.
        let overlapping = "## q\na\n<!-- x <!-->\n<!-- id: card-q1 -->\n";
        let range = first_id_token_span(overlapping, "card-q1").unwrap();
        assert_eq!("card-q1", &overlapping[range]);
    }

    // ── Region stamps (ADR 0034) ──

    const DECK_HEAD: &str =
        "---\nformat-version: 1\nid: \"deck-regionregionregionregion\"\n---\n\n";

    #[test]
    fn an_unstamped_region_is_minted_into_on_the_first_pass_and_then_stable() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            "d.md",
            &format!(
                "{DECK_HEAD}## q <!-- id: card-regionregionregionregionre -->\n![](a.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 -->\n\n---\nanswer\n"
            ),
        );
        let outcome = stamp_deck(&path).unwrap();
        assert_eq!(1, outcome.minted_regions.len());
        let stamp = &outcome.minted_regions[0];
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(&format!("height=2 b:{stamp} -->")),
            "the mint lands inside the directive comment before its terminator: {text}"
        );

        let again = stamp_deck(&path).unwrap();
        assert!(
            again.minted_regions.is_empty(),
            "a stamped region is left alone"
        );
        assert_eq!(
            text,
            fs::read_to_string(&path).unwrap(),
            "the second pass is a byte no-op"
        );
    }

    #[test]
    fn a_duplicate_stamp_within_one_parent_card_is_reminted_not_fused() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            "d.md",
            &format!(
                "{DECK_HEAD}## q <!-- id: card-regionregionregionregionre -->\n![](a.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 b:a1b2c3 -->\n<!-- blank: rect x=5 y=5 width=2 height=2 b:a1b2c3 -->\n\n---\nanswer\n"
            ),
        );
        let outcome = stamp_deck(&path).unwrap();
        assert_eq!(
            1,
            outcome.minted_regions.len(),
            "exactly one side of the collision re-mints"
        );
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(
            1,
            text.matches("b:a1b2c3").count(),
            "the first keeps its stamp"
        );
        assert!(text.contains(&format!("b:{}", outcome.minted_regions[0])));
    }

    #[test]
    fn a_later_duplicate_region_is_reminted_in_file_order_across_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            "d.md",
            &format!(
                "{DECK_HEAD}## q <!-- id: card-regionregionregionregionre -->\nanswer one\n<!-- blank: span hidden=\"one\" word=2 b:a1b2c3 -->\n![](a.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 b:a1b2c3 -->\n"
            ),
        );

        let outcome = stamp_deck(&path).unwrap();

        assert_eq!(1, outcome.minted_regions.len());
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("<!-- blank: span hidden=\"one\" word=2 b:a1b2c3 -->"),
            "the earlier region keeps the identity and history it already owned: {text}"
        );
        assert!(
            text.contains(&format!(
                "<!-- blank: rect x=1 y=1 width=2 height=2 b:{} -->",
                outcome.minted_regions[0]
            )),
            "the later pasted region receives the fresh identity: {text}"
        );
    }

    #[test]
    fn identical_stamps_on_different_parent_cards_are_harmless_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let card = |token: &str| {
            format!(
                "## q{token} <!-- id: card-{token}{token}{token}{token}{token}{token}re -->\n![](a.png)\n<!-- blank: rect x=1 y=1 width=2 height=2 b:a1b2c3 -->\n\n---\nanswer\n"
            )
        };
        let path = write(
            &dir,
            "d.md",
            &format!("{DECK_HEAD}{}{}", card("xxxxx"), card("yyyyy")),
        );
        let before = fs::read_to_string(&path).unwrap();
        let outcome = stamp_deck(&path).unwrap();
        assert!(
            outcome.minted_regions.is_empty(),
            "cross-card sameness is not a collision"
        );
        assert_eq!(before, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn a_cover_is_never_stamped() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            "d.md",
            &format!(
                "{DECK_HEAD}## q <!-- id: card-regionregionregionregionre -->\n![](a.png)\n<!-- cover: rect x=1 y=1 width=2 height=2 -->\n\n---\nanswer\n"
            ),
        );
        let outcome = stamp_deck(&path).unwrap();
        assert!(outcome.minted_regions.is_empty());
        assert!(!fs::read_to_string(&path).unwrap().contains(" b:"));
    }

    #[test]
    fn a_stamp_matching_text_inside_a_quoted_value_is_not_the_token() {
        let line =
            r#"<!-- blank: rect x=1 y=1 width=2 height=2 hidden="say b:a1b2c3 aloud" b:a1b2c3 -->"#;
        let offset = stamp_token_offset(line, "a1b2c3").unwrap();
        assert!(
            line[..offset].ends_with("aloud\" "),
            "the quoted decoy is skipped; found at {offset}: {line}"
        );
    }
}
