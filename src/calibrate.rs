//! Grader-calibration probes: the single source for the hand-labeled answers
//! that spot-check whether a grading model is trustworthy.

use anyhow::Result;

use crate::{
    config::{AskConfig, ExamConfig, Strictness},
    exam::{self, ExamQuestion, Verdict},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeKind {
    Safety,
    Fairness,
}

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

const SQRT2_Q: &str = "Prove that the square root of 2 is irrational.";
const SQRT2_POINTS: &[&str] = &[
    "assume for contradiction that sqrt(2) is rational, written as a fraction a/b in lowest terms",
    "derive a^2 = 2 b^2, so a^2 is even, therefore a is even",
    "write a = 2k and substitute to get b^2 = 2 k^2, so b is even too",
    "a and b both even contradicts lowest terms, so the assumption fails and sqrt(2) is irrational",
];

const DERIV_Q: &str = "Using the limit definition of the derivative (first principles), derive the \
                       derivative of f(x) = x^2. Show the steps.";
const DERIV_POINTS: &[&str] = &[
    "start from the limit definition: f'(x) = lim as h->0 of [f(x+h) - f(x)] / h",
    "substitute f(x) = x^2 and expand (x+h)^2 = x^2 + 2xh + h^2",
    "simplify the numerator to 2xh + h^2 and divide by h (h != 0) to get 2x + h",
    "take the limit as h -> 0 to get f'(x) = 2x",
];

pub const PROBES: &[Probe] = &[
    Probe {
        name: "confident_but_wrong",
        kind: ProbeKind::Safety,
        question: MOVE_Q,
        points: MOVE_POINTS,
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
    Probe {
        name: "math_proof_full",
        kind: ProbeKind::Fairness,
        question: SQRT2_Q,
        points: SQRT2_POINTS,
        answer: "Suppose sqrt(2) were rational. Then sqrt(2) = a/b with a and b integers \
                 sharing no common factor (lowest terms). Squaring, 2 = a^2/b^2, so a^2 = 2 b^2. \
                 Thus a^2 is even, so a is even, say a = 2k. Substituting, 4 k^2 = 2 b^2, so \
                 b^2 = 2 k^2, meaning b^2 is even and b is even too. But then a and b are both \
                 even, sharing a factor of 2, contradicting lowest terms. So sqrt(2) is irrational.",
        strictness: Strictness::Balanced,
    },
    Probe {
        name: "math_derivation_full",
        kind: ProbeKind::Fairness,
        question: DERIV_Q,
        points: DERIV_POINTS,
        answer: "By definition f'(x) = lim_{h->0} [f(x+h) - f(x)] / h. With f(x)=x^2 this is \
                 [(x+h)^2 - x^2]/h. Expanding, (x+h)^2 = x^2 + 2xh + h^2, so the numerator is \
                 2xh + h^2. Dividing by h (h != 0) gives 2x + h, and as h -> 0 that tends to 2x. \
                 So f'(x) = 2x.",
        strictness: Strictness::Balanced,
    },
    // Drops the 2xh cross term, yielding 0: a lenient grader checking only
    // the setup would pass it.
    Probe {
        name: "math_wrong_algebra",
        kind: ProbeKind::Safety,
        question: DERIV_Q,
        points: DERIV_POINTS,
        answer: "f'(x) = lim_{h->0} [(x+h)^2 - x^2]/h. Now (x+h)^2 = x^2 + h^2, so the numerator \
                 is h^2, and h^2/h = h, which goes to 0. So f'(x) = 0.",
        strictness: Strictness::Balanced,
    },
    // Correct final answer (2x) via the wrong method (power rule, not first
    // principles): grading the answer alone would wrongly pass it.
    Probe {
        name: "math_answer_without_method",
        kind: ProbeKind::Safety,
        question: DERIV_Q,
        points: DERIV_POINTS,
        answer: "By the power rule, the derivative of x^n is n*x^(n-1). So for x^2 we bring down \
                 the 2 and subtract 1 from the exponent, giving 2*x^1 = 2x.",
        strictness: Strictness::Balanced,
    },
    // Names the technique with zero mechanism: fluent and hollow, no
    // evidence of understanding.
    Probe {
        name: "math_hollow_proof",
        kind: ProbeKind::Safety,
        question: SQRT2_Q,
        points: SQRT2_POINTS,
        answer: "This is a proof by contradiction. We assume sqrt(2) is rational, so sqrt(2) = a/b \
                 for some integers. Working through the algebra leads to a contradiction, which \
                 shows our assumption was wrong. Therefore sqrt(2) must be irrational.",
        strictness: Strictness::Balanced,
    },
];

pub struct ProbeResult {
    pub name: &'static str,
    pub kind: ProbeKind,
    pub verdict: Verdict,
    pub ok: bool,
}

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
        let mut names: Vec<&str> = PROBES.iter().map(|p| p.name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(total, names.len(), "probe names must be unique");
        assert!(PROBES.iter().any(|p| p.kind == ProbeKind::Safety));
        assert!(PROBES.iter().any(|p| p.kind == ProbeKind::Fairness));
    }

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
        let cli = fake_grader(dir.path(), "pass");
        let results = run(&ExamConfig::default(), &ask_config(&cli)).unwrap();
        assert_eq!(PROBES.len(), results.len());
        for r in &results {
            match r.kind {
                ProbeKind::Safety => assert!(!r.ok, "{} must not be ok on a pass", r.name),
                ProbeKind::Fairness => assert!(r.ok, "{} must be ok on a pass", r.name),
            }
        }

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
            "probes across three strictness levels must grade in three calls"
        );
    }
}
