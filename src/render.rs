use serde::{Deserialize, Serialize};

use crate::{
    card::Card,
    l1::{BLANK, HIDDEN},
};

// Struct variants (not newtype) because serde's internal tagging can't tag
// newtype variants; wire shape is `{"kind": ..., "text": ...}` (docs/API.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum NoteUnit {
    Sentence { text: String },
    Code { lines: Vec<String> },
}

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
            // Empty code blocks produce no unit.
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

fn flush_prose(prose: &mut String, units: &mut Vec<NoteUnit>) {
    for sentence in split_sentences(prose) {
        if !sentence.is_empty() {
            units.push(NoteUnit::Sentence { text: sentence });
        }
    }
    prose.clear();
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextSpan {
    Text(String),
    Blank(String),
    Hidden(String),
}

pub fn context_spans(line: &str) -> Vec<ContextSpan> {
    let mut spans = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
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
        assert_eq!(
            context_spans("____ here"),
            vec![Blank("____".into()), Text(" here".into())]
        );
    }
}
