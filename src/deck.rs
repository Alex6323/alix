//! A deck is a parsed flashcard file.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use clap::ValueEnum;
use thiserror::Error;

use crate::{
    answer::Mode,
    card::{Card, Direction, Frontend},
    parser::{self, ParseError},
    scheduler::SchedulerKind,
    session::{self, Order},
    store::{MAX_STAGE, Store},
};

/// Per-deck defaults declared with `% key: value` header directives, e.g.
/// `% mode: line` or `% order: sequential`. Each is `None` unless the deck
/// sets it; an explicit CLI flag always takes precedence. Unknown keys and
/// unparseable values are ignored, so the directives never break a deck.
#[derive(Debug, Default, Clone)]
pub struct DeckSettings {
    /// Default answer mode for this deck (`% mode: ...`).
    pub mode: Option<Mode>,
    /// Default scheduler for this deck (`% scheduler: ...`).
    pub scheduler: Option<SchedulerKind>,
    /// Default card order for this deck (`% order: ...`).
    pub order: Option<Order>,
    /// Default review direction for this deck (`% direction: ...`).
    pub direction: Option<Direction>,
    /// Default frontend for this deck (`% frontend: ...`).
    pub frontend: Option<Frontend>,
    /// Directory that card `% img:` / `% img-back:` filenames resolve against
    /// (`% img-dir: ...`). Absolute, or relative to the deck file's folder.
    pub img_dir: Option<PathBuf>,
    /// Top Leitner stage for this deck (`% max-stage: 1..=5`). `None` uses the
    /// global [`MAX_STAGE`]. Reaching it retires the card.
    pub max_stage: Option<u8>,
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
                "direction" => settings.direction = Direction::from_str(value, true).ok(),
                "frontend" => settings.frontend = Frontend::from_str(value, true).ok(),
                "img-dir" => settings.img_dir = Some(PathBuf::from(value)),
                "max-stage" => {
                    settings.max_stage = value.trim().parse::<u8>().ok().map(|n| n.clamp(1, MAX_STAGE))
                }
                _ => {}
            }
        }
        settings
    }
}

/// How far through a deck the user is, derived from its cards' current stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeckState {
    /// No card has been reviewed yet.
    NotStarted,
    /// Some cards reviewed, but not all are at the top stage.
    Started,
    /// Every card is at the top Leitner stage.
    Finished,
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
        // Fold the declared frontend (card override, else deck) and resolve each
        // card's image filenames to absolute paths against the deck's `img-dir`
        // (or the deck file's own folder when none is set). No filesystem check:
        // a missing image must not stop the deck from loading.
        let base_dir = image_base_dir(&path, settings.img_dir.as_deref());
        for card in &mut cards {
            card.frontend = card.frontend.or(settings.frontend);
            card.image = card.image.take().map(|p| resolve_image(&base_dir, p));
            card.image_back = card.image_back.take().map(|p| resolve_image(&base_dir, p));
            // Deck-wide top stage (`% max-stage:`); reaching it retires the card.
            card.max_stage = settings.max_stage;
        }
        // Expand the declared direction (card override, else deck) into cards.
        // `reverse` swaps the card, `both` adds the swapped one alongside; the
        // reversed card keeps the source line, so the session treats the pair as
        // siblings. Direction doesn't apply to cloze cards.
        let mut expanded = Vec::with_capacity(cards.len());
        for card in cards {
            let direction = card.direction.or(settings.direction).unwrap_or_default();
            if card.hash_lines.is_some() || direction == Direction::Forward {
                expanded.push(card);
            } else {
                let reversed = card.reversed();
                match direction {
                    Direction::Reverse => expanded.push(reversed),
                    Direction::Both => {
                        expanded.push(card);
                        expanded.push(reversed);
                    }
                    Direction::Forward => unreachable!("handled above"),
                }
            }
        }
        let cards = expanded;
        Ok(Self {
            path,
            subject,
            cards,
            links,
            requires,
            settings,
        })
    }

    /// The deck's completion state: `Finished` once every card is retired
    /// (reached its top stage; see [`session::is_retired`]), `NotStarted` while
    /// no card has been reviewed, `Started` in between. An empty deck is
    /// `NotStarted`.
    pub fn state(&self, store: &Store) -> DeckState {
        let total = self.cards.len();
        if total == 0 {
            return DeckState::NotStarted;
        }
        let retired = self
            .cards
            .iter()
            .filter(|c| session::is_retired(c, store))
            .count();
        if retired == total {
            DeckState::Finished
        } else if self.cards.iter().all(|c| store.get(c.id()).is_none()) {
            DeckState::NotStarted
        } else {
            DeckState::Started
        }
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

/// Finds the file a `% requires:` value refers to: as given, next to the
/// requiring deck, or in the decks directory; with or without a `.txt` suffix.
pub fn resolve_dep(
    req: &str,
    decks_dir: Option<&Path>,
    requiring_dir: Option<&Path>,
) -> Option<PathBuf> {
    let with_txt = |p: &Path| -> PathBuf {
        if p.extension().is_some() {
            p.to_path_buf()
        } else {
            p.with_extension("txt")
        }
    };
    let mut candidates = vec![PathBuf::from(req), with_txt(Path::new(req))];
    for dir in [requiring_dir, decks_dir].into_iter().flatten() {
        candidates.push(dir.join(req));
        candidates.push(with_txt(&dir.join(req)));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Whether `deck` is "locked": any of its transitive `% requires:` prerequisites
/// is not yet [`Finished`](DeckState::Finished). `decks_dir` resolves prerequisite
/// names. A missing prerequisite, an unreadable file, or a dependency cycle is
/// treated as non-blocking (a broken graph never hides a deck — those problems
/// surface when you actually review it).
pub fn is_locked(deck: &Deck, decks_dir: Option<&Path>, store: &Store) -> bool {
    fn prereqs_finished(
        deck: &Deck,
        decks_dir: Option<&Path>,
        store: &Store,
        visited: &mut HashSet<PathBuf>,
    ) -> bool {
        for req in &deck.requires {
            let Some(path) = resolve_dep(req, decks_dir, deck.path.parent()) else {
                continue; // missing prerequisite: don't lock on it
            };
            let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !visited.insert(key) {
                continue; // already checked, or a cycle: stop recursing
            }
            let Ok(prereq) = Deck::load(&path) else {
                continue; // unreadable prerequisite: don't lock on it
            };
            if prereq.state(store) != DeckState::Finished
                || !prereqs_finished(&prereq, decks_dir, store, visited)
            {
                return false;
            }
        }
        true
    }
    !prereqs_finished(deck, decks_dir, store, &mut HashSet::new())
}

/// The directory that card image filenames resolve against: the deck's
/// `% img-dir:` if set (made absolute against the deck file's folder when it is
/// itself relative), else the deck file's own folder.
fn image_base_dir(deck_path: &Path, img_dir: Option<&Path>) -> PathBuf {
    let deck_dir = deck_path.parent().unwrap_or_else(|| Path::new("."));
    match img_dir {
        Some(dir) if dir.is_absolute() => dir.to_path_buf(),
        Some(dir) => deck_dir.join(dir),
        None => deck_dir.to_path_buf(),
    }
}

/// Resolves one card image: an absolute value is used as-is; otherwise it is
/// joined onto the deck's image base directory.
fn resolve_image(base: &Path, image: PathBuf) -> PathBuf {
    if image.is_absolute() {
        image
    } else {
        base.join(image)
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
    use crate::store::MAX_STAGE;

    fn write_deck(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn empty_store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        (store, dir)
    }

    /// Drives a card to retirement: top stage, reached by passing (streak ≥ 1).
    fn retire(store: &mut Store, id: u64) {
        let s = store.get_or_insert(id, 0);
        s.stage = MAX_STAGE;
        s.streak = 1;
    }

    #[test]
    fn deck_state_reflects_card_stages() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.txt", "# a\n\t1\n# b\n\t2\n");
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        assert_eq!(DeckState::NotStarted, deck.state(&store));

        // One card seen but not retired -> started.
        store.get_or_insert(deck.cards[0].id(), 0).stage = 2;
        assert_eq!(DeckState::Started, deck.state(&store));

        // Every card retired (top stage, reached by passing) -> finished.
        for card in &deck.cards {
            let s = store.get_or_insert(card.id(), 0);
            s.stage = MAX_STAGE;
            s.streak = 1;
        }
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn empty_deck_is_not_started() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "e.txt", "% only a comment\n");
        let deck = Deck::load(&path).unwrap();
        let (store, _s) = empty_store();
        assert!(deck.cards.is_empty());
        assert_eq!(DeckState::NotStarted, deck.state(&store));
    }

    #[test]
    fn max_stage_directive_finishes_deck_at_lower_stage() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.txt", "% max-stage: 2\n# a\n\t1\n");
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(2), deck.settings.max_stage);
        assert_eq!(Some(2), deck.cards[0].max_stage); // stamped onto the card
        let (mut store, _s) = empty_store();

        // Stage 2 reached by passing -> retired -> finished, below MAX_STAGE.
        let s = store.get_or_insert(deck.cards[0].id(), 0);
        s.stage = 2;
        s.streak = 1;
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn locked_until_prerequisite_finished() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "basics.txt", "# a\n\t1\n");
        let adv = write_deck(dir.path(), "advanced.txt", "% requires: basics\n# x\n\ty\n");
        let advanced = Deck::load(&adv).unwrap();
        let basics = Deck::load(dir.path().join("basics.txt")).unwrap();
        let (mut store, _s) = empty_store();
        let dd = Some(dir.path());

        // basics not started -> advanced locked.
        assert!(is_locked(&advanced, dd, &store));
        // basics started but not finished -> still locked.
        store.get_or_insert(basics.cards[0].id(), 0).stage = 2;
        assert!(is_locked(&advanced, dd, &store));
        // basics finished -> advanced unlocked.
        retire(&mut store, basics.cards[0].id());
        assert!(!is_locked(&advanced, dd, &store));
        // A deck without prerequisites is never locked.
        assert!(!is_locked(&basics, dd, &store));
    }

    #[test]
    fn locking_is_transitive() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.txt", "# a\n\t1\n");
        write_deck(dir.path(), "b.txt", "% requires: a\n# b\n\t2\n");
        let cpath = write_deck(dir.path(), "c.txt", "% requires: b\n# c\n\t3\n");
        let c = Deck::load(&cpath).unwrap();
        let a = Deck::load(dir.path().join("a.txt")).unwrap();
        let b = Deck::load(dir.path().join("b.txt")).unwrap();
        let (mut store, _s) = empty_store();
        let dd = Some(dir.path());

        assert!(is_locked(&c, dd, &store));
        // Finishing only the direct prerequisite is not enough — its own
        // prerequisite is still unfinished.
        retire(&mut store, b.cards[0].id());
        assert!(is_locked(&c, dd, &store));
        // Finish the foundation too -> unlocked.
        retire(&mut store, a.cards[0].id());
        assert!(!is_locked(&c, dd, &store));
    }

    #[test]
    fn missing_prerequisite_does_not_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.txt", "% requires: nope\n# a\n\t1\n");
        let deck = Deck::load(&path).unwrap();
        let (store, _s) = empty_store();
        assert!(!is_locked(&deck, Some(dir.path()), &store));
    }

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
    fn explain_mode_directive_parses_and_stamps_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("u.txt");
        std::fs::write(&path, "% mode: explain\n# why?\n\tpoint one\n\tpoint two\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Mode::Explain), deck.settings.mode);
        assert_eq!(Some(Mode::Explain), deck.cards[0].mode); // stamped onto the card
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
    fn direction_both_expands_to_forward_and_reverse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "# purported\n% direction: both\n\tangeblich\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(2, deck.cards.len());
        assert_eq!("purported", deck.cards[0].front);
        assert_eq!(vec!["angeblich"], deck.cards[0].back);
        assert_eq!("angeblich", deck.cards[1].front);
        assert_eq!(vec!["purported"], deck.cards[1].back);
        assert_eq!(deck.cards[0].line, deck.cards[1].line); // sibling group
        assert_ne!(deck.cards[0].id(), deck.cards[1].id());
    }

    #[test]
    fn direction_reverse_keeps_only_the_swapped_card() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "# q\n% direction: reverse\n\ta\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!("a", deck.cards[0].front);
        assert_eq!(vec!["q"], deck.cards[0].back);
    }

    #[test]
    fn deck_level_direction_applies_to_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "% direction: both\n# a\n\tb\n").unwrap();
        assert_eq!(2, Deck::load(&path).unwrap().cards.len());
    }

    #[test]
    fn direction_does_not_apply_to_cloze() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        // Deck-level `both` must not reverse a cloze card (one hole -> one card).
        std::fs::write(&path, "% direction: both\n#? fill\n\tThe {{x}} thing.\n").unwrap();
        assert_eq!(1, Deck::load(&path).unwrap().cards.len());
    }

    #[test]
    fn image_resolves_against_img_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(
            &path,
            "% img-dir: /assets/imgs\n# q\n% img: moon.png\n\tWaxing\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            Some(PathBuf::from("/assets/imgs/moon.png")),
            deck.cards[0].image
        );
        assert_eq!(Frontend::Web, deck.cards[0].frontend()); // image -> web
    }

    #[test]
    fn image_resolves_against_deck_dir_without_img_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "# q\n% img: moon.png\n\tWaxing\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(dir.path().join("moon.png")), deck.cards[0].image);
    }

    #[test]
    fn absolute_card_image_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(
            &path,
            "% img-dir: /assets\n# q\n% img: /elsewhere/moon.png\n\tWaxing\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            Some(PathBuf::from("/elsewhere/moon.png")),
            deck.cards[0].image
        );
    }

    #[test]
    fn deck_level_frontend_applies_to_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.txt");
        std::fs::write(&path, "% frontend: web\n# a\n\tb\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Frontend::Web), deck.cards[0].frontend);
        assert_eq!(Frontend::Web, deck.cards[0].frontend());
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
