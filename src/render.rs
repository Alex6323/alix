//! Frontend-independent rendering model.
//!
//! Turns a [`Card`]'s free-text note into display structure — prose split into
//! sentences, fenced code blocks kept verbatim — without committing to any
//! frontend. There are no colors here, no width-based wrapping, no terminal
//! glyphs: just the structural decisions that every frontend would otherwise
//! have to reimplement. The ratatui TUI paints this model with a yellow left
//! bar; a future web frontend would paint the same model as HTML.
//!
//! A card's front, context, and back lines need no structuring — they are read
//! straight off the [`Card`] by each frontend. Only notes carry markup worth
//! interpreting once and sharing. Width-dependent wrapping is left to the
//! frontend (terminals wrap manually via [`wrap_text`]; the web lets CSS do
//! it), as is anything reveal-state-dependent (typing progress, line-by-line,
//! cloze holes), which belongs with the interactive layer.

use crate::card::Card;

/// A note decomposed into ordered display units.
///
/// Units appear in document order. Frontends separate consecutive units from
/// one another (the TUI inserts a blank gutter line between them) and apply
/// their own wrapping and styling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteUnit {
    /// One sentence of prose, trimmed, with its terminating period attached.
    /// Hard-wrapped `!` lines are joined before splitting, so a sentence is a
    /// real sentence rather than a source line.
    Sentence(String),
    /// A fenced code block: its lines verbatim, indentation preserved. Never
    /// sentence-split or wrapped — code reads as written.
    Code(Vec<String>),
}

/// Decomposes a card's note into ordered [`NoteUnit`]s. Returns an empty list
/// when the card has no note (or only a blank one).
///
/// Consecutive prose lines are joined into one buffer and then split into
/// sentences, so a note hard-wrapped across several `!` lines does not turn
/// each source line into its own "sentence". A ```` ``` ```` fence toggles a
/// verbatim code block.
pub fn note_units(card: &Card) -> Vec<NoteUnit> {
    let Some(note) = &card.note else {
        return Vec::new();
    };

    let mut units = Vec::new();
    let mut in_code = false;
    let mut code: Vec<String> = Vec::new();
    let mut prose = String::new();

    for logical in note.lines() {
        if logical.trim_start().starts_with("```") {
            // Fence delimiter: toggle code mode; the ``` line itself is
            // dropped. Empty blocks produce no unit.
            if in_code {
                let block = std::mem::take(&mut code);
                if !block.is_empty() {
                    units.push(NoteUnit::Code(block));
                }
                in_code = false;
            } else {
                flush_prose(&mut prose, &mut units);
                in_code = true;
                code.clear();
            }
            continue;
        }
        if in_code {
            code.push(logical.to_string());
            continue;
        }
        let trimmed = logical.trim();
        if !trimmed.is_empty() {
            if !prose.is_empty() {
                prose.push(' ');
            }
            prose.push_str(trimmed);
        }
    }

    flush_prose(&mut prose, &mut units);
    // An unterminated code fence still yields its gathered lines.
    if !code.is_empty() {
        units.push(NoteUnit::Code(code));
    }
    units
}

/// Splits the accumulated prose into one [`NoteUnit::Sentence`] per sentence
/// and clears the buffer.
fn flush_prose(prose: &mut String, units: &mut Vec<NoteUnit>) {
    for sentence in split_sentences(prose) {
        if !sentence.is_empty() {
            units.push(NoteUnit::Sentence(sentence));
        }
    }
    prose.clear();
}

/// Splits text into sentences, breaking after a period that is followed by
/// whitespace or the end of the text. A period followed by a non-space (as in
/// "2.1") does not split, so numbers stay intact. The terminating period stays
/// attached to its sentence.
pub fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut sentences = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        let ends_sentence = chars[i] == '.' && chars.get(i + 1).is_none_or(|c| c.is_whitespace());
        if ends_sentence {
            let sentence: String = chars[start..=i].iter().collect();
            if !sentence.trim().is_empty() {
                sentences.push(sentence.trim().to_string());
            }
            start = i + 1;
        }
    }
    if start < chars.len() {
        let tail: String = chars[start..].iter().collect();
        if !tail.trim().is_empty() {
            sentences.push(tail.trim().to_string());
        }
    }
    sentences
}

/// Greedy word-wrap to `width` columns (counted in chars). Returns at least
/// one row, so a blank line still renders. A word longer than `width` (e.g. a
/// long Move type path) is hard-broken across rows.
pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let wlen = word.chars().count();
        if line.is_empty() {
            // place `word` below
        } else if line.chars().count() + 1 + wlen <= width {
            line.push(' ');
            line.push_str(word);
            continue;
        } else {
            rows.push(std::mem::take(&mut line));
        }
        if wlen <= width {
            line.push_str(word);
        } else {
            for ch in word.chars() {
                if line.chars().count() == width {
                    rows.push(std::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
    }
    rows.push(line);
    rows
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn card_with_note(note: &str) -> Card {
        Card::plain(
            Arc::from("s.txt"),
            "front".to_string(),
            vec!["back".to_string()],
            Some(note.to_string()),
            1,
        )
    }

    #[test]
    fn no_note_yields_no_units() {
        let card = Card::plain(Arc::from("s.txt"), "f".into(), vec!["b".into()], None, 1);
        assert!(note_units(&card).is_empty());
    }

    #[test]
    fn prose_splits_into_sentences() {
        let units = note_units(&card_with_note("First one. Second one."));
        assert_eq!(
            units,
            vec![
                NoteUnit::Sentence("First one.".into()),
                NoteUnit::Sentence("Second one.".into()),
            ]
        );
    }

    #[test]
    fn hard_wrapped_prose_joins_before_splitting() {
        // Two source lines, one sentence: they must join, not become two.
        let units = note_units(&card_with_note("A sentence spread\nacross two lines."));
        assert_eq!(
            units,
            vec![NoteUnit::Sentence("A sentence spread across two lines.".into())]
        );
    }

    #[test]
    fn code_block_is_verbatim() {
        let note = "Intro here.\n```\nfn main() {\n    let x = 1;\n}\n```";
        let units = note_units(&card_with_note(note));
        assert_eq!(
            units,
            vec![
                NoteUnit::Sentence("Intro here.".into()),
                NoteUnit::Code(vec![
                    "fn main() {".into(),
                    "    let x = 1;".into(),
                    "}".into(),
                ]),
            ]
        );
    }

    #[test]
    fn prose_after_code_is_its_own_unit() {
        let note = "```\ncode\n```\nAfter the block.";
        let units = note_units(&card_with_note(note));
        assert_eq!(
            units,
            vec![
                NoteUnit::Code(vec!["code".into()]),
                NoteUnit::Sentence("After the block.".into()),
            ]
        );
    }

    #[test]
    fn unterminated_fence_still_yields_code() {
        let units = note_units(&card_with_note("```\nlonely line"));
        assert_eq!(units, vec![NoteUnit::Code(vec!["lonely line".into()])]);
    }

    #[test]
    fn period_in_number_does_not_split() {
        let units = note_units(&card_with_note("See section 2.1 for details."));
        assert_eq!(
            units,
            vec![NoteUnit::Sentence("See section 2.1 for details.".into())]
        );
    }

    #[test]
    fn short_line_is_one_row() {
        assert_eq!(wrap_text("a short note", 40), vec!["a short note"]);
    }

    #[test]
    fn wraps_on_word_boundaries() {
        assert_eq!(wrap_text("a bb ccc", 4), vec!["a bb", "ccc"]);
    }

    #[test]
    fn hard_breaks_a_word_longer_than_width() {
        assert_eq!(wrap_text("ab supercali", 5), vec!["ab", "super", "cali"]);
    }

    #[test]
    fn empty_line_yields_one_empty_row() {
        assert_eq!(wrap_text("", 10), vec![""]);
    }

    #[test]
    fn zero_width_does_not_panic() {
        assert_eq!(
            wrap_text("hi there", 0),
            vec!["h", "i", "t", "h", "e", "r", "e"]
        );
    }
}
