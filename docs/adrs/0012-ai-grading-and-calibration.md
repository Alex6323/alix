# 0012: AI grading and calibration

- Status: Accepted
- Evidence: cargo test --test calibrate -- --ignored --nocapture --test-threads=1 in Makefile
- Recorded: 2026-07-24
- Retrospective: Yes

## Decision history

Commit `8b22a6b` introduced live grader calibration probes on 2026-06-22.
Commit `b383e3f` exposed grading spot checks through doctor on 2026-07-13, and
`da0dffa` added mathematical derivation probes on 2026-07-15. Commit `6c37164`
made live calibration a required desktop and mobile release step on
2026-07-24.

## Context

Open reconstruction and source-grounded exams test meaning, completeness, and
contradiction. Exact string comparison cannot determine whether different
wording demonstrates the required understanding. Self-grading alone would let
learners mark semantically wrong answers as mastery and weaken the product's
exam claims.

Model grading is probabilistic and can drift when the configured provider or
model changes even if repository code does not. Deterministic software tests
cannot measure that live behavior, while live model calls are too
authenticated, costed, and non-deterministic for blocking CI.

## Decision

Alix uses a configured language model as the semantic evaluator for open
reconstruction where string comparison cannot establish correctness. Prompts
supply the question, required points, learner answer, strictness, and a closed
verdict vocabulary.

Grader output is parse-or-error. Exam JSON must deserialize into the expected
shape with the expected number of grades. Trace grading must begin with a
recognized verdict. Malformed, missing, or unrecognized output never
fabricates a passing result.

Two verification layers have different jobs:

- deterministic tests validate prompts, parsing, thresholds, failure handling,
  and state transitions using fake CLIs;
- ignored live calibration tests evaluate clear-cut safety and fairness probes
  against the configured real model.

`make calibrate` is a deliberate release gate for desktop and mobile. Safety
probes are false-pass-sensitive: an answer known not to merit Pass must not
receive Pass. A failed run is evidence to investigate; a later lucky rerun
does not erase it.

## Consequences

- Semantic exams can recognize correct paraphrases and reject substantive
  contradictions.
- Mastery depends partly on an external probabilistic evaluator.
- Releases require authenticated, costed manual calibration.
- CI remains deterministic and cannot by itself certify current model
  behavior.
- Provider or model changes can block a release without a code change.
- Grading errors remain visible instead of turning uncertainty into success.

## Alternatives considered

### Exact string or keyword grading

This is deterministic but cannot reliably judge paraphrase, reasoning,
contradiction, or coverage of several required points.

### Learner self-grading for semantic exams

Self-assessment is appropriate in ordinary review modes, but using it as the
mastery evaluator would make the exam gate circular and inconsistent.

### Live model tests in blocking CI

CI calls would require secrets and budget, introduce provider availability
failures, and make the required gate non-reproducible.

### Rerun until calibration passes

This converts nondeterminism into a green-by-chance policy and hides the
false-pass risk the probes exist to reveal.

### Treat malformed output as Partial or Fail

Fail-closed grading avoids a false Pass, but inventing a verdict still hides a
provider or parser fault. An explicit error is more honest and diagnosable.

## Compatibility

Verdict names and resulting progress transitions are product behavior.
Prompt wording and provider-specific output extraction remain implementation
details as long as they preserve the parse-or-error and calibration
constraints.

## Security

Learner answers and source-derived questions are sent to the configured
provider. The grading prompt is self-contained and `source_access` is disabled;
provider tool isolation still has the backend-specific limits recorded in ADR
0009. Model output is untrusted input and must be parsed into bounded types
before affecting progress.

The highest-risk error is a false Pass because it can assert mastery without
evidence. Calibration therefore gives safety probes an asymmetric role.

## Verification

- `src/exam.rs` owns structured semantic grading and refuses malformed or
  incomplete result sets.
- `src/trace_ai.rs` refuses unrecognized verdicts rather than inferring one.
- `src/calibrate.rs` defines shared safety and fairness probes.
- `tests/calibrate.rs` contains ignored real-provider tests with clear-cut
  expected outcomes.
- Deterministic fake-CLI tests cover parsing and failure paths.
- `RELEASING.md` requires `make calibrate` and forbids lucky-rerun release
  reasoning.

## Reversal

Replace model grading when a deterministic evaluator can meet the same
semantic requirements, or evidence shows the calibrated model boundary cannot
achieve acceptable false-pass behavior. A replacement must define mastery
semantics, migrate affected workflows, and provide an equally explicit
behavioral release gate.
