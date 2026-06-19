# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
- The startup deck picker (terminal and web) is now two-phase: `Enter` (or
  tapping a deck name in the browser) starts the **focused** deck immediately; to
  study several at once you tick decks (`Space` / checkbox) and **Confirm**
  (`Tab` in the TUI), which reviews the chosen decks and starts them as a merged
  session. A **locked** deck can no longer be *started* for review (was
  advisory) — but it stays fully browsable (`flash browse` ignores locking) and
  resettable. The `flash reset` / `flash deps` pickers keep their plain
  multi-select.
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
