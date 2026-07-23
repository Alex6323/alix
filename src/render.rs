use serde::{Deserialize, Serialize};

use crate::{
    card::Card,
    inline::DisplayProjector,
    parser::{BLANK, HIDDEN},
};

// Struct variants (not newtype) because serde's internal tagging can't tag
// newtype variants; wire shape is `{"kind": ..., "text": ...}` (docs/API.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum NoteUnit {
    Sentence {
        text: String,
        runs: Vec<crate::inline::InlineRun>,
    },
    Code {
        lines: Vec<String>,
    },
    Checklist {
        items: Vec<ChecklistItem>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub checked: bool,
    pub text: String,
    pub runs: Vec<crate::inline::InlineRun>,
}

pub fn checklist_items(lines: &[&str]) -> Option<Vec<ChecklistItem>> {
    let mut projector = DisplayProjector::default();
    checklist_items_with(lines, &mut projector)
}

fn checklist_items_with(
    lines: &[&str],
    projector: &mut DisplayProjector,
) -> Option<Vec<ChecklistItem>> {
    let mut items = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let (checked, raw) = crate::parser::checklist::parse_line(line)?;
        let raw = raw.trim();
        items.push(ChecklistItem {
            checked,
            text: crate::inline::strip_inline(raw),
            runs: projector.project(raw),
        });
    }
    (!items.is_empty()).then_some(items)
}

pub fn note_units(card: &Card) -> Vec<NoteUnit> {
    let mut projector = DisplayProjector::default();
    note_units_with(card, &mut projector)
}

pub(crate) fn note_units_with(card: &Card, projector: &mut DisplayProjector) -> Vec<NoteUnit> {
    card.note
        .as_deref()
        .map(|note| text_units_with(note, projector))
        .unwrap_or_default()
}

fn text_units_with(text: &str, projector: &mut DisplayProjector) -> Vec<NoteUnit> {
    let mut units = Vec::new();
    let mut code_fence = None;
    let mut code: Vec<String> = Vec::new();
    let mut prose = String::new();
    let mut checklist = Vec::new();

    for logical in text.lines() {
        if let Some(marker) = fence_marker(logical) {
            if code_fence == Some(marker) {
                let block = std::mem::take(&mut code);
                if !block.is_empty() {
                    units.push(NoteUnit::Code { lines: block });
                }
                code_fence = None;
            } else if code_fence.is_none() {
                flush_checklist(&mut checklist, &mut units);
                flush_prose(&mut prose, &mut units, projector);
                code_fence = Some(marker);
                code.clear();
            } else {
                code.push(logical.to_string());
            }
            continue;
        }
        if code_fence.is_some() {
            code.push(logical.to_string());
            continue;
        }
        let trimmed = logical.trim();
        if trimmed.is_empty() {
            flush_checklist(&mut checklist, &mut units);
            continue;
        }
        if let Some(mut items) = checklist_items_with(&[logical], projector) {
            flush_prose(&mut prose, &mut units, projector);
            checklist.append(&mut items);
            continue;
        }
        if crate::inline::is_display_math_line(trimmed) {
            flush_checklist(&mut checklist, &mut units);
            flush_prose(&mut prose, &mut units, projector);
            units.push(NoteUnit::Sentence {
                text: trimmed.to_string(),
                runs: projector.project(trimmed),
            });
            continue;
        }
        flush_checklist(&mut checklist, &mut units);
        if !prose.is_empty() {
            prose.push(' ');
        }
        prose.push_str(trimmed);
    }

    flush_checklist(&mut checklist, &mut units);
    flush_prose(&mut prose, &mut units, projector);
    // An unterminated code fence still yields its gathered lines.
    if !code.is_empty() {
        units.push(NoteUnit::Code { lines: code });
    }
    units
}

pub fn front_units(front: &str) -> Option<Vec<NoteUnit>> {
    let mut projector = DisplayProjector::default();
    front_units_with(front, &mut projector)
}

pub(crate) fn front_units_with(
    front: &str,
    projector: &mut DisplayProjector,
) -> Option<Vec<NoteUnit>> {
    let units = text_units_with(front, projector);
    units
        .iter()
        .any(|unit| match unit {
            NoteUnit::Checklist { .. } => true,
            NoteUnit::Sentence { runs, .. } => runs
                .iter()
                .any(|run| run.math.as_ref().is_some_and(|math| math.display)),
            NoteUnit::Code { .. } => true,
        })
        .then_some(units)
}

fn fence_marker(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    trimmed.starts_with("```").then_some('`').or_else(|| {
        trimmed
            .starts_with("~~~")
            .then_some('~')
    })
}

fn flush_checklist(checklist: &mut Vec<ChecklistItem>, units: &mut Vec<NoteUnit>) {
    if !checklist.is_empty() {
        units.push(NoteUnit::Checklist {
            items: std::mem::take(checklist),
        });
    }
}

fn flush_prose(
    prose: &mut String,
    units: &mut Vec<NoteUnit>,
    projector: &mut DisplayProjector,
) {
    for sentence in split_sentences(prose) {
        if !sentence.is_empty() {
            let runs = projector.project(&sentence);
            units.push(NoteUnit::Sentence {
                text: sentence,
                runs,
            });
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

    fn sentence(text: &str) -> NoteUnit {
        NoteUnit::Sentence {
            text: text.into(),
            runs: crate::inline::parse_inline(text),
        }
    }

    #[test]
    fn no_note_yields_no_units() {
        let card = Card::plain(Arc::from("s.txt"), "f".into(), vec!["b".into()], None, 1);
        assert!(note_units(&card).is_empty());
    }

    #[test]
    fn prose_splits_into_sentences() {
        let units = note_units(&card_with_note("First one. Second one."));
        assert_eq!(units, vec![sentence("First one."), sentence("Second one.")]);
    }

    #[test]
    fn hard_wrapped_prose_joins_before_splitting() {
        let units = note_units(&card_with_note("A sentence spread\nacross two lines."));
        assert_eq!(units, vec![sentence("A sentence spread across two lines.")]);
    }

    #[test]
    fn display_math_line_flushes_surrounding_prose() {
        let units = note_units(&card_with_note("Before.\n$$x^2$$\nAfter."));
        assert_eq!(units.len(), 3);
        assert_eq!(units[0], sentence("Before."));
        let NoteUnit::Sentence { text, runs } = &units[1] else {
            panic!("display math should be a sentence unit");
        };
        assert_eq!(text, "$$x^2$$");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].math.as_ref().unwrap().display);
        assert_eq!(units[2], sentence("After."));
    }

    #[test]
    fn display_math_makes_front_units_structural() {
        let units = front_units("Before\n$$x^2$$\nAfter").unwrap();
        assert_eq!(units.len(), 3);
        let NoteUnit::Sentence { runs, .. } = &units[1] else {
            panic!("display math should be a sentence unit");
        };
        assert!(runs[0].math.as_ref().unwrap().display);
    }

    #[test]
    fn dollars_in_fenced_code_never_render_as_math() {
        for fence in ["```", "~~~"] {
            let note = format!("{fence}\n$x^2$\n{fence}");
            let units = note_units(&card_with_note(&note));
            assert_eq!(
                units,
                vec![NoteUnit::Code {
                    lines: vec!["$x^2$".into()]
                }]
            );
        }
    }

    #[test]
    fn front_units_are_some_only_when_a_task_list_is_present() {
        assert_eq!(None, front_units("What is the capital of France?"));
        let units = front_units("Given this list:\n- [x] keep\n- [ ] drop").unwrap();
        assert_eq!(
            units,
            vec![
                NoteUnit::Sentence {
                    text: "Given this list:".into(),
                    runs: crate::inline::parse_inline("Given this list:"),
                },
                NoteUnit::Checklist {
                    items: vec![
                        ChecklistItem {
                            checked: true,
                            text: "keep".into(),
                            runs: crate::inline::parse_inline("keep"),
                        },
                        ChecklistItem {
                            checked: false,
                            text: "drop".into(),
                            runs: crate::inline::parse_inline("drop"),
                        },
                    ],
                },
            ]
        );
    }

    #[test]
    fn a_task_list_note_becomes_a_checklist_unit() {
        let units = note_units(&card_with_note("Recall:\n- [x] do this\n- [ ] not that"));
        assert_eq!(
            units,
            vec![
                NoteUnit::Sentence {
                    text: "Recall:".into(),
                    runs: crate::inline::parse_inline("Recall:"),
                },
                NoteUnit::Checklist {
                    items: vec![
                        ChecklistItem {
                            checked: true,
                            text: "do this".into(),
                            runs: crate::inline::parse_inline("do this"),
                        },
                        ChecklistItem {
                            checked: false,
                            text: "not that".into(),
                            runs: crate::inline::parse_inline("not that"),
                        },
                    ],
                },
            ]
        );
    }

    #[test]
    fn code_block_is_verbatim() {
        let note = "Intro here.\n```\nfn main() {\n    let x = 1;\n}\n```";
        let units = note_units(&card_with_note(note));
        assert_eq!(
            units,
            vec![
                sentence("Intro here."),
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
                sentence("After the block."),
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
        assert_eq!(units, vec![sentence("See section 2.1 for details.")]);
    }

    #[test]
    fn note_units_serialize_as_the_documented_wire_shape() {
        let units = vec![
            NoteUnit::Sentence {
                text: "One owner.".into(),
                runs: crate::inline::parse_inline("One owner."),
            },
            NoteUnit::Code {
                lines: vec!["let s;".into()],
            },
            NoteUnit::Checklist {
                items: vec![ChecklistItem {
                    checked: true,
                    text: "Own it".into(),
                    runs: crate::inline::parse_inline("**Own** it"),
                }],
            },
        ];
        assert_eq!(
            serde_json::json!([
                {"kind": "sentence", "text": "One owner.", "runs": [{"text": "One owner."}]},
                {"kind": "code", "lines": ["let s;"]},
                {"kind": "checklist", "items": [{
                    "checked": true,
                    "text": "Own it",
                    "runs": [{"text": "Own", "bold": true}, {"text": " it"}]
                }]},
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
