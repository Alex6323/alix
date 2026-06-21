# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **`flash explore --title` / `--max-stage` shape the scaffolded workspace; the
  goal becomes its description.** `flash explore --into <dir>` now takes an
  optional `--title` for the workspace's `flash.toml` `title` (omitted, the
  folder name is used) and `--max-stage <1–5>` to set a shared `[defaults]`
  `max-stage` cap for every member deck. It also writes the `--goal` as a new
  `flash.toml` **`description`** field instead of an ignored `goal` key; a
  workspace's `description` shows **dim under its row** in both pickers (terminal
  and web).
- **Confirm before abandoning a review; commit the picker filter with Esc**
  (terminal) — quitting a review **mid-session** now asks to confirm (`Enter`
  leaves, any other key stays), so a stray `Esc` no longer drops a queued session;
  a finished session or a hard `Ctrl-C` still leaves at once (matching the web
  frontend). In the picker, `Esc` in the filter box now **keeps the filter** and
  drops to the list focused on the first match (a second `Esc` clears it), instead
  of discarding what you typed.
- **Picker disables decks with nothing to review** (terminal) — a deck with no
  card due right now (fully drilled, or all on cooldown) is dimmed and badged
  with a 🕒 clock, mirroring how a 🔒 locked (`% requires:`) deck looks, and
  `Enter` on it is a **no-op** — no more starting an empty session that bounces
  you out to a "Nothing to review right now" message. Such decks also can't be
  ticked into a merged review, and (in a workspace drill-in) sink below the
  startable ones. `--cram`, which ignores cooldowns, turns the gating off; browse
  never gates (any deck is browsable). New lib helper `session::has_reviewable`.
- **Reworked deck picker + trace walking from the picker** (terminal) — the
  no-argument picker is a clean, **single-launch** list (no checkboxes): `Enter`
  opens the focused row. Its header is just `flash`; rows are grouped into
  **Workspaces** (each showing when it last made progress, from its own store) ·
  **Recent** (loose decks you reviewed lately) · **Folders**, a blank line between
  sections, with the filter searching *every* loose deck. A deck that lives inside
  a workspace is kept out of Recent — you reach it by opening its workspace. Rows
  that share a title (two workspaces named the same) get a path hint to tell them
  apart; over-long rows (a trace's `% trace:` sentence) are truncated with `…`.
  Rows you can't start now are dimmed and `Enter` is a no-op: 🔒 locked
  (`% requires:` unfinished), 🕒 nothing due (on cooldown); a mastered deck reads
  `mastered 🎉`. The focus is on the **list** by default with Vim-style keys,
  rebindable in a new `[picker]` config section (`j`/`k` or arrows move, `l`/`Enter`
  open, `h`/`Esc`/`Backspace` back, `m` opens the Mastered window, `/` or `Ctrl-F`
  filters); jumping to the first/last row is fixed at `g`/`G` (or Home/End), like
  the `[browse]` pager. Mastered/done and locked decks are kept out of Recent (a quick
  launchpad); **`m` opens a dedicated Mastered window** of the exam-passed decks,
  or the filter reaches them. Long `% title:` / `% trace:` labels are capped so
  rows stay short. The picker and the review/walk/exam it launches now share **one
  terminal**: opening a deck and returning to the workspace no longer tears the
  TUI down and reopens it.
  Opening a **workspace** or **folder** drills into its members drawn as an
  **unlock dependency tree** — a deck nests under the `% requires:` prerequisite
  that gates it, foundations at the roots, siblings startable-first, each badged
  `· trace ·` / `· deck ·`. Opening a workspace, stepping back (`Esc`/`Backspace`),
  and **returning after a review/walk/exam** all happen within **one live screen**
  — no TUI teardown/reopen — so you can study a deck and **land back in the
  picker** (the workspace you came from, or the top list) to pick the next; only
  an `Esc` at the picker itself quits. The big gap it closes: a **trace** opened from the picker now
  **walks** (predict → reveal) instead of being flattened into a card review —
  both in the top-level drill-in and `flash workspace <dir>`. An explicit
  `flash review <trace.txt>` still flattens it (honoring the literal command). The
  multi-select machinery is retained in the code but unused for now. The web picker
  follows in a later phase.
- **Per-workspace progress store** — a deck inside a workspace (a folder with a
  `flash.toml`) now tracks its progress in a **`progress.json` inside that
  workspace**, not the one global `~/.local/share/flash/progress.json`. So a
  workspace is a self-contained, portable unit (decks + `assets/` snapshots +
  progress in one folder), its history is isolated, and same-named decks in
  different workspaces no longer collide in one store. Loose decks (and plain
  folders without a manifest) keep the global store; `--store <path>` overrides
  either; a workspace can redirect its store with a `store = "..."` line in the
  `flash.toml`. Resolution: `--store` > the single workspace all the session's
  decks share > global. Applies across the CLI/TUI (`review`, `trace`, `exam`,
  `browse`, `stats`/`list`, `reset`, `flash workspace`); the web frontend follows
  with the picker revamp. (No migration — workspace decks start fresh in the
  workspace store; existing global progress for them is left in place.)
- **Trace source snapshots** — creating a workspace by exploring a source
  (`flash explore --into <dir> --build`) now **freezes the cited excerpts** into
  the workspace as its final step: for each checkpoint it writes a small snippet
  file into the workspace's `assets/` folder (`assets/01.rs`, `02.rs`, …) holding
  just the lines that checkpoint reveals, and repoints the `% at:` (and the
  trace's `% source:`) at it. This stops the line-number locators from drifting
  when the upstream source is later edited (the walk reads the source live, so a
  moved line silently shows the wrong excerpt), and makes the workspace
  self-contained — **without copying whole (possibly huge) source files**. A
  re-based snippet loses its original line numbers, so when those matter the
  original `file:lines` is preserved in the card's note (`! from
  scheduler.rs:90-98`). The source is plain text (any file, any topic — no
  version-control assumption). It's automatic for explored workspaces, not a
  command; a loose trace over a live `% source:` is left as-is. Rationale in
  `docs/traces.md`.
- **`flash import <file.tsv>`** — import an Anki "Notes in Plain Text" export
  (tab-separated `front<TAB>back`) into a flash deck, no Claude needed. It skips
  Anki's `#`-prefixed header lines, turns `<br>` tags into separate answer
  lines, decodes the common HTML entities, and backslash-escapes a back line
  that would otherwise read as a `%` comment or `!` note; rows missing a side
  are dropped. The result is validated and written to `~/decks/` (`-o`/`--print`/
  `--force`, like `flash deck`). Conversion lives in the lib
  (`import::tsv_to_deck`).
- **`flash deck <source>`** (renamed from `flash generate`, which no longer
  exists as an alias) — generates a facts deck with Claude from a **web page URL or a
  local file/directory path**, mirroring `flash trace`. A URL is fetched with
  WebFetch and the deck starts with a `% link:`; a local source is explored
  read-only with `Read`/`Glob`/`Grep` at its root and the deck starts with a
  `% source:` (so `flash exam` can grade against it). This gives a facts-deck stub
  from `flash explore --into` a manual fill path (point `flash deck` at its
  `% source:`).
- **Traces (`flash trace`, experimental)** — a guided predict-and-verify walk
  along a *path* through a `% source:`, drilling the connections between facts
  (the edges) rather than isolated facts. A trace deck declares a `% trace:`
  (a path description that marks it a trace) and a sequence of `explain`-style checkpoint cards,
  each with a `% at:` locator (`file:lines`, or just `lines` for a single-file
  source) into the real source, and optional `% given:` lines that name
  off-screen symbols the question leans on (shown as a list under the question,
  so a tight excerpt doesn't orphan the names it uses). Walking it goes hop by hop: you **predict**
  before anything reveals, the real excerpt is **read live** from the source and
  shown with the key points, you self-judge the **gap** (Got / Partial / Missed
  — a weak edge resets so it resurfaces sooner, via the normal per-checkpoint
  SRS), and after the last hop you **compress** the whole path into two
  sentences. Self-judged and offline (no model call) by default; **`flash trace
  --grade`** instead has Claude judge each typed prediction against the key points
  and return the verdict + one line of feedback (a model call per hop, run at the
  lightweight `[ask]` tier — not the heavy build defaults below). **`flash
  trace <deck> --serve`** walks it in the **web frontend** (the same
  frontend-agnostic `Walk` state machine the terminal uses): a left **path rail**
  whose nodes color in by Got / Partial / Missed, each checkpoint's source shown
  in a line-numbered excerpt, and `--serve --grade` running the live grade on a
  background thread while the page polls; `--port`/`--lan` work as in `review`.
  `flash trace <deck> --map`
  prints the path without quizzing; the generic AI exam refuses a trace (its
  verification is the walk itself). See `examples/keypress-to-grade.txt`.
  **`flash trace --build <deck>`** discovers the path for you: declare just the
  `% trace:` and `% source:`, and Claude explores the source (read-only
  `Read`/`Glob`/`Grep`, with the source root as its working directory — no write
  or shell access), traces the single load-bearing path, and writes the
  checkpoints back into the deck. The build prompt encodes the chain rules from
  `docs/traces.md`, so generated traces are paths, not quizzes. Configurable via
  a new `[trace]` section (model, effort, timeout, extra guidance) — which,
  unlike the other AI features, **defaults to a strong model (`opus`) and high
  effort (`--effort high`)** because building is one-shot, correctness-critical
  and fails silently on a weak model. A new `effort` knob also exists on `[ask]`
  (off by default) and is plumbed through to the CLI's `--effort` flag.
  **`flash trace --suggest <source>`** recons a source (read-only, one pass) and
  prints a ranked menu of candidate traces to author — a path-question, a spine
  sketch, and a suggested scope each, no checkpoints — closing the "what's worth
  tracing?" gap before `--build`.
- **`flash explore <source>` (experimental)** — goal-driven exploration:
  prints an ordered **learning plan** toward a `--goal` (default
  "understand the whole source"), the fact **decks** and **traces** worth
  authoring. Each item is tagged `[trace]`/`[deck]` (chosen by shape — edges
  become traces, node-shaped fact tables become decks), carries its `% requires:`
  prerequisites (the list is a valid topological order, foundations first), and a
  `% source:` scope. The goal scopes coverage — a broad goal spans every
  subsystem, a narrow one collapses to its slice (and traces it deeper). By
  default read-only (prints the plan); **`--into <dir>`** materializes it into a
  **workspace** folder — a `flash.toml` (the goal) plus a stub deck/trace file per
  item, `% requires:`-wired in dependency order with absolute `% source:` paths,
  ready to `flash trace --build` / author (refuses a non-empty dir unless
  `--force`). Add **`--build`** to fill them: `flash explore … --into <dir>
  --build` explores the source **once**, then resumes that same CLI session to
  write the full content of every item — predict-verify checkpoints for traces,
  fact cards for decks — so the workspace is review-ready in one command, with the
  items coherent (written from one understanding) and facts decks filled too.
  **`--walk`** instead builds an **explore walk** — a predict-verify
  trace over the source's *shape* (what it is → its domain nouns → entry point →
  spine → the first paths worth tracing), each hop revealing real structural
  evidence (the manifest, the module list, the entry enum). It's written to a file
  (`-o`, default `explore.txt`) and walked immediately, reusing the `flash trace`
  walk; re-walk later with `flash trace <file>`.
- **Workspaces** — a folder of decks reviewed together with shared directives.
  A folder is a **workspace** when it has a `flash.toml` manifest (a scoped
  `config.toml`) setting a `title` and a `[defaults]` table of directives that
  fill in what each deck leaves unset (precedence CLI > card > deck > workspace >
  default); a folder of decks *without* a manifest is a plain **folder** — still
  reviewable, but not a workspace. Both appear as their own rows in the picker
  (terminal and web, labeled "workspace" vs "folder") and drill into their decks
  (review all, or tick a subset); `flash review`/`browse <folder>` reviews the
  whole cluster. **`flash workspace <dir>`** opens a workspace into its own picker
  and routes each member to the right thing — a **facts deck** → review, a **trace
  deck** → predict-verify walk — returning to the picker when done. Great for
  clusters like a vocabulary set that should all be `direction = "both"` without
  repeating it per file.
- `% title:` deck directive (also usable in a `workspace.flash` manifest): a
  display name shown in the picker, session header, `flash list` and `flash stats`
  instead of the file name. Display-only and never part of card identity.
- **`flash exam <deck>`** — the AI exam, which *verifies understanding* and
  gates progression (rung 3 of the AI-exam direction). A deck declares its
  ground truth with `% source: <url-or-file>` (repeatable); the exam asks Claude
  for fresh open questions generated **from that source** (never from the cards,
  which would be circular), reads your typed answers, and grades them
  Pass/Partial/Fail against per-question rubric points. Passing marks the deck
  **mastered**, which is what now unlocks dependent decks — drilling a `% source:`
  deck to the top stage leaves it *exam due* (a new deck state, shown in the
  picker and `flash stats`) rather than finished; source-less decks keep the
  mechanical "finished = drilled" unlock. On a fail, the missed concepts can be
  turned into remediation cards appended to the deck — the card type is chosen
  per gap (cloze/plain for a missed fact, `% mode: explain` for a missed
  concept), and overlapping gaps are consolidated into a single card — then
  re-drill, re-sit. **Grading strictness is per deck** —
  `% strictness: strict | balanced | lenient` (or `flash exam --strictness`, or
  the `[exam]` default) — because some material needs every point recalled while
  other is about grasping the idea: `strict` treats an omitted rubric point as a
  gap, `balanced` (default) judges understanding and forgives terse phrasing,
  `lenient` only flags clearly wrong answers (orthogonal to `pass_threshold`,
  which sets how many answers must pass). New `[exam]` config section (`model`,
  `timeout_secs`, `num_questions`, `pass_threshold`, `strictness`, `extra`);
  reuses the `[ask]` command/permission/tools (WebFetch reads a source URL).
  `flash reset` of a deck also clears its mastered state. A URL `% source:` also
  doubles as an ask-Claude reference link (no duplicate `% link:` needed); a
  `% link:` never becomes an exam source.
  The exam is **fully interactive in both frontends** (rung 3b): answer one
  question at a time (Back/Next), then see a per-question breakdown — `flash exam`
  and `flash serve` share one engine (`exam::Sitting`) that runs Claude on a
  background thread and polls, so neither blocks. You reach it by **picking an
  `exam due` deck** (it launches the exam instead of an empty review) or from the
  **session-end summary** when a deck you were drilling just became exam-due.
  Exam-due decks aren't tickable into a merged review (they have no due cards).
- `% mode: explain` — **understanding cards**. The front is an open prompt and
  the back lines are the *key points* a good answer should cover (not a string to
  reproduce). You optionally type your explanation, reveal the points, and
  self-grade (Again/Good/Easy) on whether you covered them — for cards aimed at
  understanding over recall. The typing is optional and unchecked (a self-graded
  mode can't verify it); the web shows your answer beside the points. Works in
  both frontends and pairs with ask-Claude. (Daily tier of the planned AI exam.)
- Ask-Claude in the **web frontend** (`--serve`): an "Ask" button / the `?` key
  on an answered card opens a chat panel (Send / Save note / Close), mirroring
  the TUI feature. The server runs `claude -p` on a background thread and the
  page polls for the reply, so the single-threaded server stays responsive; one
  conversation spans the session (`--session-id`/`--resume`), and Save note
  appends a condensed note to the deck file. Reachable wherever you serve,
  including `--lan`.
- `% max-stage: N` deck directive (1–5, default 5): the deck's top Leitner
  stage. A card that reaches it **retires** — it rests and is no longer
  scheduled (not even under `--cram`) until `flash reset` — so material you only
  need a couple of times (e.g. code review) drops out instead of recurring
  forever. `% max-stage: 1` = "get it right once and it's done." A deck is
  *finished* once all its cards retire. `flash list` shows retired cards as
  `resting`, and stage histograms (picker, summary, `flash stats`) render
  unreachable stages above the cap as a dim `–` instead of `0`.
- Deck completion states and unlocks. Each deck has a state derived from its
  cards' stages — not started / started / finished (all cards at the top stage)
  — shown in the deck picker (terminal and web) and `flash stats`. A deck is
  **locked** while any of its `% requires:` prerequisites isn't finished
  (finishing a foundation unlocks what builds on it); locked decks are dimmed
  with a 🔒 but stay selectable (advisory). Derived live from progress, with no
  new directive or storage.
- Repeated `TAB` in typing mode progressively reveals the answer: each press
  uncovers two more characters until the line is fully shown (still counts the
  card as failed); typing or deleting resets the reveal.
- In-browser deck selection: `flash --serve` (and `flash browse --serve`) with no
  deck files now opens a deck-selection screen in the browser instead of the
  terminal picker — a checklist of the same decks (recent first), with a Start
  button that builds the session in place. Passing decks on the CLI still skips
  it. A running web session can return to the picker via "Choose other decks"
  (on the summary or the menu) to study a different deck without restarting.
  Selection only accepts deck names from the live catalog, so no path is built
  from request input.
- Mark a card for removal during review or browse with the new `remove` key
  (default `Ctrl-X` in review, `x` in browse, `x`/Remove button in the web UI).
  It is dropped without being asked again (cloze siblings too); the marked
  cards are deleted from their deck files and their progress is pruned — at
  the end of the session in the TUI, immediately in the web UI (which has no
  end-of-session).
- Local web frontend: add `--serve` to `review` or `browse` to run it in the
  browser instead of the terminal, reusing the same session logic and writing
  to the same progress store, so browser and CLI share one history. All answer
  modes work (flip, line-by-line, typing, fuzzy, multiple choice); controls are
  touch-friendly and mirror the configured `[keys]` bindings. Binds to localhost
  by default; `--lan` exposes it to the network (no auth), `--port`/`[serve]`
  set the port (both `--port` and `--lan` require `--serve`). Built on
  `tiny_http`.
- Per-card answer mode: a `% mode:` directive placed after a card's front
  overrides the deck's `% mode:` for that card only, so one deck can mix modes
  (e.g. a `line` lyrics card among `flip` cards). Cloze sub-cards inherit their
  source card's mode.
- A mode badge at the top of the answer section on every card, in both the TUI
  and the web app — `flip`, `typing exact`, `typing fuzzy`, `choice`,
  `line by line` — so typing vs fuzzy (otherwise identical input prompts) is
  clear at a glance.
- Dual-direction cards: a `% direction:` directive (per card or deck-wide,
  `forward`/`reverse`/`both`) reviews a card both ways — `both` generates the
  card and its swap (e.g. `purported → angeblich` and back). The two get
  distinct progress, are kept apart in the queue, and are removed together;
  cloze cards are unaffected.
- Image cards: `% img:` (question side) and `% img-back:` (answer side, revealed
  with the back) attach an image to a card; a deck-level `% img-dir:` sets the
  folder filenames resolve against (else the deck file's folder; absolute card
  paths are used as-is). Images render in the web frontend only, so an image card
  is automatically web-only — the TUI skips such cards (and refuses, pointing at
  `--serve`, if a whole deck is web-only). A general `% frontend: any|tui|web`
  directive (per card or deck-wide) controls this explicitly; `flash check` warns
  about missing image files. `/img/<key>` URLs are opaque hashes of registered
  deck paths, so the server never joins request input to a filesystem path.
- `flash reset` clears stored progress: for one or more decks, a single card
  (`--card <id-or-front-text>`), or everything (`--all`). With no decks it opens
  the same checkbox picker as `review`/`browse` to choose them; `--cards` opens
  a picker over a deck's cards (those with progress). Confirms first unless
  `-y`/`--yes`, and refuses to act without a terminal rather than wiping
  silently.

### Changed
- **You can sit the AI exam early to test out of the drilling.** The exam no
  longer requires every card drilled to the top stage first — it's available as
  soon as a deck has a `% source:` and its `% requires:` are satisfied (drilled
  or not). Passing it **masters** the deck regardless of card progress, which
  **unlocks its dependents** — so a learner who already knows a topic isn't
  forced to grind its cards. Exams still flow in dependency order: a **locked**
  deck stays un-examable until its prerequisites are mastered (pass *their* exams
  first). In the browser picker, a focused examable deck gets a **"Take exam"**
  button (and the `x` key); `flash exam <deck>` does the same from the terminal.
- **The web deck-selection screen now mirrors the terminal picker.** It is
  **single-launch** (no checkboxes): click a deck to start it, or open a
  **Workspace** / **Folder** to drill into its **unlock dependency tree** (each
  deck nested under the prerequisite that gates it). Rows are grouped into
  **Workspaces** (each with its last-progress time) · **Recent** loose decks ·
  **Folders**, and the filter searches *every* loose deck. A deck you can't start
  is dimmed — 🔒 locked (`% requires:`), 🕒 nothing due — and mastered/done/locked
  decks are kept out of Recent, with a `mastered 🎉` deck tucked into a **Mastered
  window** (`m`); navigation honors the `[picker]` config keys (served to the page
  at `/api/picker-keys`). A **locked** deck can no longer be *started* for review
  (was advisory), but stays fully browsable (`flash browse` ignores locking) and
  resettable; the `flash reset` / `flash deps` pickers keep their plain
  multi-select. The shared badge / lock / dependency-tree logic now lives in the
  library (`picker::deck_status` and the exposed dependency-forest helpers),
  consumed by both frontends. A **trace** picked from the in-browser picker now
  **walks** (predict → verify, just like the terminal), hosted by the review
  server at `/walk`; a **Back to decks** (or `Esc`) returns to the picker.
- **A card that reaches the top Leitner stage now retires** (rests, no longer
  scheduled until `flash reset`) instead of recurring at the stage-5 weekly
  cooldown. This is the default even without `% max-stage:`, and makes a
  *finished* deck stay finished.
- The TUI's remaining-card count moved from the header to the bottom-right of
  the footer, shown as `N↓` after the pass/fail tally — matching the web
  frontend's score line (the header now carries only the stage histogram).
- Typing mode grades multi-line answers **order-independently**: a card whose
  answer is several items can be typed in any order, each completed line matched
  to whichever expected line it best fits (TUI and web). Single-line answers are
  unchanged.
- Typing feedback now keeps the typed text on screen and, on a wrong line, shows
  the correct answer underneath with a check mark (the TUI previously discarded
  the input and repainted only the answer; the web already did this).
- "New session" on the summary is disabled when nothing is due: the TUI omits
  the hint and makes the key inert, the web disables the button and shows a
  "nothing due" note — instead of only reacting after the key is pressed.
- **Breaking — cloze hole syntax is now `{{ }}`** (was `{ }`). A lone `{` or `}`
  is literal inside `#?` cards, so code with braces needs no escaping. Cloze
  identity is now hashed from the parsed structure (delimiters removed) rather
  than the raw braced text, so existing cloze cards' progress is reset once — but
  future markup changes won't cost progress again. Existing `#?` decks must be
  rewritten `{x}` → `{{x}}` or they fail to load (they'd have no holes).
- Note rendering moved into a frontend-independent `render` module that emits a
  structured model (`NoteUnit`: sentence-split prose or verbatim code blocks);
  the TUI now only paints it. No change to how notes look — this lets a future
  frontend reuse the same note structuring instead of reimplementing it.
- The answer mode is now resolved per card instead of once per session:
  CLI `--mode` > the card's `% mode:` > the deck's `% mode:` > the built-in
  default. `--mode` still forces every card.
- Deck-level directives (`% mode/order/scheduler`) must now sit in the deck
  header, before the first card; a `% key: value` after a card front is treated
  as a per-card override.
- `flash check` no longer fails on warnings: duplicate-answer warnings are
  advisory, so it exits non-zero only when a deck won't parse, and prints a
  `N error(s), M warning(s)` summary.
- Web review now shows the expected answer whenever a typed line differed —
  including a fuzzy pass within tolerance — matching the TUI, so typos aren't
  reinforced.

### Fixed
- **You can start the AI exam on a deck inside a workspace from the browser.**
  `POST /api/exam/start` only resolved top-level deck names, so the "Take exam"
  action silently failed (a 400) for a workspace **member**; it now resolves
  members by their qualified `<workspace>/<file>` name too, like `/api/select`.
- **The web ask-Claude panel now shows only the current card's exchanges.** It
  was rendering the whole session's conversation, so every former card's Q&A
  piled up on screen. The display is now scoped to the card you're on (and so is
  the "save note" condense), while the CLI conversation still spans the session —
  Claude keeps the full context. The card's front + answer are pinned just above
  the input, for easy reference while you type a question. (The terminal ask view
  already scoped per card.)
- **The TUI reflows immediately on a terminal resize.** The event loops redrew
  from a size query that could be momentarily stale right after a resize, so the
  screen sometimes stayed unchanged until the next keypress refreshed it. They now
  resize with the dimensions the resize event itself carries — picker, review,
  exam, and browse.

## [0.1.0] - 2026-06-16

First release of `flash`: a terminal spaced-repetition flashcard trainer with
a ratatui TUI, plain-text decks, two schedulers, several answer modes, cloze
cards, deck dependencies, an ask-Claude helper, and AI deck generation.

### Deck format
- Plain text: `#` card front at column 0, indented answer lines, `! ` notes
  (multiple `!` lines form one multi-line note), `% ` comments, `\` to escape
  a leading markup character. Indented `#` lines are answer content, not new
  cards.
- **Cloze cards**: a `#?` front with `{holes}` in the answer expands into one
  sub-card per hole; sibling holes are masked and spaced apart in the queue.
- **Directives** (`% key: value`): `mode`, `order`, `scheduler` set per-deck
  defaults; read from the requested deck(s) only, overridden by CLI flags.
- **Dependencies** (`% requires: <deck>`): prerequisite decks are pulled in
  transitively and ordered foundations-first; cycles and missing prerequisites
  are reported. Prerequisites contribute cards only, not directives.
- **Reference links** (`% link: <url>`) are offered to the ask-Claude feature.

### Review
- Answer modes: **flip** (default, self-graded), **typing** (char-by-char),
  **fuzzy** (whole-line, typo-tolerant), **choice** (multiple choice with
  distractors sampled from the session), and **line** (reveal the back one
  line at a time — for lyrics, poems, ordered lists).
- Schedulers: **Leitner** (the original 6-stage boxes, compatibility-verified)
  and **SM-2** (per-card ease factors), interchangeable.
- Session controls: `--new` (new-card cap), `--limit`, `--cram`,
  `--order sequential`, restart from the summary screen, failed cards requeued
  within the run.
- Notes render as a quoted block, split into sentences, with fenced code shown
  verbatim (indentation preserved).

### Ask Claude
- Press `?` on an answered card to ask the Claude Code CLI about it without
  leaving the session; one conversation spans the run. `Ctrl-N` condenses the
  exchange into note lines appended to the deck. The input line supports cursor
  movement and editing. Runs headless with a safe permission model (`dontAsk`
  + an exclusive `WebFetch`/`WebSearch` allowlist).

### AI deck generation
- `flash generate <url>` builds a deck from a web page via Claude (WebFetch),
  with a prompt that spreads cards across four layers of understanding, uses
  cloze and notes, and self-reviews for redundancy; `--review` adds a second
  refinement pass. Output is validated and saved (or `--print`ed); `--cards`
  and `[generate]` config tune it. Claude only returns text — flash writes the
  file — so no extra tool permissions are needed.

### Other commands
- `flash browse` — read-only walk through cards (no grading, no writes).
- `flash deps` (alias `require`) — edit a deck's prerequisites with a checkbox
  picker.
- `flash stats`, `flash list`, `flash check`, `flash config`.
- Startup **deck picker** (recent decks + the decks directory) when run with no
  arguments.

### Configuration
- `~/.config/flash/config.toml` with `[keys]`, `[browse]`, `[ask]`,
  `[generate]` sections and `decks_dir`. `flash config --init` writes a
  self-documenting template (every option commented at its default);
  `flash config` prints the active settings. Key bindings are rebindable.

### Storage
- Card identity is a stable XxHash64 over the deck file name plus the back
  lines (a test pins the value so upgrades never orphan progress). Progress is
  stored at `~/.local/share/flash/progress.json`, created on first use.

### Desktop
- `assets/install-desktop.sh` installs an icon, launcher, and `.desktop` entry.
