//! Grader calibration (QUALITY plan, step 4).
//!
//! These run the REAL `grade_prompt` through every calibrated backend CLI
//! against the hand-labeled probes in `alix::calibrate` (the single source
//! `alix doctor --grading` also runs), to catch the one failure mode the
//! deterministic tests structurally cannot: a *lenient* grader. "mastered" is
//! only as honest as this stays. Each backend grades at its CLI-default model
//! plus, where one is named, a weakest "floor" model, and every call prints
//! the backend, the requested model, and the model the stream reported, so a
//! PASS names what it certified.
//!
//! Every test here is `#[ignore]`d, so `cargo test` (and CI) compile them but
//! run none. Run them deliberately before every desktop/mobile release and
//! after changing `grade_prompt`, with `make calibrate` (needs each calibrated
//! backend CLI installed and logged in; makes real, costed calls). Unlike
//! doctor's batched spot-check, each test grades its one probe per backend and
//! model row in its own call, so a failure names exactly the drifted case.
//!
//! Two rules keep the probes robust to the model's nondeterminism. First,
//! fixtures are clear-cut, never borderline. Second, a Safety probe asserts
//! only that the verdict is not `Pass` (Partial or Fail are both fine) — only
//! a genuine leniency, an actual `Pass`, fails it. A failing calibration run
//! is not a code bug: it means `grade_prompt` drifted and should be re-tuned
//! (lenient drift on a Safety probe is the serious direction).

use alix::{
    ask::observed_model,
    backend::backend_for,
    calibrate::{PROBES, ProbeKind},
    config::{AskConfig, BackendKind, ExamConfig},
    exam::{ExamQuestion, Verdict, grade_answers},
};

// (backend, weakest "floor" model also probed; None = the CLI default only).
// Gemini is omitted: no login on the release machine. Codex has no floor:
// the ChatGPT-account CLI rejects every non-default model probed.
const CALIBRATED: [(BackendKind, Option<&str>); 3] = [
    (BackendKind::Claude, Some("haiku")),
    (BackendKind::Codex, None),
    (BackendKind::Copilot, Some("gpt-5-mini")),
];

#[test]
fn makefile_names_every_calibrated_cli() {
    let makefile = include_str!("../Makefile");
    let comment = makefile
        .split_once("# Grader calibration")
        .expect("Makefile documents the calibration target")
        .1
        .split_once("calibrate:")
        .expect("calibration documentation precedes its target")
        .0;

    for cli in ["claude", "codex", "copilot"] {
        assert!(
            comment.contains(cli),
            "Makefile calibrate documentation omits required `{cli}` CLI"
        );
    }
}

/// Grades the named probe in its own real-CLI call per calibrated backend and
/// model row, and asserts what its kind requires: a Fairness probe must
/// `Pass`, a Safety probe must NOT.
fn assert_probe(name: &str) {
    let p = PROBES
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no probe named {name:?} in alix::calibrate::PROBES"));
    for (kind, floor) in CALIBRATED {
        let rows = [None, floor];
        let rows = if floor.is_some() {
            &rows[..]
        } else {
            &rows[..1]
        };
        for &model in rows {
            let q = ExamQuestion {
                prompt: p.question.to_string(),
                points: p.points.iter().map(|x| x.to_string()).collect(),
            };
            let mut ask = AskConfig {
                backend: kind,
                model: model.map(str::to_string),
                ..AskConfig::default()
            };
            ask.command = backend_for(&ask)
                .expect("every calibrated backend is wired")
                .command()
                .to_string();
            let requested = model.unwrap_or("cli-default");
            let result = grade_answers(
                &[q],
                &[p.answer.to_string()],
                p.strictness,
                &ExamConfig::default(),
                &ask,
            )
            .unwrap_or_else(|e| {
                panic!(
                    "{name} on {} ({requested}): grade call failed. Is the `{}` CLI installed and logged in? {e:#}",
                    kind.name(),
                    ask.command
                )
            });
            let v = result.grades[0].verdict;
            println!(
                "calibrate: probe={name} backend={} requested={requested} observed={} verdict={v:?}",
                kind.name(),
                observed_model(kind.name()).unwrap_or_else(|| "unreported".to_string())
            );
            match p.kind {
                ProbeKind::Fairness => assert_eq!(
                    Verdict::Pass,
                    v,
                    "{name} on {} ({requested}): a correct answer was not passed",
                    kind.name()
                ),
                ProbeKind::Safety => assert_ne!(
                    Verdict::Pass,
                    v,
                    "{name} on {} ({requested}): an answer that must not pass was passed",
                    kind.name()
                ),
            }
        }
    }
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn confident_but_wrong_is_never_a_pass() {
    assert_probe("confident_but_wrong");
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn terse_but_correct_passes_at_balanced() {
    assert_probe("terse_correct");
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn an_empty_answer_does_not_pass() {
    assert_probe("empty_answer");
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn an_off_topic_answer_does_not_pass() {
    assert_probe("off_topic");
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn strict_fails_an_incomplete_answer() {
    assert_probe("strict_incomplete");
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn lenient_passes_the_same_incomplete_answer() {
    assert_probe("lenient_incomplete");
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn a_complete_correct_proof_passes() {
    assert_probe("math_proof_full");
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn a_complete_correct_derivation_passes() {
    assert_probe("math_derivation_full");
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn a_wrong_algebraic_step_does_not_pass() {
    assert_probe("math_wrong_algebra");
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn a_correct_answer_by_the_wrong_method_does_not_pass() {
    assert_probe("math_answer_without_method");
}

#[test]
#[ignore = "real backend CLIs; run with `make calibrate`"]
fn a_hollow_proof_with_no_mechanism_does_not_pass() {
    assert_probe("math_hollow_proof");
}
