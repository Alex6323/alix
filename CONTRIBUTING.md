# Contributing to `alix`

Thanks for wanting to help. `alix` is small on purpose, so the most useful thing
you can read before writing code is *what it's trying to be* — and what it's
deliberately not. This page is the contributor-facing distillation of the house
rules; [`CLAUDE.md`](CLAUDE.md) is the full version (it's also how the AI
collaborator on this repo is instructed). The testing expectations are in the
[house rules](#house-rules-the-craft-gate) below.

## What `alix` is

> **North star:** *alix turns the things you read into verified understanding —
> spaced repetition for retention, AI that checks you actually understand, not
> just remember.*

It's an AI-augmented spaced-repetition tool: a plain-text flashcard core with
Claude woven in (an ask-Claude tutor, AI deck generation, and an AI exam that
gates progress on verified understanding). The **library crate (`src/`) is the
single source of logic**; the TUI, the web server (`--serve`), and the CLI are
thin consumers.

**Focus is a feature.** `alix` does a few things exceptionally well rather than
many things adequately — and that only survives if the discipline is written
down and applied *while building*. So contributions are very welcome, but they
pass through a gate.

## Good first contributions

Bug reports, typo and documentation fixes, a clearer error message, a deck-format
example — these need no proposal and no ceremony. Just open a PR. The fit gate
below is only for *new features*; most contributions never touch it.

## Before you open a PR — the fit gate

The fit gate decides *whether* a change belongs. Apply it **before** writing
code — once something is built, sunk cost quietly biases everyone toward merging
it.

- **Default is no — but I'll always tell you why.** A new feature has to earn
  its place; keeping `alix` small is what keeps it good. That's not hostility,
  and it's rarely final — "no" usually means "not like this" or "not yet," not
  "never."
- **For anything non-trivial, open a [feature proposal](../../issues/new/choose)
  first.** Get a go/no-go before you build. A surprise PR for a big feature is
  the hardest to accept, however good the code — talking first means your effort
  lands somewhere it'll actually get merged.
- **Reach is welcome; scope must clear the gate.**
  - *Reach* = work that widens access to the **same one job**: install and
    onboarding, the web and TUI surfaces, backends, performance, reliability,
    docs. Admissible even though it deepens no step of the loop.
  - *Scope* = a **new job** — a new card type, a new subsystem, a feature that
    sits *beside* the core loop rather than deepening a step of it
    (review → understand → verify → retain). Name the step it deepens; if you
    can't, it's probably out.
- **Be wary of "it'll bring users."** Almost any feature can claim it, so on its
  own it doesn't help us decide — show which step of the loop it deepens instead.
- **Prefer extending an existing concept** (a `%` directive, an answer mode)
  over adding a subsystem the user has to learn. Conceptual surface area is the
  real cost, not lines of code.

### What `alix` is **not** (the NOT-list)

Negative space defines the focus. `alix` is **not**:

- a migration/compat layer for other SR tools (`import` is cards-only TSV);
- a SaaS with accounts — it's local-first, your plaintext files;
- an open-ended chat wrapper (the tutor, exam, and generator each serve a
  card/deck/source; the AI never floats free of the learning loop);
- a notes app / second brain (decks are for drilling, not storing everything);
- a gamified study suite (no streaks/XP/leaderboards for their own sake);
- a content marketplace (you bring or generate decks from *your* sources).

Proposals that move `alix` toward any of these are unlikely to land as-is. If
you think something here actually belongs in `alix`, that's a fair discussion —
open a proposal to change the list itself.

## Development setup

`alix` is a Rust project; everything goes through the **Makefile**.

| Command | What it does |
| --- | --- |
| `make build` | Compile. |
| `make test` | Run the test suite (the primary gate). |
| `make lint` | `cargo clippy --all-targets`. |
| `make fmt` | Format — **nightly** rustfmt (see below). |
| `make fmt-check` | Verify formatting without writing. |
| `make check` | `lint` + `test` — run before you call work done. |
| `make coverage` | Coverage report (`cargo-llvm-cov`, HTML). |
| `make eval` | Real-Claude grader-calibration evals (costed) — before touching `grade_*`. |
| `make run ARGS="exam mydeck.txt"` | Run the binary. |
| `make serve ARGS="review mydeck.txt"` | Web frontend. |
| `make book` | Serve the mdBook manual live. |

CI runs the same gates on every PR: `fmt` (nightly rustfmt), `check` (clippy +
tests, with `-Dwarnings`), and an informational `coverage` job.

### Formatting is nightly-only

`rustfmt.toml` uses nightly-only options, so **format with `make fmt`**
(`cargo +nightly fmt`). Do **not** run plain `cargo fmt` (stable) — it can't
apply the config and produces a large bogus diff. The tree has some pre-existing
drift, so don't reformat unrelated files; keep your diff to what you touched.

## House rules (the craft gate)

The fit gate decides *whether*; the craft gate decides *how well*. Match the
surrounding code; when in doubt, mirror it. The essentials:

- **Behavior goes in the library**, not a frontend, so the TUI and web surfaces
  share it. Frontends only consume.
- **Test-first for library logic** — write the test before (or alongside) new
  `src/` behavior, above all the AI plumbing's error paths. New behavior ships
  with a test written first; a bug fix ships with a fails-first regression test.
  Thin frontend glue is the exception.
- **`make check` and clippy must be green** before a change is done. CI denies
  warnings — fix the cause, don't silence it. If a lint is a genuine false
  positive, suppress it item-scoped with a one-line reason and prefer
  `#[expect(…)]` over `#[allow(…)]`.
- **No `unwrap` / `expect` / `panic!` in library paths** — use `?`, `.or(…)`,
  `unwrap_or_default()`, etc. `.expect("…")` only when the line directly above
  guarantees the invariant. `.unwrap()` in `#[cfg(test)]` code is fine.
- **Errors come in two layers.** Domain modules expose a `thiserror` enum;
  workflow code returns `anyhow::Result` with `bail!` + `.context(…)`. Messages
  are **lowercase, no trailing period** — `bail!("the deck declares no \`%
  source:\`")`.
- **Don't break card identity.** A card's id is `XxHash64(deck file name + its
  back lines)` — or, for cloze cards, its `hash_lines` with the `{{ }}`
  delimiters stripped, plus a per-hole index (see `Card::id` in `src/card.rs`).
  It ignores the front, notes, and comments on purpose. A careless change to the
  parser, `hash_lines`, or deck rewriting silently **wipes users' review
  progress** — preserve it.
- **Keep the UI calm.** Both frontends must stay distraction-free: only what the
  user needs right now. Default to *less* — cut a readout before adding one,
  tuck rare controls behind a menu. A noisy UI diff is treated like a failing
  test.
- **No new dependency without a one-line reason** in the PR/commit. Reach for
  std or an existing dep first.
- **Keep the two docs in sync.** `README.md` is the **reference** (deck format,
  every directive, all features); `docs/book/` is the **narrative manual**.
  Update whichever a user-facing change touches.
- **User-facing changes get a `CHANGELOG.md` entry** under `## [Unreleased]`
  (Added / Changed / Fixed). While we're pre-1.0, **break freely** — change
  renamed/removed flags and directives outright and record a **Breaking** note
  under Changed; no back-compat shims.
- **Prompt changes** (anything touching `grade_*`) ship with a `make eval` run
  and the calibration delta noted — that's how "mastered" stays honest.

## Commits & pull requests

- **Don't break the build, and run `make check` before pushing.**
- We keep a **linear history** — rebase your branch on `main` and avoid merge
  commits; a clean fast-forward is the goal.
- The PR template's checklist is the gate in miniature. If you can't honestly
  tick a fit-gate box, stop and open a discussion before going further.
- Commits made with AI assistance carry a `Co-Authored-By` trailer; keep that if
  you use it.

## License

By contributing, you agree your work is dual-licensed under
[MIT](LICENSE-MIT) and [Apache-2.0](LICENSE-APACHE), as described in the
README's Contribution clause. Unless you state otherwise, any contribution you
intentionally submit for inclusion is licensed as above, with no additional
terms.
