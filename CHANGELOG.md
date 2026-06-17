# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
- `flash reset` clears stored progress: for one or more decks, a single card
  (`--card <id-or-front-text>`), or everything (`--all`). Confirms first unless
  `-y`/`--yes`, and refuses to act without a terminal rather than wiping
  silently.

### Changed
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
