//! The two sanctioned deck-file write shapes (spec §2.3), the ONLY functions
//! that write identity text into a deck file.
//!
//! [`stamp_deck`] is MINT-into-unstamped: it inserts an ` <!-- id: X -->`
//! comment at the end of every unstamped card's `## ` line and, when the deck
//! has no `id:`, either splices one into a block-mapping frontmatter or
//! prepends a canonical `---`/`id: "..."`/`---` block. Its property: the
//! stamped bytes minus exactly the inserted spans equal the original bytes.
//! [`replace_card_token`] is DUPLICATE RESOLUTION: it swaps exactly the old
//! token's character span for a fresh mint, changing nothing else.
//!
//! Both writes are atomic (a sibling `.tmp` then `rename`), so a failed write
//! leaves the original bytes untouched and surfaces the error rather than
//! swallowing it. All tokens are minted before any byte is written, so a mint
//! failure aborts without a partial write.

use std::{
    collections::{HashMap, HashSet},
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{l1, token};

/// The closed six-char ASCII whitespace set the L1 parser trims directive
/// values over (l1 §3.5); mirrored here so token-value spans are located the
/// same way the parser reads them.
const WS: [char; 6] = ['\t', '\n', '\x0B', '\x0C', '\r', ' '];

/// One UTF-8 byte-order mark; kept as byte 0 across a stamp write.
const BOM: &str = "\u{feff}";

/// What a [`stamp_deck`] write minted: the card tokens inserted in document
/// order, and the deck token if the frontmatter gained an `id:`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StampOutcome {
    /// Freshly minted card tokens, one per newly-stamped `## ` front, in
    /// document order.
    pub minted_cards: Vec<String>,
    /// The freshly minted deck token, if the deck had no `id:` and one was
    /// written (spliced or prepended); `None` when the deck was already
    /// stamped.
    pub minted_deck: Option<String>,
}

/// A stamping write failure. Messages are lowercase with no trailing period.
#[derive(Debug, Error)]
pub enum StampError {
    /// The deck file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The new bytes could not be written (the original is left untouched).
    #[error("cannot write {path}: {source}")]
    Write {
        /// The path (or its temp sibling) that could not be written.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The path has no file-name component to derive a subject or temp name.
    #[error("{path} has no file name")]
    NoFileName {
        /// The offending path.
        path: PathBuf,
    },
    /// The file has no cards and no frontmatter: it is not a deck (spec
    /// §3.1.3), so it is never stamped. Refusing here defends a user's prose
    /// file from gaining a frontmatter block on session open, even if it
    /// somehow reaches this path (the enumeration scans already exclude it).
    #[error("{path} is not a deck (no cards, no frontmatter); refusing to stamp")]
    NotADeck {
        /// The offending path.
        path: PathBuf,
    },
    /// A card's front line number pointed past the end of the file (should
    /// never happen: the parser guarantees it).
    #[error("line {0} is past the end of the file")]
    MissingLine(usize),
    /// The deck needs an `id:` but its frontmatter is not a block mapping, so
    /// no `id:` can be spliced in without risking an unloadable file (§2.3).
    #[error("frontmatter is not a block mapping, cannot splice an `id:`")]
    UnspliceableFrontmatter,
    /// The deck did not parse, so its unstamped cards could not be located.
    #[error("deck does not parse: {0}")]
    Parse(#[from] l1::L1Error),
    /// The OS CSPRNG failed while minting a token. Held by Display only:
    /// `getrandom::Error` does not implement `std::error::Error` without its
    /// `std` feature, so it cannot be a `#[source]`.
    #[error("cannot mint a token: {0}")]
    Mint(getrandom::Error),
    /// The token to replace was not found in any `<!-- id: -->` comment.
    #[error("token `{token}` is not present in any `<!-- id: -->` comment")]
    TokenNotFound {
        /// The token that was searched for.
        token: String,
    },
}

/// How a deck without an `id:` acquires one.
enum DeckAction {
    /// The deck already has an `id:`; leave the frontmatter alone.
    None,
    /// No frontmatter: prepend a canonical `---`/`id: "..."`/`---` block.
    Prepend,
    /// A block-mapping frontmatter: splice an `id:` line after its opener
    /// (the 1-based line number carried here).
    Splice(usize),
}

/// Mint identity tokens into an unstamped deck, inserting id text and changing
/// nothing else. Every unstamped card's `## ` front gains a trailing
/// ` <!-- id: X -->`; a deck without an `id:` gains one (spliced into a
/// block-mapping frontmatter, or a canonical block prepended after any BOM).
/// Non-block-mapping frontmatter is a loud write-fail that leaves the file
/// untouched. A deck with nothing to stamp is a byte no-op (no write).
pub fn stamp_deck(path: &Path) -> Result<StampOutcome, StampError> {
    stamp_deck_reclaiming(path, &HashMap::new())
}

/// [`stamp_deck`], but an unstamped card whose §7 content fingerprint is a key
/// in `reclaim` is stamped with that pre-existing token instead of a fresh mint
/// — the §1.7 lost-comment RECLAIM: a card that lost its `<!-- id: -->` comment
/// re-adopts the orphaned token its progress still hangs off. Each reclaim
/// token is used at most once (a second card of identical content mints fresh),
/// so no reclaim can mint a duplicate. An empty map is exactly [`stamp_deck`].
pub fn stamp_deck_reclaiming(
    path: &Path,
    reclaim: &HashMap<u64, String>,
) -> Result<StampOutcome, StampError> {
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

    // Work on the post-BOM body so parser line numbers and byte offsets align;
    // the BOM is reattached unchanged as byte 0.
    let bom = if original.starts_with(BOM) { BOM } else { "" };
    let body = &original[bom.len()..];

    let deck = l1::parse_l1(subject, body)?;

    // Defensive (spec §3.1.3): a file with neither cards nor frontmatter is not
    // a deck. Refuse loudly rather than prepend a canonical frontmatter block
    // into what is almost certainly a user's prose file.
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
    } else {
        match deck.frontmatter_span {
            None => DeckAction::Prepend,
            Some((open, _close)) if !deck.frontmatter.unspliceable => DeckAction::Splice(open),
            // Frontmatter exists but is not a block mapping: excluded loudly,
            // file untouched (spec §2.3).
            Some(_) => return Err(StampError::UnspliceableFrontmatter),
        }
    };

    // Nothing to write: a genuine byte no-op, so don't touch the file.
    if card_lines.is_empty() && matches!(deck_action, DeckAction::None) {
        return Ok(StampOutcome::default());
    }

    // Mint every token before writing a single byte (no partial writes).
    let deck_token = match deck_action {
        DeckAction::None => None,
        _ => Some(mint()?),
    };
    // A card line's §7 content fingerprint (a cloze block's sub-cards share it),
    // so a lost-comment reclaim can re-adopt the orphaned token by content.
    let fp_by_line: HashMap<usize, u64> = deck
        .cards
        .iter()
        .map(|card| (card.line, card.content_fingerprint))
        .collect();
    let mut reclaimed_tokens: HashSet<String> = HashSet::new();
    let mut minted_cards = Vec::with_capacity(card_lines.len());
    for line in &card_lines {
        let reclaim_token = fp_by_line
            .get(line)
            .and_then(|fp| reclaim.get(fp))
            .filter(|token| !reclaimed_tokens.contains(*token));
        match reclaim_token {
            Some(token) => {
                reclaimed_tokens.insert(token.clone());
                minted_cards.push(token.clone());
            }
            None => minted_cards.push(mint()?),
        }
    }

    // Collect every insertion into `body` as (byte offset, text), then apply
    // them right-to-left so earlier offsets stay valid.
    let mut inserts: Vec<(usize, String)> = Vec::new();
    for (line, tok) in card_lines.iter().zip(&minted_cards) {
        let offset = line_content_end(body, *line).ok_or(StampError::MissingLine(*line))?;
        inserts.push((offset, format!(" <!-- id: {tok} -->")));
    }
    let mut prepend = String::new();
    match (&deck_action, &deck_token) {
        (DeckAction::Splice(open), Some(tok)) => {
            let offset = line_start_of_next(body, *open).ok_or(StampError::MissingLine(*open))?;
            inserts.push((offset, format!("id: \"{tok}\"\n")));
        }
        (DeckAction::Prepend, Some(tok)) => {
            prepend = format!("---\nid: \"{tok}\"\n---\n");
        }
        _ => {}
    }

    inserts.sort_by(|a, b| b.0.cmp(&a.0));
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

/// Replace exactly the span of `old_token` inside its `<!-- id: ... -->`
/// comment with a fresh mint, returning the fresh token. If the token appears
/// in more than one id comment, only the first (document order) is replaced.
pub fn replace_card_token(path: &Path, old_token: &str) -> Result<String, StampError> {
    let original = fs::read_to_string(path).map_err(|source| StampError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let span =
        first_id_token_span(&original, old_token).ok_or_else(|| StampError::TokenNotFound {
            token: old_token.to_string(),
        })?;
    let fresh = mint()?;
    let mut new_text = String::with_capacity(original.len() + fresh.len());
    new_text.push_str(&original[..span.start]);
    new_text.push_str(&fresh);
    new_text.push_str(&original[span.end..]);
    write_atomic(path, &new_text)?;
    Ok(fresh)
}

/// Mint one token, mapping the CSPRNG error into a [`StampError`].
fn mint() -> Result<String, StampError> {
    token::mint().map_err(StampError::Mint)
}

/// Atomically replace `path`'s bytes: write a sibling `.<name>.tmp`, then
/// `rename` over the original. On any failure the original is left untouched.
fn write_atomic(path: &Path, contents: &str) -> Result<(), StampError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| StampError::NoFileName {
            path: path.to_path_buf(),
        })?;
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let tmp = parent.join(format!(".{file_name}.tmp"));
    fs::write(&tmp, contents).map_err(|source| StampError::Write {
        path: tmp.clone(),
        source,
    })?;
    fs::rename(&tmp, path).map_err(|source| {
        // The rename failed, so the tmp is stray: best-effort cleanup.
        let _ = fs::remove_file(&tmp);
        StampError::Write {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// The byte offset in `text` of the end of the 1-based `line`'s content: just
/// before its line terminator (`\n` or `\r\n`), or at EOF for the last line.
/// `None` if the file has fewer than `line` lines.
fn line_content_end(text: &str, line: usize) -> Option<usize> {
    let start = nth_line_start(text, line)?;
    let rest = &text[start..];
    let mut end = start + rest.find('\n').unwrap_or(rest.len());
    if end > start && text.as_bytes()[end - 1] == b'\r' {
        end -= 1;
    }
    Some(end)
}

/// The byte offset in `text` where the line after the 1-based `line` begins
/// (just past `line`'s `\n`), or EOF when `line` is the last line. `None` if
/// the file has fewer than `line` lines.
fn line_start_of_next(text: &str, line: usize) -> Option<usize> {
    let start = nth_line_start(text, line)?;
    let rest = &text[start..];
    Some(match rest.find('\n') {
        Some(nl) => start + nl + 1,
        None => text.len(),
    })
}

/// The byte offset where the 1-based `line` begins (0 for line 1, just past
/// the `(line - 1)`th `\n` otherwise). `None` if there are too few lines.
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

/// The byte range of the first `<!-- id: TOKEN -->` comment whose token value
/// equals `target`, scanned in document order. `None` if no id comment holds
/// it. (A `target` appearing only as inert fenced text is a theoretical
/// collision a 26-char random token makes vanishingly unlikely.)
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

/// The byte range within the file of an id directive's token value, given the
/// comment `body` and the body's byte start. `None` if `body` is not an `id:`
/// directive with a non-empty value. Mirrors the parser's `key: value` split
/// and its closed whitespace set.
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

    /// A fixture exercising every L1 marker (block-mapping frontmatter without
    /// an `id:`, a divided card with a fence, note and escape, a trailing-space
    /// front, and a two-hole cloze card). The trailing space on the first
    /// front is deliberate: it is what the Step-4 mutation (trimming the line
    /// before inserting) would silently normalize away.
    const FIXTURE: &str = "---\nsource: notes.md\nrequires: basics\n---\n# The Title\nintro prose\n\n## First question \nextra front line\n\n---\nthe answer\n\\--- escaped divider\n> a note\n```\nfenced\n## not a card\n```\ntail prose\n\n## Fill in the blanks\nthe \\cloze{alpha} and \\cloze{beta} here\n> cloze note\n";

    fn write(dir: &tempfile::TempDir, name: &str, text: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn stamping_inserts_ids_and_changes_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "deck.md", FIXTURE);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        // Two file cards (the cloze card's two holes stamp once), one deck id.
        assert_eq!(2, outcome.minted_cards.len());
        assert!(outcome.minted_deck.is_some());

        // Reconstruct the original by deleting exactly the inserted spans.
        let mut reconstructed = stamped;
        for tok in &outcome.minted_cards {
            let span = format!(" <!-- id: {tok} -->");
            assert_eq!(1, reconstructed.matches(&span).count(), "span {span:?}");
            reconstructed = reconstructed.replacen(&span, "", 1);
        }
        let deck_tok = outcome.minted_deck.as_ref().unwrap();
        let deck_span = format!("id: \"{deck_tok}\"\n");
        assert_eq!(1, reconstructed.matches(&deck_span).count());
        reconstructed = reconstructed.replacen(&deck_span, "", 1);

        assert_eq!(FIXTURE, reconstructed);
    }

    #[test]
    fn stamping_a_deck_without_frontmatter_prepends_the_canonical_three_line_block() {
        let dir = tempfile::tempdir().unwrap();
        let original = "## q\na\n## r\nb\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let deck_tok = outcome.minted_deck.as_ref().unwrap();

        assert!(
            stamped.starts_with(&format!("---\nid: \"{deck_tok}\"\n---\n")),
            "{stamped:?}"
        );
        // The prepended block re-parses into a stamped deck.
        let parsed = l1::parse_l1("deck.md", &stamped).unwrap();
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

        // The BOM stays byte 0, the canonical block follows it.
        assert!(stamped.starts_with(BOM));
        assert!(!stamped[BOM.len()..].starts_with(BOM));
        assert!(stamped.starts_with(&format!("{BOM}---\nid: \"{deck_tok}\"\n---\n")));
    }

    #[test]
    fn an_id_line_splices_into_block_mapping_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nsource: notes.md\n---\n## q\na\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let deck_tok = outcome.minted_deck.as_ref().unwrap();

        // The `id:` is spliced right after the opening fence, above `source:`.
        assert_eq!(
            format!("---\nid: \"{deck_tok}\"\nsource: notes.md\n---\n"),
            stamped[..stamped.find("## q").unwrap()]
        );
        let parsed = l1::parse_l1("deck.md", &stamped).unwrap();
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
        // The whole deck is excluded: the file is untouched, `## q` unstamped.
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
        let stamped_card = "## already <!-- id: 4jkya9q3m8z0tw5v9y2b4n6d8f -->\na\n";
        let original =
            format!("---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n{stamped_card}## missing\nb\n");
        let path = write(&dir, "deck.md", &original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        // Only the one unstamped card was minted; the deck already had an id.
        assert_eq!(1, outcome.minted_cards.len());
        assert_eq!(None, outcome.minted_deck);
        // The existing tokens are untouched.
        assert!(stamped.contains("4jkya9q3m8z0tw5v9y2b4n6d8f"));
        assert!(stamped.contains("9w2c7x4k1m8q3z5t0v6b2n4d8f"));
        // The missing card now carries exactly the minted token.
        let new_tok = &outcome.minted_cards[0];
        assert!(stamped.contains(&format!("## missing <!-- id: {new_tok} -->")));
    }

    #[test]
    fn token_replacement_swaps_exactly_the_old_span() {
        let dir = tempfile::tempdir().unwrap();
        let old = "4jkya9q3m8z0tw5v9y2b4n6d8f";
        let other = "zzzzzzzzzzzzzzzzzzzzzzzzzz";
        let original = format!(
            "---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n\
             ## q <!-- id: {old} -->\na\n## r <!-- id: {other} -->\nb\n"
        );
        let path = write(&dir, "deck.md", &original);

        let fresh = replace_card_token(&path, old).unwrap();
        let output = fs::read_to_string(&path).unwrap();

        // The property: output minus the replaced span == original minus old.
        assert_eq!(
            output.replacen(&fresh, "", 1),
            original.replacen(old, "", 1)
        );
        assert!(output.contains(&format!("<!-- id: {fresh} -->")));
        assert!(!output.contains(old));
        // Neither the sibling card token nor the deck token moved.
        assert!(output.contains(other));
        assert!(output.contains("9w2c7x4k1m8q3z5t0v6b2n4d8f"));
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_write_leaves_the_original_untouched() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let original = "## q\na\n";
        let path = write(&dir, "deck.md", original);

        // A read-only directory blocks creating the sibling `.tmp` file.
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
        let original = "## q <!-- id: 4jkya9q3m8z0tw5v9y2b4n6d8f -->\na\n";
        let path = write(&dir, "deck.md", original);

        let result = replace_card_token(&path, "zzzzzzzzzzzzzzzzzzzzzzzzzz");
        assert!(
            matches!(result, Err(StampError::TokenNotFound { .. })),
            "{result:?}"
        );
        assert_eq!(original, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn selecting_a_prose_file_refuses_loudly_and_never_writes() {
        // A prose `.md` (no `## ` card, no frontmatter) is not a deck: the
        // session-open stamp path refuses it instead of prepending a
        // frontmatter block, and leaves the file byte-for-byte untouched.
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
        // The whole deck is excluded: the file is untouched, `## q` unstamped.
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

        // Delete exactly the inserted prepend block and the card insertions;
        // what remains must be byte-identical to the original.
        let prefix = format!("---\nid: \"{deck_tok}\"\n---\n");
        assert!(stamped.starts_with(&prefix), "{stamped:?}");
        let mut reconstructed = stamped[prefix.len()..].to_string();
        for tok in &outcome.minted_cards {
            let span = format!(" <!-- id: {tok} -->");
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

        // Card insertions land right before each front line's `\r\n`.
        for tok in &outcome.minted_cards {
            assert!(
                stamped.contains(&format!(" <!-- id: {tok} -->\r\n")),
                "{stamped:?}"
            );
        }

        // Delete exactly the inserted deck-id line and the card insertions;
        // what remains must be byte-identical to the original, CRs included.
        let deck_span = format!("id: \"{deck_tok}\"\n");
        assert_eq!(1, stamped.matches(&deck_span).count());
        let mut reconstructed = stamped.replacen(&deck_span, "", 1);
        for tok in &outcome.minted_cards {
            let span = format!(" <!-- id: {tok} -->");
            assert_eq!(1, reconstructed.matches(&span).count(), "span {span:?}");
            reconstructed = reconstructed.replacen(&span, "", 1);
        }

        assert_eq!(original, reconstructed);
    }

    #[test]
    fn a_front_with_a_trailing_directive_still_gets_its_id_appended() {
        let dir = tempfile::tempdir().unwrap();
        let original =
            "---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## q <!-- reveal: line -->\na\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let tok = &outcome.minted_cards[0];

        assert_eq!(
            format!(
                "---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n\
                 ## q <!-- reveal: line --> <!-- id: {tok} -->\na\n"
            ),
            stamped
        );

        // Reconstruction: removing exactly the inserted span restores the
        // original, directive comment and all.
        let reconstructed = stamped.replacen(&format!(" <!-- id: {tok} -->"), "", 1);
        assert_eq!(original, reconstructed);
    }

    #[test]
    fn a_hash_run_front_gets_its_id_after_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## Foo ##\nbar\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();
        let tok = &outcome.minted_cards[0];

        assert_eq!(
            format!(
                "---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## Foo ## <!-- id: {tok} -->\nbar\n"
            ),
            stamped
        );

        let reconstructed = stamped.replacen(&format!(" <!-- id: {tok} -->"), "", 1);
        assert_eq!(original, reconstructed);

        // Re-parse: the hash run still strips, and the token is still found.
        let parsed = l1::parse_l1("deck.md", &stamped).unwrap();
        assert_eq!("Foo", parsed.cards[0].front);
        assert_eq!(Some(tok.as_str()), parsed.cards[0].token.as_deref());
    }

    #[test]
    fn identical_cloze_fronts_on_different_lines_each_get_their_own_token() {
        let dir = tempfile::tempdir().unwrap();
        let original = "---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## Foo\n---\nthe \\cloze{a} note\n\n\
             ## Foo\n---\nthe \\cloze{b} note\n";
        let path = write(&dir, "deck.md", original);

        let outcome = stamp_deck(&path).unwrap();
        let stamped = fs::read_to_string(&path).unwrap();

        // Two distinct `## Foo` lines, byte-identical front text, each minted
        // its own token: two mints, not deduped by content.
        assert_eq!(2, outcome.minted_cards.len());
        assert_ne!(outcome.minted_cards[0], outcome.minted_cards[1]);
        for tok in &outcome.minted_cards {
            assert_eq!(
                1,
                stamped
                    .matches(&format!("## Foo <!-- id: {tok} -->"))
                    .count(),
                "{stamped:?}"
            );
        }
    }
}
