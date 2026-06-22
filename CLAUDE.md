# flash — project guide

`flash` is an **AI-augmented** spaced-repetition learning tool in Rust, with a
terminal (TUI) and a web frontend (`flash serve`). On top of a plain-text
flashcard core, Claude is woven in: an ask-Claude tutor on any card, AI deck
generation (`flash generate`), and the **AI exam** (`flash exam`) that gates
progression on verified understanding. The tool is increasingly AI-centric —
weight that when prioritizing. The **library crate is the single source of
logic**; the TUI, the web server, and the CLI are thin consumers. Put behavior
in the lib (`src/`), not in a frontend, so both surfaces share it.

## Dev commands — use the Makefile

| Command | What it does |
| --- | --- |
| `make build` | Compile. |
| `make test` | Run the test suite (the primary gate). |
| `make lint` | `cargo clippy --all-targets`. |
| `make fmt` | Format — **nightly** rustfmt (see below). |
| `make fmt-check` | Verify formatting without writing. |
| `make check` | `lint` + `test` — run before considering work done. |
| `make run ARGS="exam mydeck.txt"` | Run the binary with args. |

## Formatting is nightly-only

`rustfmt.toml` uses nightly-only options, so **formatting must go through the
nightly toolchain** (`make fmt` → `cargo +nightly fmt`). Do **not** run plain
`cargo fmt` (stable): it can't apply the config and reformats by different rules,
producing a large bogus diff. The tree also has some pre-existing rustfmt drift,
so don't reformat unrelated files as part of a change — keep your diff to what
you touched.

## Code style (Rust)

These are flash's house idioms — the things clippy and rustfmt *don't* catch and
that a change should match. The global rules (simple, readable, small focused
functions, meaningful names) still apply on top; this section is what's specific
to this codebase. When in doubt, mirror the surrounding code.

- **Errors come in two layers.** Domain modules (`deck`, `store`, `parser`)
  expose a `thiserror` enum for typed failures; workflow code (`exam`, `ask`,
  `generate`) returns `anyhow::Result` and uses `bail!` + `.context(...)` at call
  boundaries. Message style is **lowercase, no trailing period**, e.g.
  ``bail!("the deck declares no `% source:` to examine against")``. Add
  `.context(...)` where it helps the user locate the failure, not at every `?`.

- **No `unwrap` / `expect` / `panic!` in library paths.** Reach for `?`, `.or(…)`,
  `unwrap_or_default()`, or `unwrap_or_else(…)` instead. `.expect("…")` is allowed
  only when the line directly above guarantees the invariant (e.g.
  `child.stdin.take().expect("stdin was piped")`). In `#[cfg(test)]` code,
  `.unwrap()` on tempfiles and fixtures is fine.

- **Doc comments are prose, not signatures.** Every module opens with a `//!`
  summary; public items get a `///` sentence or two on *intent* (the why), not a
  restatement of the type. Keep field docs to a line.

- **Tests live inline; integration tests in `tests/`.** Unit tests go in a
  `#[cfg(test)] mod tests` at the bottom of the module they cover. `tests/` holds
  the end-to-end suites: `tests/cli.rs` drives the built binary as a subprocess
  (deterministic — temp decks + `--store`, no real Claude — so it runs in CI),
  and `tests/eval.rs` is the `#[ignore]`d real-Claude grader-calibration harness
  (`make eval`). Name tests as full snake_case sentences stating condition +
  expectation (`passing_the_exam_masters_an_undrilled_deck`). Anything that shells
  out to Claude uses the shared harness in `src/testutil.rs`: `fake_reply` (drains
  stdin then emits a canned reply — use it for fixed outputs to avoid the EPIPE
  race), `fake_cli`, `ask_config`, and wrap the call in `exec_lock()` (serializes
  fork/exec; poison-tolerant).

- **Keep threading in the lib.** Background work follows the `ask::spawn` shape: a
  function that spawns a thread and returns a `Receiver`; frontends only
  `try_recv`/poll. Don't spawn threads from the TUI or the web server.

- **Small idioms.** Chain `Option` precedence with `.or(…)`
  (`card.mode.or(deck.mode)`), not `match`. Write deck/store files atomically
  (write a `.tmp`, then `rename`). For repeated I/O errors, define a local
  `io_err` closure and `.map_err(io_err)`. Construct structs with
  `Struct { field, ..Default::default() }` rather than adding a builder — at
  these struct sizes the update syntax already covers "set some, default the
  rest" (a builder only earns its boilerplate with many optional fields plus
  validation).

- **CI denies warnings** (`RUSTFLAGS: -Dwarnings`), so the build must stay clean.
  Fix the cause rather than silence it — never suppress a real `dead_code` /
  `unused` / correctness lint just to get past CI. When a lint is a genuine false
  positive or the refactor it would force is worse than the lint (e.g.
  `clippy::too_many_arguments` on a server/picker entry point), suppress it
  **item-scoped with a one-line reason**, and prefer **`#[expect(…)]` over
  `#[allow(…)]`**: an `#[expect]` is self-cleaning — once the lint stops firing,
  the compiler flags it (`unfulfilled_lint_expectations`, an error under
  `-Dwarnings`), so a stale suppression can't linger. Keep `#[allow]` only for
  broad module-/crate-level or conditionally-firing cases, where an expectation
  would be spuriously unfulfilled.

## Conventions

- **Test-first for library logic.** Write the test before (or alongside) new
  `src/` behavior — above all the AI plumbing's error paths, where bugs hide and
  local runs can mask races (the `testutil` fake-CLI tests exist because one such
  race slipped through). Thin frontend glue (TUI / `serve` wiring) is the
  exception — a follow-up or manual check is enough there. This is the
  deterministic half of the QUALITY plan; the grader-calibration evals
  (`make eval`) are the AI half, run deliberately before touching `grade_*`.
- **Tests and clippy must be green** before a change is done (`make check`).
  Formatting is run deliberately with `make fmt`, not enforced as a gate.
- Don't commit unless asked; never push without permission.
- **Two docs, two jobs — keep both in sync, judge which.** `README.md` is the
  **reference**: the deck format, every `%`/card directive, and all features
  (start with the "Directives at a glance" table) — update it whenever you add or
  change a directive or feature. `docs/book/` is the **narrative user manual**
  (mdBook; `make book`) — when a change affects how a feature is explained or
  used, update the relevant chapter too. Use judgment: significant user-facing
  changes warrant a book edit, internal refactors don't.
- **User-facing changes get a `CHANGELOG.md` entry** under `## [Unreleased]`
  (Keep a Changelog format: Added / Changed / Fixed). Internal refactors and
  test-only changes don't.
- **Don't break card identity.** A card's id is
  `XxHash64(deck file name + its back lines)` — or, for **cloze** cards, its
  `hash_lines`: each line's text with the `{{ }}` delimiters stripped (so
  restyling the markup doesn't reshuffle ids), plus a per-hole index — see
  `Card::id` (`src/card.rs`). It deliberately ignores the front, notes, and
  comments, so
  editing those preserves a card's review history while changing a back line
  resets it. Preserve this whenever you touch the parser, `hash_lines`, or deck
  rewriting — a careless change silently wipes users' progress.
- Roadmap and design rationale live in `ROADMAP.md` (gitignored, local).
