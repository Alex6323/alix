//! Rendering model for the web frontend.
//!
//! Turns a [`Card`]'s free-text note into display structure — prose split into
//! sentences, fenced code blocks kept verbatim — without committing to a
//! particular layout. There are no colors here, no width-based wrapping: just
//! the structural decisions the frontend would otherwise have to reimplement.
//! The web page paints this model as HTML.
//!
//! A card's front, context, and back lines need no structuring — they are read
//! straight off the [`Card`]. Only notes carry markup worth interpreting once
//! and sharing. Width-dependent wrapping (CSS on the web) is left to the
//! frontend, as is anything reveal-state-dependent (typing progress,
//! line-by-line, cloze holes), which belongs with the interactive layer.

use serde::{Deserialize, Serialize};

use crate::{
    card::Card,
    l1::{BLANK, HIDDEN},
};

/// A note decomposed into ordered display units.
///
/// Units appear in document order. The frontend separates consecutive units
/// from one another and applies its own wrapping and styling. Serializes as
/// the documented client wire shape (`{"kind": "sentence", "text": …}`, see
/// docs/API.md): struct variants, because serde's internal tagging cannot
/// tag newtype variants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum NoteUnit {
    /// One sentence of prose, trimmed, with its terminating period attached.
    /// Hard-wrapped `!` lines are joined before splitting, so a sentence is a
    /// real sentence rather than a source line.
    Sentence { text: String },
    /// A fenced code block: its lines verbatim, indentation preserved. Never
    /// sentence-split or wrapped — code reads as written.
    Code { lines: Vec<String> },
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
                    units.push(NoteUnit::Code { lines: block });
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
        units.push(NoteUnit::Code { lines: code });
    }
    units
}

/// Splits the accumulated prose into one [`NoteUnit::Sentence`] per sentence
/// and clears the buffer.
fn flush_prose(prose: &mut String, units: &mut Vec<NoteUnit>) {
    for sentence in split_sentences(prose) {
        if !sentence.is_empty() {
            units.push(NoteUnit::Sentence { text: sentence });
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

/// A piece of a cloze context line, so frontends can highlight the hole the
/// sub-card is asking and dim the hidden sibling holes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextSpan {
    /// Ordinary surrounding text.
    Text(String),
    /// The active hole — the blank this sub-card is asking for ([`BLANK`]).
    Blank(String),
    /// A hidden sibling hole, masked so it doesn't spoil itself ([`HIDDEN`]).
    Hidden(String),
}

/// Splits a cloze context line into [`ContextSpan`]s by its hole markers. A
/// line with no markers yields a single [`ContextSpan::Text`].
pub fn context_spans(line: &str) -> Vec<ContextSpan> {
    let mut spans = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        // The earliest of the two markers wins this iteration.
        let blank = rest.find(BLANK);
        let hidden = rest.find(HIDDEN);
        let (pos, marker, is_blank) = match (blank, hidden) {
            (None, None) => {
                spans.push(ContextSpan::Text(rest.to_string()));
                break;
            }
            (Some(b), None) => (b, BLANK, true),
            (None, Some(h)) => (h, HIDDEN, false),
            (Some(b), Some(h)) if b <= h => (b, BLANK, true),
            (Some(_), Some(h)) => (h, HIDDEN, false),
        };
        if pos > 0 {
            spans.push(ContextSpan::Text(rest[..pos].to_string()));
        }
        let seg = marker.to_string();
        spans.push(if is_blank {
            ContextSpan::Blank(seg)
        } else {
            ContextSpan::Hidden(seg)
        });
        rest = &rest[pos + marker.len()..];
    }
    spans
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
                NoteUnit::Sentence {
                    text: "First one.".into()
                },
                NoteUnit::Sentence {
                    text: "Second one.".into()
                },
            ]
        );
    }

    #[test]
    fn hard_wrapped_prose_joins_before_splitting() {
        // Two source lines, one sentence: they must join, not become two.
        let units = note_units(&card_with_note("A sentence spread\nacross two lines."));
        assert_eq!(
            units,
            vec![NoteUnit::Sentence {
                text: "A sentence spread across two lines.".into()
            }]
        );
    }

    #[test]
    fn code_block_is_verbatim() {
        let note = "Intro here.\n```\nfn main() {\n    let x = 1;\n}\n```";
        let units = note_units(&card_with_note(note));
        assert_eq!(
            units,
            vec![
                NoteUnit::Sentence {
                    text: "Intro here.".into()
                },
                NoteUnit::Code {
                    lines: vec!["fn main() {".into(), "    let x = 1;".into(), "}".into(),]
                },
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
                NoteUnit::Code {
                    lines: vec!["code".into()]
                },
                NoteUnit::Sentence {
                    text: "After the block.".into()
                },
            ]
        );
    }

    #[test]
    fn unterminated_fence_still_yields_code() {
        let units = note_units(&card_with_note("```\nlonely line"));
        assert_eq!(
            units,
            vec![NoteUnit::Code {
                lines: vec!["lonely line".into()]
            }]
        );
    }

    #[test]
    fn period_in_number_does_not_split() {
        let units = note_units(&card_with_note("See section 2.1 for details."));
        assert_eq!(
            units,
            vec![NoteUnit::Sentence {
                text: "See section 2.1 for details.".into()
            }]
        );
    }

    #[test]
    fn note_units_serialize_as_the_documented_wire_shape() {
        // The client contract (docs/API.md, pinned by the serve snapshot
        // tests): internally tagged with `kind`, lowercase variant names.
        let units = vec![
            NoteUnit::Sentence {
                text: "One owner.".into(),
            },
            NoteUnit::Code {
                lines: vec!["let s;".into()],
            },
        ];
        assert_eq!(
            serde_json::json!([
                {"kind": "sentence", "text": "One owner."},
                {"kind": "code", "lines": ["let s;"]},
            ]),
            serde_json::to_value(&units).unwrap()
        );
    }

    #[test]
    fn context_spans_split_holes() {
        use ContextSpan::*;
        assert_eq!(context_spans("plain text"), vec![Text("plain text".into())]);
        assert_eq!(
            context_spans("To ____ or not to […]"),
            vec![
                Text("To ".into()),
                Blank("____".into()),
                Text(" or not to ".into()),
                Hidden("[…]".into()),
            ]
        );
        // A hole at the very start, no leading text.
        assert_eq!(
            context_spans("____ here"),
            vec![Blank("____".into()), Text(" here".into())]
        );
    }
}
