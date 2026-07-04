# `alix` — project guide

`alix` is an **AI-augmented** spaced-repetition learning tool in Rust, with a
terminal (TUI) and a web frontend (`--serve`). On top of a plain-text
flashcard core, an AI backend is woven in: an in-session tutor on any card, AI
deck generation (`alix deck`), and the **AI exam** (`alix exam`) that gates
progression on verified understanding. The AI backend is pluggable (`[ask]
backend` — Claude by default; Gemini, Codex, and Copilot also supported). The
tool is increasingly AI-centric — weight that when prioritizing. The **library
crate is the single source of logic**; the TUI, the web server, and the CLI are
thin consumers. Put behavior in the lib (`src/`), not in a frontend, so both
surfaces share it.

Native mobile thin clients (Android/iOS) are anticipated, consuming these same
web endpoints — so treat the web JSON API as a **client-agnostic contract**:
review flow and session state live in the lib behind presentation-agnostic
endpoints, never in page JS (a native client can't reuse logic trapped in the
page). alix itself stays a plain bind-to-interface HTTP server — reaching it
beyond the LAN is an operator deployment choice (VPN/reverse proxy), not a
TLS/auth/accounts subsystem to grow here.

## Scope & focus

`alix` aims to do a few things well rather than many adequately.
Focus is a feature, and it survives only if the discipline is written down. New
capability here originates mostly in conversation with Claude, so this is the
gate that actually fires — apply it *while building* and push back on scope
creep rather than quietly saying yes.

**North star** (a fixed reference, not a destination — you don't "complete" it,
it keeps you on heading): *alix aims to turn the things you read into verified
understanding — spaced repetition for retention, AI that checks you actually
understand, not just remember.* Measure every feature call against this.

**What `alix` is NOT** — negative space defines the focus; propose additions,
don't silently rewrite it:

- not a full migration/compat layer for other SR tools — `import` is cards-only
  TSV, not a scheduling/lock-in importer.
- not a SaaS with accounts — local-first, your plaintext files.
- not an open-ended chat wrapper — the tutor, exam, and generator each serve a
  card/deck/source; the AI never floats free of the learning loop.
- not a notes app / second brain — decks are for drilling, not for storing
  everything you know.
- not a gamified study suite — no streaks/XP/leaderboards for their own sake.
- not a content marketplace — you bring or generate decks from *your* sources.

**Fit gate — apply these before code is written.** A feature that can't clear
them probably shouldn't be built:

- **Default is no.** The feature carries the burden of proof; a rejection never
  needs justifying.
- **Name the core-loop step it deepens** — review → understand → verify →
  retain. If you can't name one, it's out. Expansion that deepens the one job
  still counts (sharper exam grading deepens *verify*); breadth that sits beside
  the loop doesn't (a calendar, a habit tracker).
- **Reach counts too, but reach is not scope.** Work that widens access to the
  same one job — install and onboarding, the web and TUI surfaces, backends,
  performance, reliability — is admissible even though it deepens no step of
  the loop; it lets more people reach the one job. The guard: widen reach
  *without* adding scope the NOT-list rules out, and never let "it'll bring
  users" substitute for clearing this gate — almost anything can claim that.
- **Conceptual surface area is the real cost, not lines of code.** Prefer
  extending an existing concept (a `%` directive, a mode) over adding a
  subsystem the user must learn. A new concept must beat *merging into* or
  *replacing* an existing one.
- **Subtraction test, both directions.** Would removing it make `alix`
  meaningfully worse at its one job? And periodically: is anything already in
  `alix` failing that test and worth retiring or merging?
- **Settle it in plan mode.** For any non-trivial feature, state these answers
  and get an explicit go/no-go before implementing — once it's built, sunk cost
  quietly biases toward merging.

The **craft gate** is the rest of this guide (`make check`, test-first for the
lib/AI error paths, behavior in the lib, README + book + CHANGELOG synced, no new
dependency without a one-line reason). The fit gate decides *whether*; the craft
gate decides *how*.

**UI noise is a gate too — clean, no-distraction surfaces.** Both frontends (TUI
and web) must stay calm: only what the user needs *right now*, nothing competing
for attention. Every pixel/char earns its place — chrome that's always on but
rarely useful (status ladders, persistent counters, decorative readouts) is
noise, not information. When you add UI, the default is *less*: prefer one
primary action, tuck secondary/rare controls behind a menu, and cut a readout
before adding one. A long or hand-crafted label **truncates** (ellipsis) — it
never wraps or reflows the layout; headers and bars hold a fixed size regardless
of content. When in doubt, remove it; if it's genuinely useful but rare, hide it
behind a `⋮`/`m`/`?`-style affordance rather than leaving it on screen. Treat a
noisy diff the way you'd treat a failing test: not done yet.

## Dev commands — use the Makefile

| Command | What it does |
| --- | --- |
| `make build` | Compile. |
| `make test` | Run the test suite (the primary gate). |
| `make lint` | `cargo clippy --all-targets`. |
| `make fmt` | Format — **nightly** rustfmt (see below). |
| `make fmt-check` | Verify formatting without writing. |
| `make check` | `lint` + `test` — the fast, lenient inner-loop gate; run before considering work done. |
| `make ci` | **Full CI parity** — `fmt-check` + `check` under `-Dwarnings` + `coverage`, exactly as `.github/workflows/ci.yml` runs them. A green `make ci` predicts a green CI; run it before a push/release. |
| `make coverage` | Coverage report via `cargo-llvm-cov` (HTML). |
| `make eval` | Real-Claude grader-calibration evals (`tests/eval.rs`, costed) — before touching `grade_*`. |
| `make run ARGS="exam mydeck.txt"` | Run the binary with args. |
| `make serve ARGS="review mydeck.txt"` | Web frontend (`--serve`); no ARGS → in-browser picker. |
| `make book` | Serve the mdBook manual (`docs/book`), live reload. |
| `make site` | Preview the `alix.study` landing page locally (`site/`). |
| `make install` | `cargo install --path .`. |
| `make clean` | `cargo clean`. |
| `make heartbeat` | Release heartbeat — is shipped work piling up unreleased? (see below). |
| `make check-backends` | End-to-end probe of all four backends (real tiny request; needs logins). Maintainer-only. |

**Release heartbeat — run `make heartbeat` at the start of a session.** It reports
the `CHANGELOG.md [Unreleased]` entry count and days since the last `vX.Y.Z` tag;
if it prints *"a release looks due"*, surface that to the user before other work.
This is the reminder that backstops the release policy in `RELEASING.md`
(milestone-driven + a ~monthly heartbeat, no fixed train) — there's no CI cron, so
this session-start check is what keeps releases from drifting.

## Formatting is nightly-only

`rustfmt.toml` uses nightly-only options, so **formatting must go through the
nightly toolchain** (`make fmt` → `cargo +nightly fmt`). Do **not** run plain
`cargo fmt` (stable): it can't apply the config and reformats by different rules,
producing a large bogus diff. The tree also has some pre-existing rustfmt drift,
so don't reformat unrelated files as part of a change — keep your diff to what
you touched.

## Code style (Rust)

These are `alix`'s house idioms — the things clippy and rustfmt *don't* catch and
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
- **Break freely while pre-1.0.** While the version is `0.x.y`, don't add
  back-compat shims or aliases for renamed or removed commands, flags, config
  keys, or directives — change them outright and record it as a **Breaking** note
  under `## [Unreleased]` → Changed. Compatibility machinery only earns its keep
  after 1.0; before then it's just surface to carry.
- **Pre-1.0 store format: no versioning, no migrations — just break it.**
  `CURRENT_VERSION` stays at `1`; never bump it and never add a `migrate` step. When the
  persisted store/deck shape changes, change it outright and let old data break
  (`#[serde(default)]` on new fields is a fine soft break; progress that happens to survive
  is a bonus, not a goal). The version fence and migrations are a *post-1.0* concern; if the
  version is ever `>1`, collapse it back to `1`. Author's standing instruction — **do not
  propose a store version bump pre-1.0**, and don't re-raise it.
- **No new dependency without a one-line reason.** Each crate added is permanent
  maintenance and supply-chain surface — reach for std or an existing dep first,
  and when a new one genuinely earns its place, say why in the commit.
- **Don't break card identity.** A card's id is
  `XxHash64(deck file name + its back lines)` — or, for **cloze** cards, its
  `hash_lines`: each line's text with the `{{ }}` delimiters stripped (so
  restyling the markup doesn't reshuffle ids), plus a per-hole index — see
  `Card::id` (`src/card.rs`). It deliberately ignores the front, notes, and
  comments, so editing those preserves a card's review history while changing a
  back line resets it. Preserve this whenever you touch the parser,
  `hash_lines`, or deck rewriting — a careless change silently wipes users'
  progress.
- Roadmap and design rationale live in `ROADMAP.md` (gitignored, local) — the raw
  idea dump, items tagged `* [ ] -- <P0–P3|--> - (Category) <text>` (`--`/`[x]` =
  done). `PLAN.md` (also gitignored, local) is the **plan at a glance**: a
  roadmap-vs-code audit (which items are secretly already shipped), the assigned
  priorities, and a **mermaid gantt** of the actionable P0–P2 work. The roadmap
  drifts from the code — items get built without being struck — so re-verify any
  "open" item against the code before treating it as todo (recent audit found
  ~7 already shipped); keep both files in sync when you finish or reprioritize work.
  **Strike shipped items in the same session the work merges** — flip `[ ]`→`[x]`,
  priority →`--`, and append `— SHIPPED (<commit>, <date>): <one line>` (match the
  existing struck items' shape; sweep the launch-checklist duplicates too). This
  is the step that historically gets forgotten — treat "merged but not struck"
  as an unfinished task, same as a failing test.
- **SDD specs and plans live locally in `docs/specs/` and `docs/plans/`** (gitignored,
  date-named, e.g. `docs/specs/<date>-<topic>-spec.md`). Before building a spec'd feature,
  read its spec/plan there; the **memory index** and `PLAN.md` track what's currently in
  flight — these design docs aren't in the repo, so a fresh session won't see them otherwise.
- **Subagent-driven development cleans up after itself.** When an SDD run
  finishes — its branch merged or abandoned — delete its scratch: the
  `.superpowers/sdd/` ledger and any task-brief / review-package files, so a
  spent ledger doesn't linger in the working tree and get mistaken for active
  work. The ledger is a live recovery map *during* a run; once the work lands
  it has done its job.
