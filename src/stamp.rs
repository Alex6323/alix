use std::{
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

#[derive(Debug, Default, PartialEq, Eq)]
pub struct StampOutcome {
    pub minted_cards: Vec<String>,
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
    #[error("token `{token}` is not present in any `<!-- id: -->` comment")]
    TokenNotFound { token: String },
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

    // Unstamped card front lines, deduped: a cloze card's holes expand to
    // several sub-cards sharing one `## ` line, but that line stamps once.
    let mut card_lines: Vec<usize> = Vec::new();
    for card in &deck.cards {
        if card.token.is_none() && !card_lines.contains(&card.line) {
            card_lines.push(card.line);
        }
    }
    card_lines.sort_unstable();

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
    if card_lines.is_empty() && matches!(deck_action, DeckAction::None) {
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
    let mut inserts: Vec<(usize, String)> = Vec::new();
    for (line, tok) in card_lines.iter().zip(&minted_cards) {
        let anchor = block_end_line(body, *line);
        let newline = line_terminator(body, anchor);
        let offset = line_start_of_next(body, anchor).ok_or(StampError::MissingLine(anchor))?;
        let lead = if offset == body.len() && !body.ends_with('\n') {
            newline
        } else {
            ""
        };
        inserts.push((offset, format!("{lead}<!-- id: {tok} -->{newline}")));
    }
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
            inserts.push((offset, format!("{version}id: \"{tok}\"\n")));
        }
        (DeckAction::Prepend, Some(tok)) => {
            prepend = format!("---\nformat-version: {DECK_FORMAT_VERSION}\nid: \"{tok}\"\n\n---\n");
        }
        _ => {}
    }

    inserts.sort_by_key(|b| std::cmp::Reverse(b.0));
    let mut new_body = body.to_string();
    for (offset, text) in inserts {
        new_body.insert_str(offset, &text);
    }
    let new_text = format!("{bom}{prepend}{new_body}");

    write_atomic(path, &new_text)?;

    Ok(StampOutcome {
        minted_cards,
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
        false,
    ))
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
    let mut found = Vec::new();
    let mut fence: Option<char> = None;
    let mut block_end: Option<usize> = None;
    for (index, raw) in text.lines().enumerate() {
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
    match text[start..].find('\n') {
        Some(rel) if rel > 0 && text.as_bytes()[start + rel - 1] == b'\r' => "\r\n",
        _ => "\n",
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
                "---\nformat-version: 1\nid: \"{deck_tok}\"\n\n---\n"
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
            "{BOM}---\nformat-version: 1\nid: \"{deck_tok}\"\n\n---\n"
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

        let prefix = format!("---\nformat-version: 1\nid: \"{deck_tok}\"\n\n---\n");
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
}
