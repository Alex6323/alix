//! Grader-calibration evals (QUALITY plan, step 4).
//!
//! These run the REAL `grade_prompt` through the `claude` CLI against
//! hand-labeled, adversarial answers, to catch the one failure mode the
//! deterministic tests structurally cannot: a *lenient* grader. "mastered" is
//! only as honest as this stays.
//!
//! Every test here is `#[ignore]`d, so `cargo test` (and CI) compile them but
//! run none. Run them deliberately — before shipping a change to `grade_prompt`
//! — with `make eval` (needs the `claude` CLI installed and logged in; makes
//! real, costed calls).
//!
//! Two rules keep them robust to the model's nondeterminism. First, fixtures are
//! clear-cut, never borderline. Second, a "must not pass" case asserts that the
//! verdict is not `Pass` (Partial or Fail are both fine) — only a genuine
//! leniency, an actual `Pass`, fails it. A failing eval is not a code bug: it
//! means `grade_prompt` drifted lenient and should be tightened.

use flash::{
    config::{AskConfig, ExamConfig, Strictness},
    exam::{ExamQuestion, Verdict, grade_answers},
};

/// Grades one `answer` against `points` at `strictness`, via the real CLI.
fn verdict(prompt: &str, points: &[&str], answer: &str, strictness: Strictness) -> Verdict {
    let q = ExamQuestion {
        prompt: prompt.to_string(),
        points: points.iter().map(|p| p.to_string()).collect(),
    };
    let result = grade_answers(
        &[q],
        &[answer.to_string()],
        strictness,
        &ExamConfig::default(),
        &AskConfig::default(),
    )
    .expect("grade call failed — is the `claude` CLI installed and logged in?");
    result.grades[0].verdict
}

const MOVE_Q: &str = "Why does Rust move a value on assignment instead of copying it?";
const MOVE_POINTS: &[&str] = &[
    "ownership transfers so there is a single owner",
    "it prevents a double free / use-after-move",
];

#[test]
#[ignore = "real claude CLI; run with `make eval`"]
fn confident_but_wrong_is_never_a_pass() {
    let v = verdict(
        MOVE_Q,
        MOVE_POINTS,
        // Fluent, confident, and the exact opposite of the truth.
        "Rust deep-copies the value on assignment, so both bindings own \
         independent data and remain usable afterward.",
        Strictness::Balanced,
    );
    assert_ne!(Verdict::Pass, v, "a confident-but-wrong answer was passed");
}

#[test]
#[ignore = "real claude CLI; run with `make eval`"]
fn terse_but_correct_passes_at_balanced() {
    let v = verdict(
        MOVE_Q,
        MOVE_POINTS,
        "Ownership moves to the new binding and the old one is invalidated, so \
         the value isn't freed twice.",
        Strictness::Balanced,
    );
    assert_eq!(Verdict::Pass, v, "a terse-but-correct answer was failed");
}

const TCP_Q: &str = "Why does TCP use a three-way handshake to open a connection?";
const TCP_POINTS: &[&str] = &[
    "both sides exchange and agree on initial sequence numbers",
    "it confirms each side can both send and receive before data flows",
];

#[test]
#[ignore = "real claude CLI; run with `make eval`"]
fn an_empty_answer_does_not_pass() {
    let v = verdict(TCP_Q, TCP_POINTS, "", Strictness::Balanced);
    assert_ne!(Verdict::Pass, v, "an empty answer was passed");
}

#[test]
#[ignore = "real claude CLI; run with `make eval`"]
fn an_off_topic_answer_does_not_pass() {
    let v = verdict(
        TCP_Q,
        TCP_POINTS,
        // True statements, but they don't answer *why* the handshake is three-way.
        "TCP is a transport-layer protocol that gives reliable, ordered byte \
         streams and underpins HTTP, SMTP, and SSH.",
        Strictness::Balanced,
    );
    assert_ne!(Verdict::Pass, v, "an off-topic answer was passed");
}

const BORROW_Q: &str = "What two guarantees does Rust's borrow checker enforce about references?";
const BORROW_POINTS: &[&str] = &[
    "many shared (immutable) references OR one mutable reference, never both at once",
    "a reference must never outlive the data it points to (no dangling references)",
];
// Covers the aliasing rule, omits the lifetime / dangling-reference rule.
const BORROW_HALF: &str = "You can have either many immutable references or a single \
                           mutable one, but never both at the same time.";

#[test]
#[ignore = "real claude CLI; run with `make eval`"]
fn strict_fails_an_incomplete_answer() {
    let v = verdict(BORROW_Q, BORROW_POINTS, BORROW_HALF, Strictness::Strict);
    assert_ne!(
        Verdict::Pass,
        v,
        "strict passed an answer that omits a required point"
    );
}

#[test]
#[ignore = "real claude CLI; run with `make eval`"]
fn lenient_passes_the_same_incomplete_answer() {
    let v = verdict(BORROW_Q, BORROW_POINTS, BORROW_HALF, Strictness::Lenient);
    assert_eq!(Verdict::Pass, v, "lenient failed a roughly-right answer");
}
