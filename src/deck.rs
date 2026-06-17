//! A deck is a parsed flashcard file.

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use thiserror::Error;

use crate::{
    answer::Mode,
    card::Card,
    parser::{self, ParseError},
    scheduler::SchedulerKind,
    session::Order,
};

/// Per-deck defaults declared with `% key: value` header directives, e.g.
/// `% mode: line` or `% order: sequential`. Each is `None` unless the deck
/// sets it; an explicit CLI flag always takes precedence. Unknown keys and
/// unparseable values are ignored, so the directives never break a deck.
#[derive(Debug, Default, Clone, Copy)]
pub struct DeckSettings {
    /// Default answer mode for this deck (`% mode: ...`).
    pub mode: Option<Mode>,
    /// Default scheduler for this deck (`% scheduler: ...`).
    pub scheduler: Option<SchedulerKind>,
    /// Default card order for this deck (`% order: ...`).
    pub order: Option<Order>,
}

impl DeckSettings {
    /// Interprets the recognized directives; ignores the rest.
    fn from_directives(directives: &[(String, String)]) -> Self {
        let mut settings = Self::default();
        for (key, value) in directives {
            match key.as_str() {
                "mode" => settings.mode = Mode::from_str(value, true).ok(),
                "scheduler" => settings.scheduler = SchedulerKind::from_str(value, true).ok(),
                "order" => settings.order = Order::from_str(value, true).ok(),
                _ => {}
            }
        }
        settings
    }
}

/// A deck of flashcards loaded from a file.
#[derive(Debug)]
pub struct Deck {
    /// The path the deck was loaded from.
    pub path: PathBuf,
    /// The subject (= file name), part of every card's identity hash.
    pub subject: String,
    /// The cards, in file order.
    pub cards: Vec<Card>,
    /// Deck-level reference links (`% link: <url>` lines).
    pub links: Vec<String>,
    /// Prerequisite decks (`% requires: <deck>` lines), as written.
    pub requires: Vec<String>,
    /// Per-deck defaults from `% key: value` directives.
    pub settings: DeckSettings,
}

/// An error loading a deck file.
#[derive(Debug, Error)]
pub enum DeckError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
    #[error("{path}: file name is not valid UTF-8")]
    InvalidFileName { path: PathBuf },
}

impl Deck {
    /// Loads and parses a deck file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DeckError> {
        let path = path.as_ref().to_path_buf();
        let subject = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| DeckError::InvalidFileName { path: path.clone() })?
            .to_string();
        let text = std::fs::read_to_string(&path).map_err(|source| DeckError::Io {
            path: path.clone(),
            source,
        })?;
        let mut cards = parser::parse_str(&subject, &text).map_err(|source| DeckError::Parse {
            path: path.clone(),
            source,
        })?;
        let links = parser::parse_links(&text);
        let requires = parser::parse_requires(&text);
        let settings = DeckSettings::from_directives(&parser::parse_directives(&text));
        // A card without its own `% mode:` inherits the deck's mode, so each
        // card carries its effective declared mode (card override, else deck).
        for card in &mut cards {
            card.mode = card.mode.or(settings.mode);
        }
        Ok(Self {
            path,
            subject,
            cards,
            links,
            requires,
            settings,
        })
    }

    /// Returns pairs of cards within this deck that share the same identity
    /// hash (i.e. same back lines). Such cards are indistinguishable to the
    /// progress store, so the `check` command warns about them.
    pub fn duplicates(&self) -> Vec<(&Card, &Card)> {
        let mut seen: std::collections::HashMap<u64, &Card> = Default::default();
        let mut dups = Vec::new();
        for card in &self.cards {
            if let Some(first) = seen.insert(card.id(), card) {
                dups.push((first, card));
                // keep reporting against the first occurrence
                seen.insert(card.id(), first);
            }
        }
        dups
    }
}

/// Appends `notes` as `!` lines to the card whose front is at the 1-based
/// `front_line` of the deck file at `path`. The file is rewritten atomically
/// (temp file + rename); on reload the parser merges the new lines into the
/// card's (possibly multi-line) note. Card identities don't change — notes
/// are not hashed.
pub fn append_note(path: &Path, front_line: usize, notes: &[String]) -> Result<(), DeckError> {
    if notes.is_empty() {
        return Ok(());
    }
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };

    let text = std::fs::read_to_string(path).map_err(io_err)?;
    let new_text = insert_note_lines(&text, front_line, notes);

    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, new_text).map_err(io_err)?;
    std::fs::rename(&tmp, path).map_err(io_err)?;
    Ok(())
}

/// Rewrites a deck file's `% requires:` lines to exactly `deps` (deck names),
/// grouped at the top of the file; any existing `% requires:` lines are
/// removed first. Written atomically (temp + rename). Card identities are
/// unaffected — comments are not hashed — so dependencies can be changed
/// freely without disturbing progress. An empty `deps` clears them.
pub fn set_requires(path: &Path, deps: &[String]) -> Result<(), DeckError> {
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let text = std::fs::read_to_string(path).map_err(io_err)?;
    let new_text = rewrite_requires(&text, deps);

    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, new_text).map_err(io_err)?;
    std::fs::rename(&tmp, path).map_err(io_err)?;
    Ok(())
}

/// Removes whole card blocks from a deck file: every card whose front sits at
/// one of the 1-based `front_lines` is deleted along with its back lines, notes
/// and trailing blank separator. The block runs from the front (a column-0 `#`
/// line) to the next card's front, or the end of the file. Passing the front
/// line of any cloze sub-card removes the whole `#?` source block, since all of
/// its holes share that line. The file is rewritten atomically (temp + rename).
/// An empty `front_lines` is a no-op.
pub fn remove_cards(path: &Path, front_lines: &[usize]) -> Result<(), DeckError> {
    if front_lines.is_empty() {
        return Ok(());
    }
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let text = std::fs::read_to_string(path).map_err(io_err)?;
    let new_text = remove_card_blocks(&text, front_lines);

    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, new_text).map_err(io_err)?;
    std::fs::rename(&tmp, path).map_err(io_err)?;
    Ok(())
}

/// Rewrites `path` to `original` with the card blocks at `front_lines` removed.
/// Unlike [`remove_cards`], the caller supplies the file's *original* content,
/// so the line numbers stay valid however many cards were removed before. The
/// web server uses this: it removes cards immediately but keeps each deck's
/// original text in memory and re-derives the file from the growing set of
/// removed lines, sidestepping the line shifts that repeated in-place edits
/// would cause. Written atomically (temp + rename).
pub fn rewrite_without_cards(
    path: &Path,
    original: &str,
    front_lines: &[usize],
) -> Result<(), DeckError> {
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let new_text = remove_card_blocks(original, front_lines);
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, new_text).map_err(io_err)?;
    std::fs::rename(&tmp, path).map_err(io_err)?;
    Ok(())
}

/// Returns `text` with the card blocks starting at the given 1-based front
/// lines removed. A card front is a column-0 `#` line; its block extends to the
/// next column-0 `#` (or end of file), so the front, back lines, notes and the
/// blank line after it all go. A `front_line` that does not land on a card
/// front is ignored, so a stale line number can never corrupt the file.
fn remove_card_blocks(text: &str, front_lines: &[usize]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    // A column-0 `#` starts a card; an indented `#` is back content, a `%` is a
    // comment — neither starts a block.
    let is_front = |line: &str| line.starts_with('#');
    let targets: std::collections::HashSet<usize> =
        front_lines.iter().map(|n| n.saturating_sub(1)).collect();

    let mut drop = vec![false; lines.len()];
    for (i, line) in lines.iter().enumerate() {
        if targets.contains(&i) && is_front(line) {
            drop[i] = true;
            let mut j = i + 1;
            while j < lines.len() && !is_front(lines[j]) {
                drop[j] = true;
                j += 1;
            }
        }
    }

    let kept: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop[*i])
        .map(|(_, line)| *line)
        .collect();
    let mut result = kept.join("\n");
    if text.ends_with('\n') && !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// `true` if `line` is a `% requires:` directive.
fn is_requires_line(line: &str) -> bool {
    line.trim()
        .strip_prefix('%')
        .is_some_and(|rest| rest.trim().strip_prefix("requires:").is_some())
}

/// Drops existing `% requires:` lines and prepends one per `dep`.
fn rewrite_requires(text: &str, deps: &[String]) -> String {
    let kept: Vec<&str> = text.lines().filter(|l| !is_requires_line(l)).collect();
    let mut out = String::new();
    for dep in deps {
        out.push_str("% requires: ");
        out.push_str(dep);
        out.push('\n');
    }
    out.push_str(&kept.join("\n"));
    if text.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Inserts `notes` as tab-indented `!` lines after the last content line of
/// the card whose front sits at the 1-based `front_line`.
fn insert_note_lines(text: &str, front_line: usize, notes: &[String]) -> String {
    let lines: Vec<&str> = text.lines().collect();

    // Walk from the line after the front to the next column-0 front (or
    // EOF), remembering the last non-blank line that belongs to the card.
    let front_index = front_line.saturating_sub(1);
    let mut last_content = front_index;
    let mut i = front_index + 1;
    while i < lines.len() {
        if lines[i].starts_with('#') {
            break;
        }
        if !lines[i].trim().is_empty() {
            last_content = i;
        }
        i += 1;
    }

    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    for (offset, note) in notes.iter().enumerate() {
        out.insert(last_content + 1 + offset, format!("\t! {note}"));
    }

    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn load_deck_subject_is_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mydeck.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "# front\nback").unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!("mydeck.txt", deck.subject);
        assert_eq!(1, deck.cards.len());
        assert_eq!("mydeck.txt", &*deck.cards[0].subject);
    }

    #[test]
    fn insert_note_after_existing_card_content() {
        let text = "# one\n\tback 1\n\t! old note\n\n# two\n\tback 2\n";
        let notes = vec!["new a".to_string(), "new b".to_string()];
        let result = insert_note_lines(text, 1, &notes);
        assert_eq!(
            "# one\n\tback 1\n\t! old note\n\t! new a\n\t! new b\n\n# two\n\tback 2\n",
            result
        );
        // The result must still parse, with the note extended.
        let cards = crate::parser::parse_str("s", &result).unwrap();
        assert_eq!(Some("old note\nnew a\nnew b".to_string()), cards[0].note);
    }

    #[test]
    fn insert_note_on_last_card_without_note() {
        let text = "# one\n\tback 1\n";
        let result = insert_note_lines(text, 1, &["note".to_string()]);
        assert_eq!("# one\n\tback 1\n\t! note\n", result);
        let cards = crate::parser::parse_str("s", &result).unwrap();
        assert_eq!(Some("note".to_string()), cards[0].note);
    }

    #[test]
    fn insert_note_targets_the_right_card() {
        let text = "# one\n\tback 1\n\n# two\n\tback 2\n\n# three\n\tback 3\n";
        let result = insert_note_lines(text, 4, &["mid".to_string()]);
        let cards = crate::parser::parse_str("s", &result).unwrap();
        assert_eq!(None, cards[0].note);
        assert_eq!(Some("mid".to_string()), cards[1].note);
        assert_eq!(None, cards[2].note);
    }

    #[test]
    fn append_note_rewrites_the_file_and_card_ids_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "# front\n\tanswer\n").unwrap();

        let before = Deck::load(&path).unwrap();
        append_note(&path, 1, &["explained".to_string()]).unwrap();
        let after = Deck::load(&path).unwrap();

        assert_eq!(Some("explained".to_string()), after.cards[0].note);
        // Notes are not hashed: progress stays attached.
        assert_eq!(before.cards[0].id(), after.cards[0].id());
    }

    #[test]
    fn remove_card_block_drops_front_back_and_trailing_blank() {
        let text = "# one\n\tback 1\n\t! a note\n\n# two\n\tback 2\n";
        // Removing the first card takes its note and the blank separator too.
        assert_eq!("# two\n\tback 2\n", remove_card_blocks(text, &[1]));
        // Removing the last card leaves the first intact.
        assert_eq!("# one\n\tback 1\n\t! a note\n", remove_card_blocks(text, &[5]));
    }

    #[test]
    fn remove_card_block_keeps_header_and_neighbors() {
        let text = "% requires: base\n% link: https://x\n# a\n\tx\n# b\n\ty\n# c\n\tz\n";
        // The middle card goes; the header and the other two stay.
        assert_eq!(
            "% requires: base\n% link: https://x\n# a\n\tx\n# c\n\tz\n",
            remove_card_blocks(text, &[5])
        );
    }

    #[test]
    fn remove_card_block_handles_indented_hash_back_line() {
        // An indented `#` is back content, not a new card, so it is part of the
        // block and does not end it.
        let text = "# q\n\t# answer with a hash\n# next\n\tb\n";
        assert_eq!("# next\n\tb\n", remove_card_blocks(text, &[1]));
    }

    #[test]
    fn remove_multiple_and_stale_line_is_ignored() {
        let text = "# a\n\tx\n# b\n\ty\n# c\n\tz\n";
        // Remove a and c; a line that isn't a front (2) is ignored.
        assert_eq!("# b\n\ty\n", remove_card_blocks(text, &[1, 2, 5]));
        // Removing everything yields an empty file (no stray newline).
        assert_eq!("", remove_card_blocks(text, &[1, 3, 5]));
    }

    #[test]
    fn remove_cards_rewrites_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "# one\n\tback 1\n\n# two\n\tback 2\n").unwrap();

        remove_cards(&path, &[1]).unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!("two", deck.cards[0].front);
    }

    #[test]
    fn settings_parsed_from_directives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(
            &path,
            "% mode: line\n% order: sequential\n% scheduler: bogus\n# f\n\tb\n",
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Mode::LineByLine), deck.settings.mode);
        assert_eq!(Some(Order::Sequential), deck.settings.order);
        // An unparseable value is ignored, not an error.
        assert_eq!(None, deck.settings.scheduler);
    }

    #[test]
    fn rewrite_requires_replaces_block_at_top() {
        let text = "% requires: old\n# a\n\tb\n";
        let out = rewrite_requires(text, &["x.txt".to_string(), "y.txt".to_string()]);
        assert_eq!("% requires: x.txt\n% requires: y.txt\n# a\n\tb\n", out);
    }

    #[test]
    fn rewrite_requires_empty_clears_them_keeping_other_comments() {
        let text = "% requires: old\n% mode: line\n# a\n\tb\n";
        assert_eq!("% mode: line\n# a\n\tb\n", rewrite_requires(text, &[]));
    }

    #[test]
    fn set_requires_roundtrips_via_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "# front\n\tanswer\n").unwrap();

        let before = Deck::load(&path).unwrap();
        set_requires(&path, &["basics.txt".to_string()]).unwrap();
        let after = Deck::load(&path).unwrap();

        assert_eq!(vec!["basics.txt".to_string()], after.requires);
        // Comments aren't hashed, so the card's identity is unchanged.
        assert_eq!(before.cards[0].id(), after.cards[0].id());

        // Clearing removes the line again.
        set_requires(&path, &[]).unwrap();
        assert!(Deck::load(&path).unwrap().requires.is_empty());
    }

    #[test]
    fn requires_parsed_from_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "% requires: basics\n% requires: x.txt\n# f\n\tb\n").unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            vec!["basics".to_string(), "x.txt".to_string()],
            deck.requires
        );
    }

    #[test]
    fn card_mode_is_card_override_else_deck_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(
            &path,
            "% mode: flip\n# a\n% mode: choice\n\tx\n# b\n\ty\n",
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Mode::Choice), deck.cards[0].mode); // card override wins
        assert_eq!(Some(Mode::Flip), deck.cards[1].mode); // inherits the deck's
    }

    #[test]
    fn cards_have_no_mode_without_directives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "# a\n\tx\n").unwrap();
        assert_eq!(None, Deck::load(&path).unwrap().cards[0].mode);
    }

    #[test]
    fn no_directives_yields_empty_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "% just a comment\n# f\n\tb\n").unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(None, deck.settings.mode);
        assert_eq!(None, deck.settings.scheduler);
        assert_eq!(None, deck.settings.order);
    }

    #[test]
    fn duplicates_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "# one\nsame\n# two\nsame\n# three\nother\n").unwrap();

        let deck = Deck::load(&path).unwrap();
        let dups = deck.duplicates();
        assert_eq!(1, dups.len());
        assert_eq!("one", dups[0].0.front);
        assert_eq!("two", dups[0].1.front);
    }
}
