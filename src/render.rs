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
    /// A frozen mermaid fence, rendered: occupies the fence's own slot in
    /// the stream. `src` is an absolute file path in the library projection;
    /// the HTTP layer rewrites it to a `/img/<key>` URL. `width`/`height`
    /// are LOGICAL pixels (the raster is 2x); `alt` carries the fence
    /// source, the accessible representation while nothing is masked.
    Diagram {
        src: String,
        width: u32,
        height: u32,
        alt: String,
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
        .map(|note| text_units_with(note, projector, true, &card.resolved_diagrams))
        .unwrap_or_default()
}

pub(crate) fn answer_units_with(
    lines: &[String],
    projector: &mut DisplayProjector,
    diagrams: &[crate::card::ResolvedDiagram],
) -> Vec<NoteUnit> {
    text_units_with(&lines.join("\n"), projector, false, diagrams)
}

/// The fence's own slot gets the rendered diagram when (and only when) a
/// resolved stamp fingerprints to the fence's interior; everything else
/// stays a Code unit, so an unfrozen or stale fence falls back to source
/// in place, never somewhere else.
fn fence_unit(
    info: &str,
    lines: Vec<String>,
    diagrams: &[crate::card::ResolvedDiagram],
) -> NoteUnit {
    if info.eq_ignore_ascii_case("mermaid") && !lines.is_empty() {
        let source = lines.join("\n");
        let print = crate::diagram::fingerprint(&source);
        if let Some(resolved) = diagrams.iter().find(|d| d.fingerprint == print) {
            return NoteUnit::Diagram {
                src: resolved.png.display().to_string(),
                width: resolved.manifest.logical_width,
                height: resolved.manifest.logical_height,
                alt: source,
            };
        }
    }
    NoteUnit::Code { lines }
}

fn text_units_with(
    text: &str,
    projector: &mut DisplayProjector,
    split_prose_sentences: bool,
    diagrams: &[crate::card::ResolvedDiagram],
) -> Vec<NoteUnit> {
    let mut units = Vec::new();
    let mut code_fence: Option<(char, String)> = None;
    let mut code: Vec<String> = Vec::new();
    let mut prose = String::new();
    let mut checklist = Vec::new();

    for logical in text.lines() {
        if let Some(marker) = fence_marker(logical) {
            match &code_fence {
                Some((open_marker, info)) if *open_marker == marker => {
                    let block = std::mem::take(&mut code);
                    if !block.is_empty() {
                        units.push(fence_unit(info, block, diagrams));
                    }
                    code_fence = None;
                }
                None => {
                    flush_checklist(&mut checklist, &mut units);
                    flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
                    let info = logical.trim_start().trim_start_matches(marker).trim();
                    code_fence = Some((marker, info.to_string()));
                    code.clear();
                }
                Some(_) => {
                    code.push(logical.to_string());
                }
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
            flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
            checklist.append(&mut items);
            continue;
        }
        if crate::inline::is_display_math_line(trimmed) {
            flush_checklist(&mut checklist, &mut units);
            flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
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
    flush_prose(&mut prose, &mut units, projector, split_prose_sentences);
    // An unterminated code fence still yields its gathered lines.
    if !code.is_empty() {
        let info = code_fence.map(|(_, info)| info).unwrap_or_default();
        units.push(fence_unit(&info, code, diagrams));
    }
    units
}

pub fn front_units(front: &str) -> Option<Vec<NoteUnit>> {
    let mut projector = DisplayProjector::default();
    front_units_with(front, &mut projector, &[])
}

pub(crate) fn front_units_with(
    front: &str,
    projector: &mut DisplayProjector,
    diagrams: &[crate::card::ResolvedDiagram],
) -> Option<Vec<NoteUnit>> {
    let units = text_units_with(front, projector, true, diagrams);
    units
        .iter()
        .any(|unit| match unit {
            NoteUnit::Checklist { .. } => true,
            NoteUnit::Sentence { runs, .. } => runs
                .iter()
                .any(|run| run.math.as_ref().is_some_and(|math| math.display)),
            NoteUnit::Code { .. } | NoteUnit::Diagram { .. } => true,
        })
        .then_some(units)
}

fn fence_marker(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    trimmed
        .starts_with("```")
        .then_some('`')
        .or_else(|| trimmed.starts_with("~~~").then_some('~'))
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
    split_prose_sentences: bool,
) {
    let chunks = if split_prose_sentences {
        split_sentences(prose)
    } else if prose.trim().is_empty() {
        Vec::new()
    } else {
        vec![prose.trim().to_string()]
    };
    for sentence in chunks {
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
    let tail: String = chars[start..].iter().collect();
    if !tail.trim().is_empty() {
        sentences.push(tail.trim().to_string());
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
    fn checklist_items_preserve_the_authored_state_and_text() {
        assert_eq!(
            Some(vec![ChecklistItem {
                checked: true,
                text: "keep this".into(),
                runs: crate::inline::parse_inline("keep this"),
            }]),
            checklist_items(&["- [x] keep this"])
        );
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
    fn ordinary_answer_units_join_authored_soft_wraps() {
        let mut projector = DisplayProjector::default();
        let lines = vec![
            "`state::open_store` loads the initialized deck, requires its".to_string(),
            "stable deck ID, and opens the document.".to_string(),
        ];
        assert_eq!(
            answer_units_with(&lines, &mut projector, &[]),
            vec![sentence(
                "`state::open_store` loads the initialized deck, requires its stable deck ID, and opens the document."
            )]
        );
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
            context_spans("To ⍰ or not to ⬚"),
            vec![
                Text("To ".into()),
                Blank("⍰".into()),
                Text(" or not to ".into()),
                Hidden("⬚".into()),
            ]
        );
        assert_eq!(
            context_spans("⍰ here"),
            vec![Blank("⍰".into()), Text(" here".into())]
        );
        assert_eq!(
            context_spans("⬚ before ⍰"),
            vec![
                Hidden("⬚".into()),
                Text(" before ".into()),
                Blank("⍰".into()),
            ]
        );
    }

    fn resolved(source: &str) -> Vec<crate::card::ResolvedDiagram> {
        vec![crate::card::ResolvedDiagram {
            fingerprint: crate::diagram::fingerprint(source),
            png: std::path::PathBuf::from("/ws/assets/deck-x/sha256-aa.png"),
            manifest: crate::diagram::DiagramManifest {
                png: "sha256-aa.png".to_string(),
                raster_width: 376,
                raster_height: 228,
                logical_width: 188,
                logical_height: 114,
                labels: Vec::new(),
            },
        }]
    }

    /// Ruling 3's slot law: the rendered diagram replaces its own fence in
    /// the ordered stream; surrounding prose keeps its position, and only
    /// a matching resolved stamp swaps.
    #[test]
    fn a_resolved_mermaid_fence_becomes_a_diagram_unit_in_its_own_slot() {
        let source = "flowchart LR\n A-->B";
        let text = "before\n```mermaid\nflowchart LR\n A-->B\n```\nafter";
        let mut projector = DisplayProjector::default();
        let units = text_units_with(text, &mut projector, false, &resolved(source));
        assert_eq!(3, units.len(), "{units:?}");
        assert!(matches!(&units[0], NoteUnit::Sentence { text, .. } if text == "before"));
        let NoteUnit::Diagram {
            src,
            width,
            height,
            alt,
        } = &units[1]
        else {
            panic!("the fence slot holds the diagram: {units:?}");
        };
        assert_eq!("/ws/assets/deck-x/sha256-aa.png", src);
        assert_eq!(
            (&188, &114),
            (width, height),
            "logical, not raster, dimensions"
        );
        assert_eq!(source, alt, "the fence source is the accessible text");
        assert!(matches!(&units[2], NoteUnit::Sentence { text, .. } if text == "after"));
    }

    /// The server resolves availability: no resolved stamp, a stale
    /// fingerprint, or a non-mermaid fence all stay code units in place.
    #[test]
    fn unresolved_stale_and_foreign_fences_stay_code_units() {
        let mut projector = DisplayProjector::default();
        let cases: [(&str, Vec<crate::card::ResolvedDiagram>, &str); 3] = [
            (
                "```mermaid\nflowchart LR\n A-->B\n```",
                Vec::new(),
                "unfrozen",
            ),
            (
                "```mermaid\nflowchart LR\n A-->B\n```",
                resolved("flowchart LR\n A-->EDITED"),
                "stale fingerprint",
            ),
            (
                "```rust\nflowchart LR\n A-->B\n```",
                resolved("flowchart LR\n A-->B"),
                "non-mermaid fence",
            ),
        ];
        for (text, diagrams, why) in cases {
            let units = text_units_with(text, &mut projector, false, &diagrams);
            assert!(
                matches!(&units[0], NoteUnit::Code { .. }),
                "{why}: must stay a code unit, got {units:?}"
            );
        }
    }
}
