# Dual-agent differential orchestrator

This standalone Python CLI runs Claude Code and Codex against one frozen
specification in isolated git worktrees. It preserves every prompt, transcript,
patch, state transition, repro, and score so an experiment can be inspected or
resumed without relying on chat history.

It is development tooling, not part of the Alix Cargo workspace.

## Setup

Requirements:

- Python 3.12 or newer
- `uv`
- `claude` and `codex` on `PATH`
- a Rust target repository with `make check`, `make mutants`, and `make gate`
- `cargo nextest` and `cargo mutants`

```sh
uv sync --project orchestrator
uv run --project orchestrator orchestrate --help
```

## Run

```sh
uv run --project orchestrator orchestrate run \
  --mode symmetric \
  --spec /absolute/path/spec.md \
  --plan /absolute/path/plan.md \
  --repo /absolute/path/target \
  --base main \
  --run-dir /absolute/path/runs \
  --max-fix-rounds 2
```

`--claude-model` and `--codex-model` pin each agent's model for the whole run
(`--claude-model opus`, `--codex-model gpt-5.6-sol`). Both are recorded in
`state.json`, so a resume reuses them. Without them each CLI follows its own
ambient default, which leaves the run unreproducible and can strand it on an
exhausted model's rate limit.

Resume or print the current report:

```sh
uv run --project orchestrator orchestrate resume \
  --run-dir /absolute/path/runs/<run-id>
uv run --project orchestrator orchestrate report \
  --run-dir /absolute/path/runs/<run-id>
```

The run root must be outside the target repository. Agent worktrees live under
its hidden `.worktrees/` directory. The visible run directory contains only the
frozen inputs and durable evidence: atomic `state.json`, transcripts, patch
snapshots, finding patches, scores, and the Markdown report.

## Protocols

Symmetric mode gives the byte-identical frozen spec and optional plan to both
agents. Implementation remains independent. Each review candidate is exported
without branch history into a neutral one-commit repository.

A review finding is actionable only when it supplies:

- an independently applicable test-only patch and exact test name
- only new integration test files under `tests/`, with existing tests untouched
- a test that compiles and fails against the exact reviewed commit
- a green untouched baseline after the patch is reverted
- an ordinary supported user trigger and concrete user-visible impact

The report records the reviewed SHA, patch SHA-256, commands, observed failure,
trigger, impact, and resolution. Submissions missing any evidence are
preferences and never reach the fix phase.

Asymmetric mode gives the implementation to one agent and a blank Rust stub
crate to the property author. The target spec must contain an exact compilable
Rust contract:

````markdown
## API

```rust
pub trait Counter {
    fn increment(&mut self) -> u64;
}
```
````

Without that section, the run reports a spec bug and stops. The property author
never receives the implementation worktree path. Its test files are copied
verbatim to the implementation branch and remain immutable during fix rounds.

## Scoring and landing

A branch is eligible only if `make check` passes on it. Among eligible
branches the lower penalty wins:

- each unresolved verified defect filed against the branch: 10,000
- each of the opponent's regression tests the branch fails: 10,000
- each missed mutant: 100
- each pedantic warning: 2
- changed lines: 0.001 each

The two 10,000 terms are deliberately equal. A defect is a defect whether it
was filed against this branch or only caught by the other agent's test, and
findings are directional: an agent that finds a bug in its opponent is never
asked whether its own branch has the same one.

Eligibility is `make check`, not `make gate`, so correctness disqualifies a
branch but test-completeness does not. Missed mutants are a graded cost
instead: a surviving mutant says the branch's tests are thin, which should
lose to a rival that is genuinely wrong only if nothing worse is on the table.

Exact ties and runs where no branch passes `make check` stop for a human
decision. Otherwise
landing requires the base ref to remain at its frozen SHA and its checkout to be
clean. The orchestrator commits the union of tests first, applies the winning
implementation second, then fast-forwards the base.

## Development gates

```sh
uv run --project orchestrator python -m unittest discover \
  -s orchestrator/tests
uv run --project orchestrator mypy \
  --config-file orchestrator/pyproject.toml \
  orchestrator/src orchestrator/tests
```

The optional real-agent smoke test is deliberately separate and costed:

```sh
uv run --project orchestrator python orchestrator/tests/live_smoke.py --live
```
