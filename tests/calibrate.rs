//! Grader calibration (QUALITY plan, step 4).
//!
//! These run the REAL `grade_prompt` through the `claude` CLI against the
//! hand-labeled probes in `alix::calibrate` (the single source `alix doctor
//! --grading` also runs), to catch the one failure mode the deterministic
//! tests structurally cannot: a *lenient* grader. "mastered" is only as
//! honest as this stays.
//!
//! Every test here is `#[ignore]`d, so `cargo test` (and CI) compile them but
//! run none. Run them deliberately, before shipping a change to `grade_prompt`,
//! with `make calibrate` (needs the `claude` CLI installed and logged in; makes
//! real, costed calls). Unlike doctor's batched spot-check, each test grades
//! its one probe in its own call, so a failure names exactly the drifted case.
//!
//! Two rules keep the probes robust to the model's nondeterminism. First,
//! fixtures are clear-cut, never borderline. Second, a Safety probe asserts
//! only that the verdict is not `Pass` (Partial or Fail are both fine) — only
//! a genuine leniency, an actual `Pass`, fails it. A failing calibration run
//! is not a code bug: it means `grade_prompt` drifted and should be re-tuned
//! (lenient drift on a Safety probe is the serious direction).

use alix::{
    calibrate::{PROBES, ProbeKind},
    config::{AskConfig, ExamConfig},
    exam::{ExamQuestion, Verdict, grade_answers},
};

/// Grades the named probe in its own real-CLI call and asserts what its kind
/// requires: a Fairness probe must `Pass`, a Safety probe must NOT.
fn assert_probe(name: &str) {
    let p = PROBES
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no probe named {name:?} in alix::calibrate::PROBES"));
    let q = ExamQuestion {
        prompt: p.question.to_string(),
        points: p.points.iter().map(|x| x.to_string()).collect(),
    };
    let result = grade_answers(
        &[q],
        &[p.answer.to_string()],
        p.strictness,
        &ExamConfig::default(),
        &AskConfig::default(),
    )
    .expect("grade call failed — is the `claude` CLI installed and logged in?");
    let v = result.grades[0].verdict;
    match p.kind {
        ProbeKind::Fairness => {
            assert_eq!(Verdict::Pass, v, "{name}: a correct answer was not passed")
        }
        ProbeKind::Safety => assert_ne!(
            Verdict::Pass,
            v,
            "{name}: an answer that must not pass was passed"
        ),
    }
}

#[test]
#[ignore = "real claude CLI; run with `make calibrate`"]
fn confident_but_wrong_is_never_a_pass() {
    assert_probe("confident_but_wrong");
}

#[test]
#[ignore = "real claude CLI; run with `make calibrate`"]
fn terse_but_correct_passes_at_balanced() {
    assert_probe("terse_correct");
}

#[test]
#[ignore = "real claude CLI; run with `make calibrate`"]
fn an_empty_answer_does_not_pass() {
    assert_probe("empty_answer");
}

#[test]
#[ignore = "real claude CLI; run with `make calibrate`"]
fn an_off_topic_answer_does_not_pass() {
    assert_probe("off_topic");
}

#[test]
#[ignore = "real claude CLI; run with `make calibrate`"]
fn strict_fails_an_incomplete_answer() {
    assert_probe("strict_incomplete");
}

#[test]
#[ignore = "real claude CLI; run with `make calibrate`"]
fn lenient_passes_the_same_incomplete_answer() {
    assert_probe("lenient_incomplete");
}
