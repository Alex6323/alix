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
- a Rust target repository with `make check` and `make gate`
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

The lower penalty wins:

- failed gate: 1,000,000
- each unresolved verified defect: 10,000
- cross-test failure rate: up to 1,000
- each missed mutant: 100
- each pedantic warning: 2
- changed lines: 0.001 each

Exact ties and runs with no passing gate stop for a human decision. Otherwise
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
