<!--
Thanks for contributing to alix! This checklist mirrors the fit gate and craft
gate from CONTRIBUTING.md. Small fix or docs-only PR? Tick "Docs / internal"
below and skip whatever doesn't apply — don't sweat every box. A box you can't
tick on a feature PR is a useful signal it isn't ready yet.
-->

## What this PR does

<!-- One or two plain sentences. What changes for the user, or why this exists. -->

## Which kind of change is this? (tick exactly one)

- [ ] **Reach** — widens access to alix's one job (install, onboarding, web/TUI, a backend, performance, reliability, docs). Adds no new scope.
- [ ] **Deepens the loop** — sharpens an existing step of review → understand → verify → retain. Step it deepens: `____`
- [ ] **Fix** — corrects a bug; no new capability.
- [ ] **Docs / internal** — README, book, refactor, tests, CI. No user-facing behavior change.

## Fit gate — this PR does **NOT** (tick all; if you can't tick one, stop and open a discussion first)

- [ ] add a **new job / card / deck concept** the user must learn that doesn't beat merging into an existing one
- [ ] move alix toward the **NOT-list** — accounts/SaaS · notes app or second brain · open-ended chat wrapper · gamification (streaks/XP/leaderboards) · content marketplace · full SR-tool migration layer
- [ ] add a **dependency** without a one-line reason (below or in the commit)
- [ ] add always-on UI chrome that's rarely useful (status ladders, persistent counters, decorative readouts)

> Non-trivial feature? Link the issue/discussion where its fit was agreed **before** the code was written: #____

## Craft gate (tick all that apply)

- [ ] `make check` is green (clippy + tests; CI runs with `-Dwarnings`)
- [ ] formatted with `make fmt` (nightly rustfmt — **not** plain `cargo fmt`)
- [ ] new behavior has a test written first; a bug fix has a fails-first regression test
- [ ] behavior lives in the **library** (`src/`), not a frontend; both surfaces share it
- [ ] no `unwrap` / `expect` / `panic!` in library paths (test code excepted)
- [ ] **card identity preserved** — didn't reset users' progress via `hash_lines` / back-line hashing changes
- [ ] docs synced: `README.md` (reference) + `docs/book/` (manual) for user-facing changes
- [ ] `CHANGELOG.md` entry under `## [Unreleased]` for user-facing changes (Breaking note for a 0.x rename/removal)
- [ ] new dependency? a one-line reason is included
- [ ] touched a `grade_*` prompt? ran `make eval` and noted the calibration delta below

## Anything reviewers should know

<!-- Trade-offs, follow-ups, the calibration delta if you ran make eval, screenshots for UI changes. -->
