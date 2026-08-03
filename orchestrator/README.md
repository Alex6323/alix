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
snapshots, finding patches, scores, an append-only `progress.log`, and the
Markdown report. Progress is also mirrored to stderr. Agent entries include a
periodic elapsed-time heartbeat and the number of changed worktree paths, so a
long invocation remains observable without exposing partial model prose.

Independent agent calls within one protocol phase run concurrently.
Per-worktree patch validation and commits remain isolated. Shared state saves,
review repro verification, and both `make gate` runs are serialized. Interrupting
a parallel phase terminates every active agent process group before returning
control to the resumable state machine.

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
The stub inherits the target's `rustfmt.toml` and nightly pin when present, and
the engine formats the property suite with that toolchain before the copy.

## Scoring and landing

Correctness is an eligibility boundary, not a score. A candidate is eligible
only when:

- `make check` passes
- no verified defect remains unresolved

Raw cross-tests are opponent-authored evidence, not a neutral gate. Among
eligible candidates, the lower quality penalty wins: up to 1,000 points for
cross-test failure rate, 100 points for each missed mutant, two points for each
pedantic warning added beyond the frozen base, plus 0.001 per changed line.
`make gate` still runs and its mutation result remains visible, but a survivor
measures the candidate tests rather than making an otherwise correct
implementation ineligible. A symmetric run requires every candidate to be
eligible before making a recommendation; one surviving branch is not a
complete comparison. Exact ties, incomplete comparisons, and runs with no
eligible candidate stop for a human decision. Otherwise landing requires the
base ref to remain at its frozen SHA and its checkout to be clean. The
orchestrator commits the union of tests first, applies the winning
implementation second, then fast-forwards the base.

When `make check` fails, mutation testing does not run and the report renders
the mutation result as `skipped`, never as zero missed mutants.

Runs retain full agent worktrees, build outputs, neutral review exports, and
evidence. The first full Alix run used about 7.6 GB. Budget at least 10 GB for a
comparable run; concurrent agents raise peak usage and can otherwise fail
mid-run with ENOSPC. This machine sweeps files under `~/tmp` after seven
untouched days, so put `--run-dir` outside `~/tmp` when the evidence must remain
inspectable. Build scratch and disposable worktrees may still live there.

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
