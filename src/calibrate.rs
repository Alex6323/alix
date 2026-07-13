//! Grader-calibration probes: the single source for the hand-labeled answers
//! that spot-check whether a grading model is trustworthy.
//!
//! Two consumers share these fixtures. `tests/calibrate.rs` (`make calibrate`)
//! runs them one at a time against the maintainer's backend before a
//! `grade_*` prompt ships, and `alix doctor --grading` runs them batched
//! against the *user's* configured backend, so someone on a different CLI or
//! a cheaper model can see whether their exam grades can be trusted.
//!
//! Probes come in two kinds, and the split is the point. A [`Safety`] probe is
//! an answer that must NOT pass: if it does, the model grades leniently and
//! "mastered" lies — the one failure that matters. A [`Fairness`] probe is a
//! correct answer that should pass: failing it means the model is harsher
//! than intended — annoying, but the grades stay honest. Consumers report the
//! two with different severity. Six probes are a spot check, not a
//! certification; keep the fixtures clear-cut, never borderline.
//!
//! [`Safety`]: ProbeKind::Safety
//! [`Fairness`]: ProbeKind::Fairness

use anyhow::Result;

use crate::{
    config::{AskConfig, ExamConfig, Strictness},
    exam::{self, ExamQuestion, Verdict},
};

/// What a probe's expected outcome means for the grader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeKind {
    /// The answer must NOT pass; a pass means the grader is lenient and exam
    /// results overstate understanding.
    Safety,
    /// The answer should pass; a miss means the grader is stricter than
    /// calibrated, which frustrates but never lies.
    Fairness,
}

/// One hand-labeled grading case: a question, its rubric, an answer, and the
/// strictness it is graded at. Whether a `Pass` is required follows from
/// [`kind`](Self::kind).
pub struct Probe {
    pub name: &'static str,
    pub kind: ProbeKind,
    pub question: &'static str,
    pub points: &'static [&'static str],
    pub answer: &'static str,
    pub strictness: Strictness,
}

const MOVE_Q: &str = "Why does Rust move a value on assignment instead of copying it?";
const MOVE_POINTS: &[&str] = &[
    "ownership transfers so there is a single owner",
    "it prevents a double free / use-after-move",
];

const TCP_Q: &str = "Why does TCP use a three-way handshake to open a connection?";
const TCP_POINTS: &[&str] = &[
    "both sides exchange and agree on initial sequence numbers",
    "it confirms each side can both send and receive before data flows",
];

const BORROW_Q: &str = "What two guarantees does Rust's borrow checker enforce about references?";
const BORROW_POINTS: &[&str] = &[
    "many shared (immutable) references OR one mutable reference, never both at once",
    "a reference must never outlive the data it points to (no dangling references)",
];
// Covers the aliasing rule, omits the lifetime / dangling-reference rule.
const BORROW_HALF: &str = "You can have either many immutable references or a single \
                           mutable one, but never both at the same time.";

/// The six probes: four safety (a wrong, empty, off-topic, or
/// strictly-incomplete answer must not pass) and two fairness (a terse and an
/// incomplete-but-correct answer should pass at the right strictness).
pub const PROBES: &[Probe] = &[
    Probe {
        name: "confident_but_wrong",
        kind: ProbeKind::Safety,
        question: MOVE_Q,
        points: MOVE_POINTS,
        // Fluent, confident, and the exact opposite of the truth.
        answer: "Rust deep-copies the value on assignment, so both bindings own \
                 independent data and remain usable afterward.",
        strictness: Strictness::Balanced,
    },
    Probe {
        name: "empty_answer",
        kind: ProbeKind::Safety,
        question: TCP_Q,
        points: TCP_POINTS,
        answer: "",
        strictness: Strictness::Balanced,
    },
    Probe {
        name: "off_topic",
        kind: ProbeKind::Safety,
        question: TCP_Q,
        points: TCP_POINTS,
        // True statements, but they don't answer *why* the handshake is three-way.
        answer: "TCP is a transport-layer protocol that gives reliable, ordered byte \
                 streams and underpins HTTP, SMTP, and SSH.",
        strictness: Strictness::Balanced,
    },
    Probe {
        name: "terse_correct",
        kind: ProbeKind::Fairness,
        question: MOVE_Q,
        points: MOVE_POINTS,
        answer: "Ownership moves to the new binding and the old one is invalidated, so \
                 the value isn't freed twice.",
        strictness: Strictness::Balanced,
    },
    Probe {
        name: "strict_incomplete",
        kind: ProbeKind::Safety,
        question: BORROW_Q,
        points: BORROW_POINTS,
        answer: BORROW_HALF,
        strictness: Strictness::Strict,
    },
    Probe {
        name: "lenient_incomplete",
        kind: ProbeKind::Fairness,
        question: BORROW_Q,
        points: BORROW_POINTS,
        answer: BORROW_HALF,
        strictness: Strictness::Lenient,
    },
];

/// One probe's outcome against a live grader.
pub struct ProbeResult {
    pub name: &'static str,
    pub kind: ProbeKind,
    pub verdict: Verdict,
    /// Whether the grader behaved as calibrated for this probe's kind.
    pub ok: bool,
}

/// Runs every probe against the configured grader, batching the probes that
/// share a strictness into one [`exam::grade_answers`] call (three real calls
/// for the six probes). Batching mirrors production: a real exam grades all
/// its questions in one prompt. Results come back grouped by strictness, in
/// first-appearance order.
pub fn run(exam_cfg: &ExamConfig, ask_cfg: &AskConfig) -> Result<Vec<ProbeResult>> {
    let mut order: Vec<Strictness> = Vec::new();
    for p in PROBES {
        if !order.contains(&p.strictness) {
            order.push(p.strictness);
        }
    }
    let mut results = Vec::with_capacity(PROBES.len());
    for strictness in order {
        let group: Vec<&Probe> = PROBES
            .iter()
            .filter(|p| p.strictness == strictness)
            .collect();
        let questions: Vec<ExamQuestion> = group
            .iter()
            .map(|p| ExamQuestion {
                prompt: p.question.to_string(),
                points: p.points.iter().map(|x| x.to_string()).collect(),
            })
            .collect();
        let answers: Vec<String> = group.iter().map(|p| p.answer.to_string()).collect();
        let graded = exam::grade_answers(&questions, &answers, strictness, exam_cfg, ask_cfg)?;
        for (p, g) in group.iter().zip(graded.grades) {
            let ok = match p.kind {
                ProbeKind::Safety => g.verdict != Verdict::Pass,
                ProbeKind::Fairness => g.verdict == Verdict::Pass,
            };
            results.push(ProbeResult {
                name: p.name,
                kind: p.kind,
                verdict: g.verdict,
                ok,
            });
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{ask_config, exec_lock, fake_cli};

    #[test]
    fn probes_are_well_formed() {
        assert_eq!(6, PROBES.len());
        let mut names: Vec<&str> = PROBES.iter().map(|p| p.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(6, names.len(), "probe names must be unique");
        assert!(PROBES.iter().any(|p| p.kind == ProbeKind::Safety));
        assert!(PROBES.iter().any(|p| p.kind == ProbeKind::Fairness));
    }

    /// A fake grader that counts its invocations in `<dir>/calls.log` and
    /// answers every question in the batch with `verdict` — it reads the
    /// prompt and emits one grade per "Question N:" line, so one script
    /// serves batches of any size.
    fn fake_grader(dir: &std::path::Path, verdict: &str) -> std::path::PathBuf {
        let log = dir.join("calls.log");
        let body = format!(
            r#"PATH=/usr/bin:/bin
tmp="{dir}/prompt.$$"
cat > "$tmp"
echo x >> "{log}"
n=$(grep -c '^Question ' "$tmp")
printf '{{"grades":['
i=1
while [ "$i" -le "$n" ]; do
  [ "$i" -gt 1 ] && printf ','
  printf '{{"verdict":"{verdict}","feedback":"f","missed":[]}}'
  i=$((i+1))
done
printf ']}}'"#,
            dir = dir.display(),
            log = log.display(),
            verdict = verdict,
        );
        fake_cli(dir, &body)
    }

    #[test]
    fn run_maps_verdicts_to_ok_by_kind() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // A grader that passes EVERYTHING is maximally lenient: every safety
        // probe must report not-ok, every fairness probe ok.
        let cli = fake_grader(dir.path(), "pass");
        let results = run(&ExamConfig::default(), &ask_config(&cli)).unwrap();
        assert_eq!(PROBES.len(), results.len());
        for r in &results {
            match r.kind {
                ProbeKind::Safety => assert!(!r.ok, "{} must not be ok on a pass", r.name),
                ProbeKind::Fairness => assert!(r.ok, "{} must be ok on a pass", r.name),
            }
        }

        // And a grader that never passes flips both kinds.
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_grader(dir.path(), "partial");
        let results = run(&ExamConfig::default(), &ask_config(&cli)).unwrap();
        for r in &results {
            match r.kind {
                ProbeKind::Safety => assert!(r.ok, "{} must be ok on a partial", r.name),
                ProbeKind::Fairness => assert!(!r.ok, "{} must not be ok on a partial", r.name),
            }
        }
    }

    #[test]
    fn run_batches_one_call_per_strictness() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_grader(dir.path(), "pass");
        run(&ExamConfig::default(), &ask_config(&cli)).unwrap();
        let calls = std::fs::read_to_string(dir.path().join("calls.log")).unwrap();
        assert_eq!(
            3,
            calls.lines().count(),
            "six probes across three strictness levels must grade in three calls"
        );
    }
}
