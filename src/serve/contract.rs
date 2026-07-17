//! The JSON-API contract snapshot suite (docs/API.md): every wire-facing
//! DTO gets its entire serialized shape pinned by full-object equality, so
//! any field add/remove/rename/retype fails here with a pointer at the
//! doc. Each pin also emits its expected JSON to `tests/contracts/` — the
//! machine-readable corpus for thin-client codegen. The page-private
//! keybinding DTOs (`KeyDto`, `ReviewKeys`, `PickerKeysDto`, `BrowseKeys`)
//! are deliberately out of contract and unpinned.

use serde_json::json;

use super::*;
use crate::{answer::TypedResult, render::NoteUnit};

/// Pins a DTO's exact wire shape and emits it to the codegen corpus.
/// A failure means the JSON contract moved: update docs/API.md's field
/// table + example for this anchor AND add a CHANGELOG entry.
fn pin<T: serde::Serialize>(anchor: &str, dto: &T, expected: serde_json::Value) {
    let actual = serde_json::to_value(dto).unwrap();
    assert_eq!(
        actual, expected,
        "wire shape drifted from docs/API.md#{anchor} — update the doc's \
         field table + example AND add a CHANGELOG entry"
    );
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{anchor}.json")),
        serde_json::to_string_pretty(&expected).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
fn statedto_select_phase_wire_shape() {
    let dto = StateDto {
        kind: "review",
        phase: "select",
        card: None,
        choices: None,
        keypoints: None,
        acquire: false,
        mode: "flip",
        depth: "recall",
        input: "type",
        remaining: 0,
        initial: 0,
        reviews: 0,
        passed: 0,
        failed: 0,
        acquired: 0,
        exam_due: Vec::new(),
        can_restart: false,
        promotable: false,
        label: "select decks".to_string(),
    };
    pin(
        "StateDto.select",
        &dto,
        json!({
            "kind": "review",
            "phase": "select",
            "card": null,
            "choices": null,
            "keypoints": null,
            "acquire": false,
            "mode": "flip",
            "depth": "recall",
            "input": "type",
            "remaining": 0,
            "initial": 0,
            "reviews": 0,
            "passed": 0,
            "failed": 0,
            "acquired": 0,
            "exam_due": [],
            "can_restart": false,
            "promotable": false,
            "label": "select decks"
        }),
    );
}

#[test]
fn statedto_review_phase_wire_shape() {
    let dto = StateDto {
        kind: "review",
        phase: "review",
        card: Some(CardDto {
            front: "What is ownership?".to_string(),
            context: vec!["Chapter 4".to_string()],
            back: vec!["every value has one owner".to_string()],
            reshaped: true,
            note: vec![
                NoteUnit::Sentence {
                    text: "Ownership frees memory deterministically.".to_string(),
                },
                NoteUnit::Code {
                    lines: vec!["let s = String::new();".to_string()],
                },
            ],
            img: Some("/img/0123456789abcdef".to_string()),
            img_back: Some("/img/0123456789abcdef".to_string()),
            at: Some("string.rs:120-128".to_string()),
            citation: Some(ExcerptDto {
                path: "src/string.rs".to_string(),
                lines: vec![LineDto {
                    n: 120,
                    text: "let s = String::new();".to_string(),
                }],
                truncated: false,
            }),
            citation_error: None,
            crumb: Some(CrumbDto {
                regions: vec!["intro".to_string(), "body".to_string()],
                current: 1,
                cells: vec![vec![0.5], vec![1.0]],
            }),
        }),
        choices: Some(vec!["owner".to_string(), "borrower".to_string()]),
        keypoints: Some(vec!["one owner per value".to_string()]),
        acquire: false,
        mode: "flip",
        depth: "recall",
        input: "type",
        remaining: 3,
        initial: 5,
        reviews: 2,
        passed: 1,
        failed: 1,
        acquired: 4,
        exam_due: vec!["rust.txt".to_string()],
        can_restart: true,
        promotable: true,
        label: "rust.txt".to_string(),
    };
    pin(
        "StateDto.review",
        &dto,
        json!({
            "kind": "review",
            "phase": "review",
            "card": {
                "front": "What is ownership?",
                "context": ["Chapter 4"],
                "back": ["every value has one owner"],
                "reshaped": true,
                "note": [
                    {"kind": "sentence", "text": "Ownership frees memory deterministically."},
                    {"kind": "code", "lines": ["let s = String::new();"]}
                ],
                "img": "/img/0123456789abcdef",
                "img_back": "/img/0123456789abcdef",
                "at": "string.rs:120-128",
                "citation": {
                    "path": "src/string.rs",
                    "lines": [{"n": 120, "text": "let s = String::new();"}],
                    "truncated": false
                },
                "citation_error": null,
                "crumb": {
                    "regions": ["intro", "body"],
                    "current": 1,
                    "cells": [[0.5], [1.0]]
                }
            },
            "choices": ["owner", "borrower"],
            "keypoints": ["one owner per value"],
            "acquire": false,
            "mode": "flip",
            "depth": "recall",
            "input": "type",
            "remaining": 3,
            "initial": 5,
            "reviews": 2,
            "passed": 1,
            "failed": 1,
            "acquired": 4,
            "exam_due": ["rust.txt"],
            "can_restart": true,
            "promotable": true,
            "label": "rust.txt"
        }),
    );
}

#[test]
fn walkdto_predict_phase_wire_shape() {
    let dto = WalkDto {
        kind: "walk",
        phase: "predict",
        description: "how a String grows".to_string(),
        source: Some("src/lib.rs".to_string()),
        total: 2,
        current: 2,
        path: vec![
            HopDto {
                prompt: "push begins".to_string(),
                delta: Some("passed"),
                current: false,
            },
            HopDto {
                prompt: "capacity doubles".to_string(),
                delta: None,
                current: true,
            },
        ],
        prompt: Some("what does push do when full?".to_string()),
        givens: vec!["a String at capacity".to_string()],
        locator: Some("lib.rs:40-52".to_string()),
        prediction: None,
        excerpt: None,
        excerpt_error: None,
        points: Vec::new(),
        note: None,
        auto_grade: false,
        thinking: false,
        verdict: None,
        feedback: None,
        grade_error: None,
        summary: None,
    };
    pin(
        "WalkDto.predict",
        &dto,
        json!({
            "kind": "walk",
            "phase": "predict",
            "description": "how a String grows",
            "source": "src/lib.rs",
            "total": 2,
            "current": 2,
            "path": [
                {"prompt": "push begins", "delta": "passed", "current": false},
                {"prompt": "capacity doubles", "delta": null, "current": true}
            ],
            "prompt": "what does push do when full?",
            "givens": ["a String at capacity"],
            "locator": "lib.rs:40-52",
            "prediction": null,
            "excerpt": null,
            "excerpt_error": null,
            "points": [],
            "note": null,
            "auto_grade": false,
            "thinking": false,
            "verdict": null,
            "feedback": null,
            "grade_error": null,
            "summary": null
        }),
    );
}

#[test]
fn walkdto_done_phase_wire_shape() {
    let dto = WalkDto {
        kind: "walk",
        phase: "done",
        description: "how a String grows".to_string(),
        source: None,
        total: 3,
        current: 3,
        path: vec![HopDto {
            prompt: "capacity doubles".to_string(),
            delta: Some("partly"),
            current: false,
        }],
        prompt: None,
        givens: Vec::new(),
        locator: None,
        prediction: Some("it reallocates".to_string()),
        excerpt: Some(ExcerptDto {
            path: "src/lib.rs".to_string(),
            lines: vec![LineDto {
                n: 40,
                text: "self.grow();".to_string(),
            }],
            truncated: true,
        }),
        excerpt_error: None,
        points: vec!["amortized doubling".to_string()],
        note: Some("see also Vec".to_string()),
        auto_grade: true,
        thinking: false,
        verdict: Some("partly"),
        feedback: Some("half right".to_string()),
        grade_error: None,
        summary: Some(SummaryDto {
            passed: 1,
            partly: 1,
            failed: 1,
            weak: vec![2, 3],
            total: 3,
        }),
    };
    pin(
        "WalkDto.done",
        &dto,
        json!({
            "kind": "walk",
            "phase": "done",
            "description": "how a String grows",
            "source": null,
            "total": 3,
            "current": 3,
            "path": [
                {"prompt": "capacity doubles", "delta": "partly", "current": false}
            ],
            "prompt": null,
            "givens": [],
            "locator": null,
            "prediction": "it reallocates",
            "excerpt": {
                "path": "src/lib.rs",
                "lines": [{"n": 40, "text": "self.grow();"}],
                "truncated": true
            },
            "excerpt_error": null,
            "points": ["amortized doubling"],
            "note": "see also Vec",
            "auto_grade": true,
            "thinking": false,
            "verdict": "partly",
            "feedback": "half right",
            "grade_error": null,
            "summary": {"passed": 1, "partly": 1, "failed": 1, "weak": [2, 3], "total": 3}
        }),
    );
}

#[test]
fn examdto_results_phase_wire_shape() {
    let dto = ExamDto {
        phase: "results",
        deck: "rust.txt".to_string(),
        strictness: "balanced",
        total: 1,
        current: 1,
        question: None,
        answer: String::new(),
        on_last: true,
        grades: vec![ExamGradeDto {
            question: "Why does Rust use ownership?".to_string(),
            points: vec!["memory safety without a GC".to_string()],
            answer: "it frees memory deterministically".to_string(),
            verdict: "PASS",
            feedback: "solid".to_string(),
            missed: Vec::new(),
        }],
        passed: Some(true),
        gaps: Vec::new(),
        can_remediate: false,
        remediated_count: None,
        is_trace: false,
        unlocks: vec!["next.txt".to_string()],
        thinking: false,
        error: None,
        elapsed: None,
        cooldown_ms: None,
    };
    pin(
        "ExamDto.results",
        &dto,
        json!({
            "phase": "results",
            "deck": "rust.txt",
            "strictness": "balanced",
            "total": 1,
            "current": 1,
            "question": null,
            "answer": "",
            "on_last": true,
            "grades": [{
                "question": "Why does Rust use ownership?",
                "points": ["memory safety without a GC"],
                "answer": "it frees memory deterministically",
                "verdict": "PASS",
                "feedback": "solid",
                "missed": []
            }],
            "passed": true,
            "gaps": [],
            "can_remediate": false,
            "remediated_count": null,
            "is_trace": false,
            "unlocks": ["next.txt"],
            "thinking": false,
            "error": null,
            "elapsed": null,
            "cooldown_ms": null
        }),
    );
}

#[test]
fn examdto_cooldown_phase_wire_shape() {
    let dto = cooldown_dto("deck.txt", 90000);
    pin(
        "ExamDto.cooldown",
        &dto,
        json!({
            "phase": "cooldown",
            "deck": "deck.txt",
            "strictness": "balanced",
            "total": 0,
            "current": 0,
            "question": null,
            "answer": "",
            "on_last": false,
            "grades": [],
            "passed": null,
            "gaps": [],
            "can_remediate": false,
            "remediated_count": null,
            "is_trace": true,
            "unlocks": [],
            "thinking": false,
            "error": null,
            "elapsed": null,
            "cooldown_ms": 90000
        }),
    );
}

#[test]
fn decklistdto_wire_shape() {
    let dto = DeckListDto {
        workspaces: vec![DeckItemDto {
            name: "rustws".to_string(),
            selectable: false,
            label: "Rust workspace".to_string(),
            meta: Some("3/10".to_string()),
            state: "workspace",
            locked: false,
            reviewable: true,
            reviewable_recognize: false,
            reviewable_recall: true,
            reviewable_reconstruct: true,
            mastered: false,
            is_trace: false,
            examable: false,
            has_exam: false,
            recent: true,
            is_workspace: true,
            description: Some("learn Rust ownership".to_string()),
            members: vec![MemberDto {
                name: "rustws/intro.txt".to_string(),
                selectable: true,
                label: "Intro".to_string(),
                meta: Some("3/10".to_string()),
                state: "started",
                locked: false,
                reviewable: true,
                reviewable_recognize: true,
                reviewable_recall: true,
                reviewable_reconstruct: false,
                mastered: false,
                is_trace: false,
                examable: true,
                has_exam: true,
                indent: 1,
                tree: "└─ ".to_string(),
                has_topology: false,
                badge_depth: None,
                badge_dotted: false,
                new_cards: false,
                last_depth: "recall",
            }],
            path: Some("~/decks".to_string()),
            icon: Some("/img/0123456789abcdef".to_string()),
            icon_svg: true,
            has_topology: true,
            badge_depth: Some("recall"),
            badge_dotted: true,
            new_cards: true,
            last_depth: "recall",
            deadline: Some(DeadlineDto {
                date: "2026-09-01".to_string(),
                days_left: 23,
                ready: 2,
                total: 5,
            }),
        }],
        recent: vec![DeckItemDto {
            name: "vocab.txt".to_string(),
            selectable: true,
            label: "Vocab".to_string(),
            meta: Some("new".to_string()),
            state: "new",
            locked: false,
            reviewable: true,
            reviewable_recognize: true,
            reviewable_recall: true,
            reviewable_reconstruct: false,
            mastered: false,
            is_trace: false,
            examable: false,
            has_exam: false,
            recent: true,
            is_workspace: false,
            description: None,
            members: Vec::new(),
            path: None,
            icon: None,
            icon_svg: false,
            has_topology: false,
            badge_depth: None,
            badge_dotted: false,
            new_cards: true,
            last_depth: "recall",
            // A loose deck row never carries a deadline: that's a workspace
            // setting only.
            deadline: None,
        }],
        folders: Vec::new(),
    };
    pin(
        "DeckListDto",
        &dto,
        json!({
            "workspaces": [{
                "name": "rustws",
                "selectable": false,
                "label": "Rust workspace",
                "meta": "3/10",
                "state": "workspace",
                "locked": false,
                "reviewable": true,
                "reviewable_recognize": false,
                "reviewable_recall": true,
                "reviewable_reconstruct": true,
                "mastered": false,
                "is_trace": false,
                "examable": false,
                "has_exam": false,
                "recent": true,
                "is_workspace": true,
                "description": "learn Rust ownership",
                "members": [{
                    "name": "rustws/intro.txt",
                    "selectable": true,
                    "label": "Intro",
                    "meta": "3/10",
                    "state": "started",
                    "locked": false,
                    "reviewable": true,
                    "reviewable_recognize": true,
                    "reviewable_recall": true,
                    "reviewable_reconstruct": false,
                    "mastered": false,
                    "is_trace": false,
                    "examable": true,
                    "has_exam": true,
                    "indent": 1,
                    "tree": "└─ ",
                    "has_topology": false,
                    "badge_depth": null,
                    "badge_dotted": false,
                    "new_cards": false,
                    "last_depth": "recall"
                }],
                "path": "~/decks",
                "icon": "/img/0123456789abcdef",
                "icon_svg": true,
                "has_topology": true,
                "badge_depth": "recall",
                "badge_dotted": true,
                "new_cards": true,
                "last_depth": "recall",
                "deadline": {
                    "date": "2026-09-01",
                    "days_left": 23,
                    "ready": 2,
                    "total": 5
                }
            }],
            "recent": [{
                "name": "vocab.txt",
                "selectable": true,
                "label": "Vocab",
                "meta": "new",
                "state": "new",
                "locked": false,
                "reviewable": true,
                "reviewable_recognize": true,
                "reviewable_recall": true,
                "reviewable_reconstruct": false,
                "mastered": false,
                "is_trace": false,
                "examable": false,
                "has_exam": false,
                "recent": true,
                "is_workspace": false,
                "description": null,
                "members": [],
                "path": null,
                "icon": null,
                "icon_svg": false,
                "has_topology": false,
                "badge_depth": null,
                "badge_dotted": false,
                "new_cards": true,
                "last_depth": "recall",
                "deadline": null
            }],
            "folders": []
        }),
    );
}

#[test]
fn decktopologydto_wire_shape() {
    let dto = DeckTopologyDto {
        topologies: vec![TopologyInfoDto {
            name: "north-south".to_string(),
            principle: "north to south".to_string(),
            regions: vec![RegionInfoDto {
                name: "north".to_string(),
                cells: vec![0.5, 1.0],
                due: 2,
            }],
        }],
        deck_due: 3,
    };
    pin(
        "DeckTopologyDto",
        &dto,
        json!({
            "topologies": [{
                "name": "north-south",
                "principle": "north to south",
                "regions": [{"name": "north", "cells": [0.5, 1.0], "due": 2}]
            }],
            "deck_due": 3
        }),
    );
}

#[test]
fn browsedto_wire_shape() {
    let dto = BrowseDto {
        phase: "browse",
        label: "rust.txt".to_string(),
        cards: vec![CardDto {
            front: "q".to_string(),
            context: Vec::new(),
            back: vec!["a".to_string()],
            reshaped: false,
            note: Vec::new(),
            img: None,
            img_back: None,
            at: None,
            citation: None,
            citation_error: None,
            crumb: None,
        }],
    };
    pin(
        "BrowseDto",
        &dto,
        json!({
            "phase": "browse",
            "label": "rust.txt",
            "cards": [{
                "front": "q",
                "context": [],
                "back": ["a"],
                "reshaped": false,
                "note": [],
                "img": null,
                "img_back": null,
                "at": null,
                "citation": null,
                "citation_error": null,
                "crumb": null
            }]
        }),
    );
}

// The choose/check feedback wire shapes are the core `review` types
// serialized directly (the handlers delegate to `review::choose` /
// `review::check_typed`); the anchors keep their historic DTO names so the
// corpus filenames and docs/API.md sections stay stable.
#[test]
fn choosefeedbackdto_wire_shape() {
    let feedback = crate::review::ChoiceFeedback {
        chosen: 2,
        correct: 1,
        passed: false,
    };
    pin(
        "ChooseFeedbackDto",
        &feedback,
        json!({"chosen": 2, "correct": 1, "passed": false}),
    );
}

#[test]
fn checkfeedbackdto_wire_shape() {
    let feedback = crate::review::CheckFeedback {
        results: vec![TypedResult {
            input: "pars".to_string(),
            expected: "Paris".to_string(),
            passed: false,
        }],
        passed: false,
    };
    pin(
        "CheckFeedbackDto",
        &feedback,
        json!({
            "results": [{"input": "pars", "expected": "Paris", "passed": false}],
            "passed": false
        }),
    );
}

#[test]
fn askdto_populated_wire_shape() {
    let dto = AskDto {
        transcript: vec![ExchangeDto {
            q: "why one owner?".to_string(),
            a: "so drops are deterministic".to_string(),
        }],
        thinking: true,
        status: Some("asking claude".to_string()),
        error: None,
        draft: None,
    };
    pin(
        "AskDto.populated",
        &dto,
        json!({
            "transcript": [{"q": "why one owner?", "a": "so drops are deterministic"}],
            "thinking": true,
            "status": "asking claude",
            "error": null,
            "draft": null
        }),
    );
}

#[test]
fn askdto_empty_wire_shape() {
    let dto = AskDto {
        transcript: Vec::new(),
        thinking: false,
        status: None,
        error: None,
        draft: None,
    };
    pin(
        "AskDto.empty",
        &dto,
        json!({
            "transcript": [],
            "thinking": false,
            "status": null,
            "error": null,
            "draft": null
        }),
    );
}

#[test]
fn askdto_with_draft_wire_shape() {
    let dto = AskDto {
        transcript: vec![ExchangeDto {
            q: "why one owner?".to_string(),
            a: "so drops are deterministic".to_string(),
        }],
        thinking: false,
        status: None,
        error: None,
        draft: Some(DraftCardDto {
            front: "Why does Rust use one owner per value?".to_string(),
            back: vec!["so drops are deterministic".to_string()],
        }),
    };
    pin(
        "AskDto.with_draft",
        &dto,
        json!({
            "transcript": [{"q": "why one owner?", "a": "so drops are deterministic"}],
            "thinking": false,
            "status": null,
            "error": null,
            "draft": {
                "front": "Why does Rust use one owner per value?",
                "back": ["so drops are deterministic"]
            }
        }),
    );
}

#[test]
fn createcardresp_wire_shape() {
    let dto = CreateCardResp {
        id: "12345".to_string(),
    };
    pin("CreateCardResp", &dto, json!({"id": "12345"}));
}

#[test]
fn askinfodto_and_versiondto_wire_shape() {
    let info = AskInfoDto {
        backend: "claude",
        model: "default".to_string(),
        effort: "default".to_string(),
    };
    pin(
        "AskInfoDto",
        &info,
        json!({"backend": "claude", "model": "default", "effort": "default"}),
    );
    let version = VersionDto {
        version: env!("CARGO_PKG_VERSION"),
    };
    pin(
        "VersionDto",
        &version,
        json!({"version": env!("CARGO_PKG_VERSION")}),
    );
}

#[test]
fn doctordto_wire_shape() {
    let dto = DoctorDto {
        rows: vec![
            DoctorRowDto {
                name: "config",
                status: "ok",
                detail: "~/.config/alix/config.toml parses".to_string(),
                remedy: None,
            },
            DoctorRowDto {
                name: "share",
                status: "warn",
                detail: "`wormhole` not found on PATH".to_string(),
                remedy: Some("pipx install magic-wormhole".to_string()),
            },
        ],
    };
    pin(
        "DoctorDto",
        &dto,
        json!({
            "rows": [
                {"name": "config", "status": "ok",
                 "detail": "~/.config/alix/config.toml parses", "remedy": null},
                {"name": "share", "status": "warn",
                 "detail": "`wormhole` not found on PATH",
                 "remedy": "pipx install magic-wormhole"}
            ]
        }),
    );
}

#[test]
fn pairdto_wire_shape() {
    let dto = PairDto {
        url: "http://127.0.0.1:7777/".to_string(),
        svg: None,
        lan: false,
    };
    pin(
        "PairDto",
        &dto,
        json!({"url": "http://127.0.0.1:7777/", "svg": null, "lan": false}),
    );
}

#[test]
fn resetdto_wire_shape() {
    let dto = ResetDto {
        deck: "rust.txt".to_string(),
        cards_cleared: 17,
    };
    pin(
        "ResetDto",
        &dto,
        json!({"deck": "rust.txt", "cards_cleared": 17}),
    );
}

#[test]
fn importdto_wire_shape() {
    let dto = ImportDto {
        deck: "kanji.txt".to_string(),
        cards: 40,
    };
    pin("ImportDto", &dto, json!({"deck": "kanji.txt", "cards": 40}));
}

#[test]
fn augmentdto_wire_shape() {
    let dto = AugmentDto {
        deck: "rust.txt".to_string(),
        cards: 12,
        rows: vec![AugmentRowDto {
            kind: "choices",
            label: "choice distractors",
            covered: 4,
            eligible: 12,
            items: vec!["north-south".to_string()],
            busy: true,
        }],
        busy: Some("choices"),
        elapsed: Some(3),
        error: None,
        queued: vec!["notes"],
        done: vec!["format"],
        failed: vec![FailedTargetDto {
            target: "topology",
            error: "the model did not return valid JSON".to_string(),
        }],
    };
    pin(
        "AugmentDto",
        &dto,
        json!({
            "deck": "rust.txt",
            "cards": 12,
            "rows": [{
                "kind": "choices",
                "label": "choice distractors",
                "covered": 4,
                "eligible": 12,
                "items": ["north-south"],
                "busy": true
            }],
            "busy": "choices",
            "elapsed": 3,
            "error": null,
            "queued": ["notes"],
            "done": ["format"],
            "failed": [{
                "target": "topology",
                "error": "the model did not return valid JSON"
            }]
        }),
    );
}

#[test]
fn generatedto_done_wire_shape() {
    let dto = GenerateDto {
        phase: "done",
        deck: Some("rust-ownership.txt".to_string()),
        cards: Some(12),
        elapsed: Some(41),
        error: None,
    };
    pin(
        "GenerateDto",
        &dto,
        json!({"phase": "done", "deck": "rust-ownership.txt", "cards": 12,
               "elapsed": 41, "error": null}),
    );
}

#[test]
fn sharedto_code_phase_wire_shape() {
    let dto = ShareDto {
        phase: "code",
        code: Some("7-alpha-bravo".to_string()),
        elapsed: Some(3),
        error: None,
    };
    pin(
        "ShareDto",
        &dto,
        json!({"phase": "code", "code": "7-alpha-bravo", "elapsed": 3, "error": null}),
    );
}

#[test]
fn receivedto_done_wire_shape() {
    let dto = ReceiveDto {
        phase: "done",
        landed: Some("rust-decks".to_string()),
        stripped: vec!["progress.json".to_string()],
        elapsed: Some(9),
        error: None,
    };
    pin(
        "ReceiveDto",
        &dto,
        json!({"phase": "done", "landed": "rust-decks",
               "stripped": ["progress.json"], "elapsed": 9, "error": null}),
    );
}

#[test]
fn remoteaskdto_thinking_wire_shape() {
    let dto = RemoteAskDto {
        thinking: true,
        answer: None,
        draft: None,
        note: None,
        error: None,
        elapsed: Some(3),
    };
    pin(
        "RemoteAskDto.thinking",
        &dto,
        json!({
            "thinking": true,
            "answer": null,
            "draft": null,
            "note": null,
            "error": null,
            "elapsed": 3
        }),
    );
}

#[test]
fn remoteaskdto_done_wire_shape() {
    let dto = RemoteAskDto {
        thinking: false,
        answer: Some("so drops are deterministic".to_string()),
        draft: Some(DraftCardDto {
            front: "Why does Rust use one owner per value?".to_string(),
            back: vec![
                "so drops are deterministic".to_string(),
                "no GC needed".to_string(),
            ],
        }),
        note: None,
        error: None,
        elapsed: None,
    };
    pin(
        "RemoteAskDto.done",
        &dto,
        json!({
            "thinking": false,
            "answer": "so drops are deterministic",
            "draft": {
                "front": "Why does Rust use one owner per value?",
                "back": ["so drops are deterministic", "no GC needed"]
            },
            "note": null,
            "error": null,
            "elapsed": null
        }),
    );
}

#[test]
fn remoteaskdto_note_wire_shape() {
    let dto = RemoteAskDto {
        thinking: false,
        answer: None,
        draft: None,
        note: Some(vec![
            "ownership drops values deterministically".to_string(),
            "no GC needed".to_string(),
        ]),
        error: None,
        elapsed: None,
    };
    pin(
        "RemoteAskDto.note",
        &dto,
        json!({
            "thinking": false,
            "answer": null,
            "draft": null,
            "note": ["ownership drops values deterministically", "no GC needed"],
            "error": null,
            "elapsed": null
        }),
    );
}

#[test]
fn remoteexamdto_idle_wire_shape() {
    let dto = RemoteExamDto {
        phase: "idle",
        deck: String::new(),
        strictness: "balanced",
        questions: Vec::new(),
        passed: None,
        grades: Vec::new(),
        gaps: Vec::new(),
        can_remediate: false,
        cards: None,
        thinking: false,
        elapsed: None,
        error: None,
    };
    pin(
        "RemoteExamDto.idle",
        &dto,
        json!({
            "phase": "idle",
            "deck": "",
            "strictness": "balanced",
            "questions": [],
            "passed": null,
            "grades": [],
            "gaps": [],
            "can_remediate": false,
            "cards": null,
            "thinking": false,
            "elapsed": null,
            "error": null
        }),
    );
}

#[test]
fn remoteexamdto_answering_wire_shape() {
    let dto = RemoteExamDto {
        phase: "answering",
        deck: "rust.txt".to_string(),
        strictness: "balanced",
        questions: vec![
            "Why does Rust use ownership?".to_string(),
            "What is borrowing?".to_string(),
        ],
        passed: None,
        grades: Vec::new(),
        gaps: Vec::new(),
        can_remediate: false,
        cards: None,
        thinking: false,
        elapsed: None,
        error: None,
    };
    pin(
        "RemoteExamDto.answering",
        &dto,
        json!({
            "phase": "answering",
            "deck": "rust.txt",
            "strictness": "balanced",
            "questions": ["Why does Rust use ownership?", "What is borrowing?"],
            "passed": null,
            "grades": [],
            "gaps": [],
            "can_remediate": false,
            "cards": null,
            "thinking": false,
            "elapsed": null,
            "error": null
        }),
    );
}

#[test]
fn remoteexamdto_results_wire_shape() {
    let dto = RemoteExamDto {
        phase: "results",
        deck: "rust.txt".to_string(),
        strictness: "balanced",
        questions: vec!["Why does Rust use ownership?".to_string()],
        passed: Some(false),
        grades: vec![ExamGradeDto {
            question: "Why does Rust use ownership?".to_string(),
            points: vec!["memory safety without a GC".to_string()],
            answer: "it has a garbage collector".to_string(),
            verdict: "FAIL",
            feedback: "Rust has no GC".to_string(),
            missed: vec!["memory safety without a GC".to_string()],
        }],
        gaps: vec!["ownership and the GC-free memory model".to_string()],
        can_remediate: true,
        cards: None,
        thinking: false,
        elapsed: None,
        error: None,
    };
    pin(
        "RemoteExamDto.results",
        &dto,
        json!({
            "phase": "results",
            "deck": "rust.txt",
            "strictness": "balanced",
            "questions": ["Why does Rust use ownership?"],
            "passed": false,
            "grades": [{
                "question": "Why does Rust use ownership?",
                "points": ["memory safety without a GC"],
                "answer": "it has a garbage collector",
                "verdict": "FAIL",
                "feedback": "Rust has no GC",
                "missed": ["memory safety without a GC"]
            }],
            "gaps": ["ownership and the GC-free memory model"],
            "can_remediate": true,
            "cards": null,
            "thinking": false,
            "elapsed": null,
            "error": null
        }),
    );
}

#[test]
fn remoteexamdto_remediated_wire_shape() {
    let dto = RemoteExamDto {
        phase: "remediated",
        deck: "rust.txt".to_string(),
        strictness: "balanced",
        questions: vec!["Why does Rust use ownership?".to_string()],
        passed: Some(false),
        grades: vec![ExamGradeDto {
            question: "Why does Rust use ownership?".to_string(),
            points: vec!["memory safety without a GC".to_string()],
            answer: "it has a garbage collector".to_string(),
            verdict: "FAIL",
            feedback: "Rust has no GC".to_string(),
            missed: vec!["memory safety without a GC".to_string()],
        }],
        gaps: vec!["ownership and the GC-free memory model".to_string()],
        can_remediate: false,
        cards: Some(
            "# Why does Rust use ownership?\n\tso drops are deterministic, no GC needed"
                .to_string(),
        ),
        thinking: false,
        elapsed: None,
        error: None,
    };
    pin(
        "RemoteExamDto.remediated",
        &dto,
        json!({
            "phase": "remediated",
            "deck": "rust.txt",
            "strictness": "balanced",
            "questions": ["Why does Rust use ownership?"],
            "passed": false,
            "grades": [{
                "question": "Why does Rust use ownership?",
                "points": ["memory safety without a GC"],
                "answer": "it has a garbage collector",
                "verdict": "FAIL",
                "feedback": "Rust has no GC",
                "missed": ["memory safety without a GC"]
            }],
            "gaps": ["ownership and the GC-free memory model"],
            "can_remediate": false,
            "cards": "# Why does Rust use ownership?\n\tso drops are deterministic, no GC needed",
            "thinking": false,
            "elapsed": null,
            "error": null
        }),
    );
}

#[test]
fn remotegeneratedto_generating_wire_shape() {
    let dto = RemoteGenerateDto {
        phase: "generating",
        deck: None,
        filename: None,
        cards: None,
        elapsed: Some(4),
        error: None,
    };
    pin(
        "RemoteGenerateDto.generating",
        &dto,
        json!({
            "phase": "generating",
            "deck": null,
            "filename": null,
            "cards": null,
            "elapsed": 4,
            "error": null
        }),
    );
}

#[test]
fn remotegeneratedto_done_wire_shape() {
    let dto = RemoteGenerateDto {
        phase: "done",
        deck: Some("% link: https://example.org\n# Q\n\tA\n".to_string()),
        filename: Some("example-org.txt".to_string()),
        cards: Some(1),
        elapsed: None,
        error: None,
    };
    pin(
        "RemoteGenerateDto.done",
        &dto,
        json!({
            "phase": "done",
            "deck": "% link: https://example.org\n# Q\n\tA\n",
            "filename": "example-org.txt",
            "cards": 1,
            "elapsed": null,
            "error": null
        }),
    );
}

#[test]
fn remotegeneratedto_error_wire_shape() {
    let dto = RemoteGenerateDto {
        phase: "error",
        deck: None,
        filename: None,
        cards: None,
        elapsed: None,
        error: Some("the model returned no deck content".to_string()),
    };
    pin(
        "RemoteGenerateDto.error",
        &dto,
        json!({
            "phase": "error",
            "deck": null,
            "filename": null,
            "cards": null,
            "elapsed": null,
            "error": "the model returned no deck content"
        }),
    );
}
