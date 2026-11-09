use std::path::{Path, PathBuf};

use crate::{
    card::Card,
    deck::{Deck, DeckError, write_deck_text},
    parser::{PERSONAL_PARENT_KEY, ParsedSidecarCard, SidecarNote},
    sidecar::SidecarBlock,
};

pub fn sidecar_path(deck: &Path) -> PathBuf {
    deck.with_extension(crate::workspace::LOCAL_SIDECAR_SUFFIX.trim_start_matches('.'))
}

#[derive(Debug, Default)]
pub struct Personal {
    pub cards: Vec<ParsedSidecarCard>,
    pub skipped: Vec<Card>,
    pub notes: Vec<SidecarNote>,
    pub warnings: Vec<String>,
}

impl Personal {
    /// Cards, then skipped cards, then notes; `merge` walks each kind in its
    /// own pass.
    pub fn blocks(&self) -> Vec<SidecarBlock> {
        let cards = self
            .cards
            .iter()
            .map(|parsed| &parsed.card)
            .chain(&self.skipped)
            .map(|card| SidecarBlock::Card {
                id: card.id().unwrap_or_default(),
                notes: Vec::new(),
            });
        let notes = self.notes.iter().map(|note| SidecarBlock::Note {
            card: note.card.clone(),
            note: note.note.clone(),
        });
        cards.chain(notes).collect()
    }
}

/// The personal file beside a deck. A missing, unreadable, or unparseable
/// sidecar reads as empty rather than failing the session that asked for it.
/// An unstamped card is dropped: nothing can schedule it or address a note to
/// it, so `cards` is exactly the addressable set.
pub fn read(deck_path: &Path, subject: &str) -> Personal {
    crate::profile::hit(crate::profile::Counter::SidecarReads);
    let path = sidecar_path(deck_path);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Personal::default();
    };
    let (stripped, file_lines) = crate::parser::without_notes_mapped(&text);
    let file_line = |line: usize| {
        line.checked_sub(1)
            .and_then(|index| file_lines.get(index))
            .copied()
            .unwrap_or(line)
    };
    let parsed = crate::parser::parse_sidecar(subject, &stripped).unwrap_or_default();
    Personal {
        warnings: parsed
            .problems
            .iter()
            .map(|problem| {
                format!(
                    "{}:{}: {}",
                    path.display(),
                    file_line(problem.line),
                    problem.kind.message()
                )
            })
            .collect(),
        skipped: parsed
            .skipped
            .into_iter()
            .filter(|card| card.id().is_some())
            .collect(),
        cards: parsed
            .cards
            .into_iter()
            .filter(|parsed| parsed.card.id().is_some())
            .map(|parsed| ParsedSidecarCard {
                topic_line: file_line(parsed.topic_line),
                ..parsed
            })
            .collect(),
        notes: crate::parser::notes(&text),
    }
}

pub fn card_ids(deck: &Deck) -> Vec<String> {
    let personal = read(&deck.path, &deck.subject);
    let cards: Vec<Card> = personal
        .cards
        .into_iter()
        .map(|parsed| parsed.card)
        .chain(personal.skipped)
        .collect();
    let mut ids: Vec<String> = cards.iter().filter_map(Card::id).collect();
    ids.extend(crate::card::dormant_base_ids(&cards));
    ids
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
    format!("---\n{PERSONAL_PARENT_KEY}: {deck_id}\n---\n")
}

fn marker(card_id: &str) -> String {
    format!("<!-- note: {card_id} -->")
}

/// A fresh note opens with the badge line the deck grammar asks for; later
/// lines for the same card join the run under it.
fn rewrite(text: &str, card_id: &str, notes: &[String]) -> String {
    let quoted = |note: &String| format!("> {note}");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let Some(marker_line) = lines.iter().position(|line| addresses(line, card_id)) else {
        let mut out = text.trim_end().to_string();
        out.push_str("\n\n");
        out.push_str(&marker(card_id));
        out.push_str("\n> [!NOTE]\n");
        for note in notes {
            out.push_str(&quoted(note));
            out.push('\n');
        }
        return out;
    };
    let start = marker_line + 1;
    let end = start
        + lines[start..]
            .iter()
            .take_while(|line| line.trim_start().starts_with('>'))
            .count();
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

pub fn rebind_topic(deck: &Path, base_ids: &[String], topic_id: &str) -> Result<(), DeckError> {
    let path = sidecar_path(deck);
    let text = std::fs::read_to_string(&path).map_err(|source| DeckError::Io {
        path: path.clone(),
        source,
    })?;
    let id_lines: Vec<String> = base_ids
        .iter()
        .map(|id| format!("<!-- id: {id} -->"))
        .collect();
    let mut lines: Vec<&str> = Vec::new();
    let topic = format!("<!-- topic: {topic_id} -->");
    for line in text.lines() {
        if id_lines.iter().any(|id_line| line.trim() == id_line) {
            if lines.last().is_some_and(|previous| is_topic_line(previous)) {
                lines.pop();
            }
            lines.push(&topic);
        }
        lines.push(line);
    }
    write_deck_text(&path, &format!("{}\n", lines.join("\n")))
}

fn is_topic_line(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("<!-- topic:") && line.ends_with("-->")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::Note;

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

    /// Codex's finding: a sidecar is reader-owned Markdown, so a
    /// hand-written deeper heading must stay content. Opening a sub-card
    /// there steals the stamped card's answer, and an orphan makes
    /// `read` swallow the parse error and return NO personal cards at all.
    #[test]
    fn a_sidecar_keeps_deeper_headings_as_card_content() {
        let text = "---\nformat-version: 1\nfor: deck-abc\n---\n\
                    ## personal\n\
                    answer\n\
                    ### hand-written detail\n\
                    more detail\n\
                    <!-- topic: topic-t -->\n\
                    <!-- id: card-personal -->\n";
        let cards = crate::parser::parse_sidecar("deck.local.md", text)
            .unwrap()
            .cards;
        assert_eq!(1, cards.len(), "sub-card syntax is disabled in a sidecar");
        assert_eq!(
            vec!["answer", "### hand-written detail", "more detail"],
            cards[0].card.back
        );
    }

    /// A deeper heading with no `## ` above it would orphan in a deck and
    /// empty the whole file; in a sidecar it is simply content.
    #[test]
    fn a_leading_deep_heading_never_empties_a_sidecar() {
        let text =
            "## personal\nanswer\n#### deep\ntail\n<!-- topic: topic-t -->\n<!-- id: card-p2 -->\n";
        let cards = crate::parser::parse_sidecar("deck.local.md", text)
            .unwrap()
            .cards;
        assert_eq!(1, cards.len());
    }

    #[test]
    fn an_unstamped_sidecar_card_never_becomes_an_addressable_block() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());
        std::fs::write(
            sidecar_path(&deck),
            "---\nfor: deck-abc\n---\n## draft\nanswer\n",
        )
        .unwrap();

        let personal = read(&deck, "spanish");
        assert!(personal.cards.is_empty());
        assert!(personal.blocks().is_empty());
    }

    #[test]
    fn a_missing_topic_line_warning_names_the_file_line_past_a_note_block() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());
        std::fs::write(
            sidecar_path(&deck),
            "---\nfor: deck-abc\n---\n\n\
             <!-- note: card-one -->\n> [!NOTE]\n> mine\n\n\
             ## no topic\nback\n<!-- id: card-bad -->\n",
        )
        .unwrap();

        let personal = read(&deck, "spanish");

        assert_eq!(
            vec![format!(
                "{}:11: personal card has no topic line",
                sidecar_path(&deck).display()
            )],
            personal.warnings,
            "the id line sits on line 11 of the file, past the dropped note block"
        );
        assert!(personal.cards.is_empty(), "{:?}", personal.cards);
    }

    #[test]
    fn a_topic_line_not_directly_before_the_id_line_skips_the_card_with_a_warning() {
        let good = "## good\nok\n<!-- topic: deck-abc -->\n<!-- id: card-good -->\n";
        for (row, bad, line, text) in [
            (
                "misplaced",
                "## bad\nback\n<!-- topic: deck-abc -->\n<!-- input: draw -->\n<!-- id: card-bad -->\n",
                7,
                "personal card's topic line is not directly before its id line",
            ),
            (
                "second",
                "## bad\nback\n<!-- topic: deck-abc -->\n<!-- topic: deck-abc -->\n<!-- id: card-bad -->\n",
                8,
                "personal card has a second topic line",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let deck = deck(dir.path());
            std::fs::write(
                sidecar_path(&deck),
                format!("---\nfor: deck-abc\n---\n\n{bad}\n{good}"),
            )
            .unwrap();

            let personal = read(&deck, "spanish");

            assert_eq!(
                vec![Some("card-good".to_string())],
                personal
                    .cards
                    .iter()
                    .map(|parsed| parsed.card.id())
                    .collect::<Vec<_>>(),
                "{row}: the bad card is skipped alone"
            );
            assert_eq!(
                vec![format!("{}:{line}: {text}", sidecar_path(&deck).display())],
                personal.warnings,
                "{row}: the warning names the topic line"
            );
        }
    }

    #[test]
    fn a_sidecar_sits_beside_its_deck_under_the_personal_suffix() {
        assert_eq!(
            Path::new("/decks/spanish.local.md"),
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
            "---\nfor: deck-abc\n---\n\n\
             <!-- note: card-one -->\n> [!NOTE]\n> mine\n",
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
    fn a_fenced_code_block_in_a_note_survives_the_sidecar_and_renders_as_code() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());
        let lines: Vec<String> = [
            "Shadowing creates a new binding.",
            "```rust",
            "let x = 1;",
            "let x = x + 1;",
            "```",
        ]
        .iter()
        .map(|l| l.to_string())
        .collect();

        append_note(&deck, "deck-abc", "card-one", &lines).unwrap();

        let personal = read(&deck, "spanish.md");
        let note = &personal.notes[0];
        assert_eq!("card-one", note.card);
        assert_eq!(
            lines.join("\n"),
            note.note.body,
            "the fence lines come back verbatim"
        );
        let card = Card::plain(
            std::sync::Arc::from("spanish.md"),
            "front".to_string(),
            vec!["back".to_string()],
            vec![note.note.clone()],
            1,
        );
        let units = crate::render::note_views(&card)
            .into_iter()
            .next()
            .map(|view| view.units)
            .unwrap_or_default();
        assert!(
            units.iter().any(|unit| matches!(
                unit,
                crate::render::ContentUnit::Code { lines }
                    if lines == &["let x = 1;".to_string(), "let x = x + 1;".to_string()]
            )),
            "the fence renders as one code unit: {units:?}"
        );
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
            vec![
                "<!-- note: card-one -->",
                "> [!NOTE]",
                "> first",
                "> second"
            ],
            block,
            "the marker opens the block, the badge follows, then every note: {text}"
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
        assert_eq!(Note::plain("mine".to_string()), parsed[0].note);
        assert_eq!(Note::plain("other".to_string()), parsed[1].note);
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
            "## a gap the exam found\nthe answer\n<!-- topic: topic-t -->\n<!-- id: card-two -->\n",
        )
        .unwrap();

        assert_eq!(before, std::fs::read(&deck).unwrap());
        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        assert!(text.starts_with("---\nfor: deck-abc\n---\n"), "{text}");
        let cards = crate::parser::parse_sidecar("sidecar", &crate::parser::without_notes(&text))
            .unwrap()
            .cards;
        assert_eq!(1, cards.len());
        assert_eq!(Some("card-two".to_string()), cards[0].card.id());
    }

    #[test]
    fn a_second_card_joins_the_first_without_disturbing_it() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());

        append_cards(
            &deck,
            "deck-abc",
            "## one\nx\n<!-- topic: topic-t -->\n<!-- id: card-aa -->\n",
        )
        .unwrap();
        append_cards(
            &deck,
            "deck-abc",
            "## two\ny\n<!-- topic: topic-t -->\n<!-- id: card-bb -->\n",
        )
        .unwrap();

        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        let cards = crate::parser::parse_sidecar("sidecar", &crate::parser::without_notes(&text))
            .unwrap()
            .cards;
        assert_eq!(
            vec![Some("card-aa".to_string()), Some("card-bb".to_string())],
            cards.iter().map(|c| c.card.id()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_card_and_a_note_share_one_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());

        append_note(&deck, "deck-abc", "card-one", &note("a note")).unwrap();
        append_cards(
            &deck,
            "deck-abc",
            "## mine\nback\n<!-- topic: topic-t -->\n<!-- id: card-zz -->\n",
        )
        .unwrap();

        let text = std::fs::read_to_string(sidecar_path(&deck)).unwrap();
        assert_eq!(1, crate::parser::notes(&text).len());
        assert_eq!(
            1,
            crate::parser::parse_sidecar("sidecar", &crate::parser::without_notes(&text))
                .unwrap()
                .cards
                .len()
        );
    }

    #[test]
    fn reading_a_personal_card_keeps_an_addressed_note_out_of_its_answer() {
        let dir = tempfile::tempdir().unwrap();
        let deck = deck(dir.path());
        std::fs::write(
            sidecar_path(&deck),
            "---\nfor: deck-abc\n---\n\n\
             ## mine\nback\n<!-- topic: topic-t -->\n<!-- id: card-zz -->\n\n\
             <!-- note: card-one -->\n> [!NOTE]\n> a note\n",
        )
        .unwrap();

        let personal = read(&deck, "spanish");

        assert_eq!(1, personal.cards.len());
        assert_eq!(vec!["back"], personal.cards[0].card.back);
        assert!(personal.cards[0].card.notes.is_empty());
        assert_eq!(1, personal.notes.len());
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

        let original = "---\nformat-version: 1\nfor: deck-abc\n---\n\n\
                        <!-- note: card-one -->\n> [!NOTE]\n> mine\n";
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
        assert_eq!(vec!["> [!NOTE]", "> first", "> second", "> third"], quoted);
    }
}
