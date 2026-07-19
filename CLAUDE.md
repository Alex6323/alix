# `alix` — project guide

`alix` is an **AI-augmented** spaced-repetition learning tool in Rust,
web-first (bare `alix` opens its web frontend). On top of a plain-text
flashcard core, an AI backend is woven in: an in-session tutor on any card, AI
deck/workspace generation (`alix generate`), and the **AI exam** that gates
progression on verified understanding. The AI backend is pluggable (`[ask]
backend` — Claude by default; Gemini, Codex, and Copilot also supported). The
tool is increasingly AI-centric — weight that when prioritizing. The **library
crate is the single source of logic**; the web server and the CLI are
thin consumers. Put behavior in the lib (`src/`), not in the frontend, so both
share it.

Native mobile thin clients (Android/iOS) are anticipated, consuming these same
web endpoints — so treat the web JSON API as a **client-agnostic contract**:
review flow and session state live in the lib behind presentation-agnostic
endpoints, never in page JS (a native client can't reuse logic trapped in the
page). The contract is written down in `docs/API.md`, pinned by the
`mod contract` snapshot tests in `src/serve/contract.rs` (which also emit the
`tests/contracts/*.json` codegen corpus) — change code, doc, and CHANGELOG
together. alix itself stays a plain bind-to-interface HTTP server — reaching it
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
  same one job — install and onboarding, the web surface, backends,
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

**UI noise is a gate too — clean, no-distraction surfaces.** The web frontend
must stay calm: only what the user needs *right now*, nothing competing
for attention. Every pixel/char earns its place — chrome that's always on but
rarely useful (status ladders, persistent counters, decorative readouts) is
noise, not information. When you add UI, the default is *less*: prefer one
primary action, tuck secondary/rare controls behind a menu, and cut a readout
before adding one. A long or hand-crafted label **truncates** (ellipsis) — it
never wraps or reflows the layout; headers and bars hold a fixed size regardless
of content. When in doubt, remove it; if it's genuinely useful but rare, hide it
behind a `⋮`/`m`/`?`-style affordance rather than leaving it on screen. Treat a
noisy diff the way you'd treat a failing test: not done yet.

**Funding tone: subtle hints only, never begging.** Sponsoring the development
is mentioned quietly where it fits (a Support link), and the copy always leads
with the free alternative (telling someone about alix). Never print concrete
cost figures (they go stale and are never quite true). No banners, no nags, no
guilt. In-app, exactly one quiet Support line lives in the About dialog (free
alternative first, then the sponsors link; user decision 2026-07-17), never on
a study surface. If a sentence reads like an ask for money, cut it.

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
| `make calibrate` | Real-Claude grader calibration (`tests/calibrate.rs`, costed): before touching `grade_*`. |
| `make run ARGS="stats mydeck.txt"` | Run the binary with args. |
| `make web ARGS="~/decks-test"` | Web frontend; no ARGS → the picker over the configured decks dir. |
| `make web-debug` | `web` + per-request stderr logging (the `{#server-subresource-stall}` net). |
| `make phone` / `make tablet` / `make desktop` | Run the alix mobile app on the phone/tablet emulator (boots the AVD if needed) or as a native Linux window (fastest loop). |
| `make frb-check` | Assert the frb toolchain-alignment invariants (version pins, template patches, NDK); fails on drift. |
| `make push-decks DIR=~/decks` | One-way copy of a host decks folder into the running emulator's app (dev-only; restart the app to re-list). |
| `make mobile-test` | Mobile suite vs the real core, no emulator: Dart unit/widget tests on the host dylib + the full-app integration test in a Linux window. |
| `make apk` | The arm64 release APK (debug-signed while `android/key.properties` is absent); smoke-install it before a `mobile-vX.Y.Z` tag (RELEASING.md). |
| `make book` | Serve the mdBook manual (`docs/book`), live reload. |
| `make site` | Preview the `alix.study` landing page locally (`site/`). |
| `make install` | `cargo install --path .`. |
| `make clean` | `cargo clean`. |
| `make heartbeat` | Release heartbeat — is shipped work piling up unreleased? (see below). |
| `make check-backends` | `alix doctor --all-backends` (real tiny request per backend; needs logins). Maintainer-only. |

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

- **Type names are spelled out.** No `Cfg`/`Mgr`/`Ctx`-style abbreviations on structs,
  enums, or traits — `AssembleConfig`, not `Cfg`. Established domain acronyms (`Dto`,
  `Api`, `Id`) and *variable/field* shorthand (`cfg`, `opts`) stay fine; an abbreviated
  type name needs a stated reason, not habit (user rule, 2026-07-11).

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

- **Comments: default is NO** (user rule, 2026-07-19; replaces the old prose-doc style).
  ONE trigger for the exception, as short as possible: the code cannot (easily) express it
  (a frozen literal's provenance, a deliberate exclusion that would otherwise read as a bug,
  a name/signature that would otherwise mislead). Public-ness raises care, grants nothing.
  NEVER narrate reasoning in code (reasoning lives in commit messages; decisions in the
  design docs). NEVER write a comment that can go stale easily; the test: does it restate a
  fact with a second source of truth (code, type, test) that can move independently? Never
  cite external or gitignored documents (no "spec X"/section numbers in code). Lint
  suppressions keep their one-line reason; guarded `.expect` keeps its invariant note.
  Reviewers flag unnecessary comments, never missing docs.

- **Tests live inline; integration tests in `tests/`.** Unit tests go in a
  `#[cfg(test)] mod tests` at the bottom of the module they cover. `tests/` holds
  the end-to-end suites: `tests/cli.rs` drives the built binary as a subprocess
  (deterministic — temp decks + `--store`, no real Claude — so it runs in CI),
  and `tests/calibrate.rs` is the `#[ignore]`d real-Claude grader-calibration harness
  (`make calibrate`). Name tests as full snake_case sentences stating condition +
  expectation (`passing_the_exam_masters_an_undrilled_deck`). Anything that shells
  out to Claude uses the shared harness in `src/testutil.rs`: `fake_reply` (drains
  stdin then emits a canned reply — use it for fixed outputs to avoid the EPIPE
  race), `fake_cli`, `ask_config`, and wrap the call in `exec_lock()` (serializes
  fork/exec; poison-tolerant).

- **Keep threading in the lib.** Background work follows the `ask::spawn` shape: a
  function that spawns a thread and returns a `Receiver`; the frontend only
  `try_recv`/poll. Don't spawn threads from the web server.

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

- **KISS: edge cases default to the roadmap, not the build** (user rule, 2026-07-19). Build
  the main flow. When design or review surfaces an edge case, the default disposition is a
  named roadmap item or registry residual, not inline handling. Inline handling must clear
  one of two bars: silence would corrupt data or break an invariant on a normal path, or the
  corner sits on a frozen surface (id grammar, file format) where silence today becomes
  permanent (decide those now, even if the decision is "reject"). Deferral is loud, never
  silent: keep the cheap detection (a doctor line, an error) where silence would mislead.
  Enumerate every edge case somewhere durable; build almost none of them. Reviewers flag
  inline edge-case machinery as over-engineering.
- **Performance is a core value; an inefficiency is never glossed over** (user rule,
  2026-07-19). This is a Rust project: high performance is expected, it is part of what a
  Rust tool represents, and marginal gains are still wanted. A known inefficiency (a
  full-file rewrite for an O(1) change, re-parsing unchanged data, per-item work a cache
  or batch removes) has exactly two dispositions: fixed now, or a NAMED roadmap item;
  silent acceptance is not one of them. "Fast enough" claims must survive the 10x scale
  counterfactual on real data, not fixtures; recurring offenders get an invariant-style
  regression test (counters and scaling ratios, never wall-clock thresholds).
- **Test-first for library logic.** Write the test before (or alongside) new
  `src/` behavior — above all the AI plumbing's error paths, where bugs hide and
  local runs can mask races (the `testutil` fake-CLI tests exist because one such
  race slipped through). Thin frontend glue (`serve` wiring) is the
  exception — a follow-up or manual check is enough there. This is the
  deterministic half of the QUALITY plan; the grader calibration
  (`make calibrate`) is the AI half, run deliberately before touching `grade_*`.
- **Tests and clippy must be green** before a change is done (`make check`).
  Formatting is run deliberately with `make fmt`, not enforced as a gate.
- Don't commit unless asked; never push without permission.
- **Two docs, two jobs; keep both in sync.** `docs/book/` (mdBook; `make book`)
  is the **reference and manual**: the deck format, every `%`/card directive,
  every flag, all features, chapter by chapter. It is updated on **every**
  user-facing change. `README.md` is the **landing page** for both GitHub *and
  crates.io* (pitch, install, quickstart, a small inline deck example, a
  top-level command table, a capability list, links). It is deliberately
  **self-contained** — a crates.io or offline reader must see what alix is
  without leaving the page — but **not** a reference. Sync test: the README
  changes only when a *slot* does — a **new or renamed top-level command** (the
  command table) or a **headline capability** (the capability list / deck
  example); finer detail (new directives, flags, per-feature behavior) is
  **book-only**. README book-links are **relative** (`docs/book/src/NN-*.md`) so
  they render on GitHub, survive offline, and version with the checkout; only the
  top Manual/Slides/Site banner uses hosted `alix.study` URLs (relative links do
  not resolve on crates.io, which is why the example + command table carry the
  substance there). Internal refactors touch neither doc.
- **User-facing changes get a `CHANGELOG.md` entry** under `## [Unreleased]`
  (Keep a Changelog format: Added / Changed / Fixed). Internal refactors and
  test-only changes don't.
- **Keep the living docs lean — this file, `CHANGELOG.md`, commit/PR text.** Condense each
  rule/change to its shortest form that still carries the information — cut filler, war-stories,
  and rationale-at-length (those go in the spec or memory), never the substance. If it can't be
  shorter without losing something relevant, long is fine. The measure is information-per-word,
  not line count.
- **Break freely while pre-1.0.** While the version is `0.x.y`, don't add
  back-compat shims or aliases for renamed or removed commands, flags, config
  keys, or directives — change them outright and record it as a **Breaking** note
  under `## [Unreleased]` → Changed. Compatibility machinery only earns its keep
  after 1.0; before then it's just surface to carry.
- **Pre-1.0 store format: no versioning, no migrations — break it.** `CURRENT_VERSION` stays
  `1`; never bump it or add a `migrate` step. Change the shape outright and let old data break
  (`#[serde(default)]` is a fine soft break). **Don't propose a store version bump pre-1.0.**
- **No new dependency without a one-line reason.** Each crate added is permanent
  maintenance and supply-chain surface — reach for std or an existing dep first,
  and when a new one genuinely earns its place, say why in the commit. The
  Cargo.toml comment states only what the dep is *for* (its purpose, one line);
  the justification against these rules goes in the commit message, never into
  the file.
- **Counterweight to the above: don't hand-roll a correctness-critical commodity.** A standard,
  well-specified algorithm (scheduler, standard-format parser, crypto) should come from a
  maintained crate — re-implementing it from a spec is the *higher*-risk choice. Separate a
  heavy *full package* (e.g. an ML optimizer) from its light *core* before rejecting a dep on
  weight.
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
- **When a newer spec overrules an older one, stamp the old spec** — a header blockquote,
  first thing in the file: `**SUPERSEDED <date> by \`<newer-spec-file>\`**` (or
  DEPRECATED/REPLACED-BY), one line on what still survives, and "do not build from this
  spec". Do it in the same session the newer spec lands — a fresh session must never
  build from a stale spec (e.g. the session-levels spec superseding the difficulty-ladder
  spec). Partial supersession names the surviving sections explicitly.
- **Devil's-advocate a spec before sign-off.** Before a design spec is approved/locked,
  run an explicit adversarial pass — argue *against* the design, hardest objections first,
  on the project's own gates (fit gate, north star, the NOT-list, soundness); then prosecute
  the rebuttals back (steelman the alternative). A spec isn't "locked" until it has survived
  one. Cheap, and it catches mis-founded designs before a build pays for them.
- **A doc for fresh readers gets a context-free read before it's called delivered** (user
  rule, 2026-07-19). Hand the doc ALONE to a fresh agent with one question: "list every
  term or claim you cannot resolve from this document alone"; fix what it lists. The known
  offender is session-born shorthand (spec codenames like "L1", `{#anchors}`, arc names):
  from inside the session it reads as the thing's name, so the writer cannot see it, and an
  "imagine a future reader" prose rule cannot trigger on what the writer cannot see. Only a
  reader who actually lacks the context detects it. Applies to `docs/bugs/`,
  `docs/product/`, `docs/results/`, the book, README; a codename grep is a cheap first pass,
  the fresh read is the gate.
- **An assumption is verified only when a command failed to refute it.** In specs/plans
  write `Assumption — falsification command · expected output · fallback`; never a bare
  "verify X". A step satisfiable by *reading* is not verification. Docs (including
  `docs/API.md`) are hypotheses about the code, not evidence of its behavior — only an
  execution closes an assumption. When a trace agrees with you, keep going: stop on
  contradiction, not on agreement. A second reader is a correlated sensor — independence
  means a different *method* (run it), not another pair of eyes. Weigh the asymmetry: the
  experiment costs seconds, being wrong costs the build. (Worked example: `docs/API.md`
  promised `DeckItemDto.name` was always selectable; two agents "verified" it by reading,
  and the kids client shipped a dead button. One `curl` returned 400.)
- **When blocked, stop — never build around the obstacle.** A wall the plan didn't anticipate
  (a cooldown, an unreachable fixture, an id you can't compute) means *report*, not improvise;
  a rule here won't survive one. Briefs must name the wall and the sanctioned route, or say
  "stop and ask".
- **A test must be able to fail, and must not sleep.** Mutation-test a new test: reintroduce
  the bug, watch it fail, restore. Never wait on wall-clock — wait on a condition, or record
  the gap as a skipped test carrying its reason. **Fixtures are content; state is generated:**
  never commit a progress store (a frozen timestamp is a time bomb), and never compute a
  `Card::id` outside the lib — a wrong id fails *silently*.
- **A fix must be shown to fix.** Reproduce the failure first, then confirm it disappears
  *because of* the change, not alongside it. A red build plus a plausible story is the most
  expensive thing you can believe.
- **A subagent inherits nothing.** Restate every binding constraint per brief; never assert
  what a file contains (tell it to read). Scope a dispatch to minutes, ask for a plan before
  it builds, keep expensive verification with the controller, and poll it —
  don't dispatch and block.
- **A rule that changes an `/api/*` response lives in the lib.** If the CLI hands it to
  the server as a closure, the server can't enforce it, the DTO can't express it, and
  `tests/api.rs` — which injects its own — can't test it. The tell: a test harness that
  must reimplement production behavior to boot a server.
- **Subagent-driven development cleans up after itself.** When an SDD run
  finishes — its branch merged or abandoned — clear its scratch with
  **`make sdd-clean`** (removes the `.superpowers/sdd/` ledger + task-brief /
  review-package files, keeps the dir + `.gitignore`), so a spent ledger doesn't
  linger in the working tree and get mistaken for active work. The ledger is a
  live recovery map *during* a run; once the work lands it has done its job.
