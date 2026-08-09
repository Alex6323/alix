use std::path::{Path, PathBuf};

use crate::{
    card::Card,
    deck::{Deck, DeckError, write_deck_text},
    parser::{DECK_FORMAT_VERSION, PERSONAL_PARENT_KEY, SidecarNote},
    sidecar::SidecarBlock,
};

pub fn sidecar_path(deck: &Path) -> PathBuf {
    deck.with_extension("personal.md")
}

#[derive(Debug, Default)]
pub struct Personal {
    pub cards: Vec<Card>,
    pub notes: Vec<SidecarNote>,
}

impl Personal {
    /// Cards then notes; `merge` walks each kind in its own pass.
    pub fn blocks(&self) -> Vec<SidecarBlock> {
        let cards = self.cards.iter().map(|card| SidecarBlock::Card {
            id: card.id().unwrap_or_default(),
            notes: Vec::new(),
        });
        let notes = self.notes.iter().map(|note| SidecarBlock::Note {
            card: note.card.clone(),
            lines: note.lines.clone(),
        });
        cards.chain(notes).collect()
    }
}

/// The personal file beside a deck. A missing, unreadable, or unparseable
/// sidecar reads as empty rather than failing the session that asked for it.
/// An unstamped card is dropped: nothing can schedule it or address a note to
/// it, so `cards` is exactly the addressable set.
pub fn read(deck_path: &Path, subject: &str) -> Personal {
    let Ok(text) = std::fs::read_to_string(sidecar_path(deck_path)) else {
        return Personal::default();
    };
    Personal {
        cards: crate::parser::parse_str(subject, &crate::parser::without_notes(&text))
            .unwrap_or_default()
            .into_iter()
            .filter(|card| card.id().is_some())
            .collect(),
        notes: crate::parser::notes(&text),
    }
}

pub fn card_ids(deck: &Deck) -> Vec<String> {
    read(&deck.path, &deck.subject)
        .cards
        .iter()
        .filter_map(Card::id)
        .collect()
}

pub fn append_note(
    deck: &Path,
    deck_id: &str,
    card_id: &str,
    notes: &[String],
) -> Result<(), DeckError> {
    if notes.is_empty() {
        return Ok(());
    }
    let path = sidecar_path(deck);
    let io_err = |source| DeckError::Io {
        path: path.clone(),
        source,
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => header(deck_id),
        Err(e) => return Err(io_err(e)),
    };
    write_deck_text(&path, &rewrite(&text, card_id, notes))
}

fn header(deck_id: &str) -> String {
    format!("---\nformat-version: {DECK_FORMAT_VERSION}\n{PERSONAL_PARENT_KEY}: {deck_id}\n---\n")
}

fn marker(card_id: &str) -> String {
    format!("<!-- note: {card_id} -->")
}

fn rewrite(text: &str, card_id: &str, notes: &[String]) -> String {
    let quoted = |note: &String| format!("> {note}");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let Some(marker_line) = lines.iter().position(|line| addresses(line, card_id)) else {
        let mut out = text.trim_end().to_string();
        out.push_str("\n\n");
        out.push_str(&marker(card_id));
        out.push('\n');
        for note in notes {
            out.push_str(&quoted(note));
            out.push('\n');
        }
        return out;
    };
    let mut end = marker_line + 1;
    while lines
        .get(end)
        .is_some_and(|line| line.trim_start().starts_with('>'))
    {
        end += 1;
    }
    for (offset, note) in notes.iter().enumerate() {
        lines.insert(end + offset, quoted(note));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn addresses(line: &str, card_id: &str) -> bool {
    let Some(body) = line
        .trim()
        .strip_prefix("<!--")
        .and_then(|l| l.strip_suffix("-->"))
    else {
        return false;
    };
    let Some(value) = body.trim().strip_prefix("note:") else {
        return false;
    };
    value.trim() == card_id
}

/// Appends already-stamped card blocks to the sidecar, creating it on first
/// write. The authored deck is never opened.
pub fn append_cards(deck: &Path, deck_id: &str, blocks: &str) -> Result<(), DeckError> {
    let blocks = blocks.trim();
    if blocks.is_empty() {
        return Ok(());
    }
    let path = sidecar_path(deck);
    let io_err = |source| DeckError::Io {
        path: path.clone(),
        source,
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => header(deck_id),
        Err(e) => return Err(io_err(e)),
    };
    let mut out = text.trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(blocks);
    out.push('\n');
    write_deck_text(&path, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck(dir: &Path) -> PathBuf {
        let deck = dir.join("spanish.md");
        std::fs::write(
            &deck,
            "---\nformat-version: 1\nid: deck-abc\n---\n## darse cuenta <!-- id: card-one -->\nto realise\n",
        )
        .unwrap();
        deck
    }

    fn note(text: &str) -> Vec<String> {
        vec![text.to_string()]
    }

    #[test]
    fn a_sidecar_sits_beside_its_deck_under_the_personal_suffix() {
        assert_eq!(
            Path::new("/decks/spanish.personal.md"),
            sidecar_path(Path::new("/decks/spanish.md"))
        );
    }

    #[test]
    fn the_first_note_creates_a_sidecar_naming_the_deck_it_belongs_to() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());

        append_note(&deck, "deck-abc", "card-one", &note("mine")).unwrap();

        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        assert_eq!(
            "---\nformat-version: 1\nfor: deck-abc\n---\n\n\
             <!-- note: card-one -->\n> mine\n",
            text
        );
    }

    #[test]
    fn writing_a_note_never_touches_the_authored_deck() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());
        let before = std::fs::read(&deck).unwrap();

        append_note(&deck, "deck-abc", "card-one", &note("mine")).unwrap();

        assert_eq!(
            before,
            std::fs::read(&deck).unwrap(),
            "the authored deck must be byte-identical after a note is written"
        );
    }

    #[test]
    fn a_second_note_for_one_card_extends_that_block_rather_than_repeating_it() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());

        append_note(&deck, "deck-abc", "card-one", &note("first")).unwrap();
        append_note(&deck, "deck-abc", "card-one", &note("second")).unwrap();

        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        assert_eq!(1, text.matches("<!-- note: card-one").count());
        let first = text.find("> first").unwrap();
        let second = text.find("> second").unwrap();
        assert!(first < second, "notes keep the order they were written in");
    }

    #[test]
    fn every_note_written_to_a_block_stays_below_its_opening_marker() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());

        append_note(&deck, "deck-abc", "card-one", &note("first")).unwrap();
        append_note(&deck, "deck-abc", "card-one", &note("second")).unwrap();

        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        let block: Vec<&str> = text
            .lines()
            .skip_while(|line| *line != "<!-- note: card-one -->")
            .collect();
        assert_eq!(
            vec!["<!-- note: card-one -->", "> first", "> second"],
            block,
            "the marker opens the block and every note follows it: {text}"
        );
        assert_eq!(1, text.matches("card-one").count());
    }

    #[test]
    fn a_note_for_another_card_opens_its_own_block() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());

        append_note(&deck, "deck-abc", "card-one", &note("mine")).unwrap();
        append_note(&deck, "deck-abc", "card-two", &note("other")).unwrap();

        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        let parsed = crate::parser::notes(&text);
        assert_eq!(
            vec!["card-one".to_string(), "card-two".to_string()],
            parsed.iter().map(|n| n.card.clone()).collect::<Vec<_>>()
        );
        assert_eq!(vec!["mine".to_string()], parsed[0].lines);
        assert_eq!(vec!["other".to_string()], parsed[1].lines);
    }

    #[test]
    fn an_empty_note_list_writes_no_sidecar_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());

        append_note(&deck, "deck-abc", "card-one", &[]).unwrap();

        assert!(!sidecar_path(&deck).exists());
    }

    #[test]
    fn a_personal_card_lands_in_the_sidecar_not_the_deck() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());
        let before = std::fs::read(&deck).unwrap();

        append_cards(
            &deck,
            "deck-abc",
            "## a gap the exam found <!-- id: card-two -->\nthe answer\n",
        )
        .unwrap();

        assert_eq!(before, std::fs::read(&deck).unwrap());
        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        assert!(
            text.starts_with("---\nformat-version: 1\nfor: deck-abc\n---\n"),
            "{text}"
        );
        let cards = crate::parser::parse_str("sidecar", &text).unwrap();
        assert_eq!(1, cards.len());
        assert_eq!(Some("card-two".to_string()), cards[0].id());
    }

    #[test]
    fn a_second_card_joins_the_first_without_disturbing_it() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());

        append_cards(&deck, "deck-abc", "## one <!-- id: card-aa -->\nx\n").unwrap();
        append_cards(&deck, "deck-abc", "## two <!-- id: card-bb -->\ny\n").unwrap();

        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        let cards = crate::parser::parse_str("sidecar", &text).unwrap();
        assert_eq!(
            vec![Some("card-aa".to_string()), Some("card-bb".to_string())],
            cards.iter().map(|c| c.id()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_card_and_a_note_share_one_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());

        append_note(&deck, "deck-abc", "card-one", &note("a note")).unwrap();
        append_cards(&deck, "deck-abc", "## mine <!-- id: card-zz -->\nback\n").unwrap();

        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        assert_eq!(1, crate::parser::notes(&text).len());
        assert_eq!(1, crate::parser::parse_str("sidecar", &text).unwrap().len());
    }

    #[test]
    fn empty_card_text_writes_no_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());
        append_cards(&deck, "deck-abc", "   \n").unwrap();
        assert!(!sidecar_path(&deck).exists());
    }

    /// Only a missing sidecar means "start one". Every other read failure is
    /// the reader's own file being unreachable, and writing a fresh one over
    /// it would take their notes with it.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_sidecar_errors_instead_of_being_replaced() {
        use std::os::unix::fs::PermissionsExt;

        let original =
            "---\nformat-version: 1\nfor: deck-abc\n---\n\n<!-- note: card-one -->\n> mine\n";
        for writer in ["append_note", "append_cards"] {
            let dir = tempfile::tempdir().unwrap();
            let deck = deck(dir.path());
            let sidecar = sidecar_path(&deck);
            std::fs::write(&sidecar, original).unwrap();
            std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o222)).unwrap();

            let result = match writer {
                "append_note" => append_note(&deck, "deck-abc", "card-one", &note("added")),
                _ => append_cards(&deck, "deck-abc", "## new <!-- id: card-two -->\nback\n"),
            };

            std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(
                matches!(result, Err(DeckError::Io { .. })),
                "{writer}: {result:?}"
            );
            assert_eq!(
                original,
                std::fs::read_to_string(&sidecar).unwrap(),
                "{writer}: the file it could not read is still the reader's"
            );
        }
    }

    #[test]
    fn several_notes_appended_at_once_keep_their_written_order() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());

        append_note(&deck, "deck-abc", "card-one", &note("first")).unwrap();
        append_note(
            &deck,
            "deck-abc",
            "card-one",
            &["second".to_string(), "third".to_string()],
        )
        .unwrap();

        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        let quoted: Vec<&str> = text.lines().filter(|line| line.starts_with('>')).collect();
        assert_eq!(vec!["> first", "> second", "> third"], quoted);
    }
}
