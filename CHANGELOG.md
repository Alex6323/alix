# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- **A Recognize card with no buildable multiple-choice question no longer
  strands the review.** A deck too small for distractors (or without cached
  AI ones) reported the choice mode with no options to show, leaving the
  card with no way forward; it now falls back to a plain reveal-and-grade,
  so the reported mode is honest for every client.

### Added
- **Multi-device roaming guards in the store.** Every save stamps which
  device wrote `progress.json` (the device name is a plaintext file in the
  data dir, rename it freely), and the library can report a recent
  *foreign* write plus any Syncthing conflict copies sitting next to a
  store. The mobile app surfaces both as banners; the web/CLI surfaces are
  a follow-up. The rule stays one device at a time; these make a slip
  visible instead of silent.
- The library exposes its version as `alix::VERSION` (the mobile About
  screen shows it next to the app's own).
- **The mobile app grew up to real phones** (its own changelog:
  `apps/mobile/CHANGELOG.md`, released as `mobile-vX.Y.Z` tags with a
  signed APK on GitHub Releases): a user-chosen shared decks folder for
  the Syncthing roaming story, the web app's look (palettes, IBM Plex, the
  brand action), a launcher identity, and an About with both versions.
- The site gains an Impressum, a privacy note, a contact address and a
  sponsor link; personal details are injected at deploy time, not stored in
  the repo.
- **A "What's new" page on the site**: an interactive timeline of releases
  and landed changes (dots with popovers, drawn from this changelog and the
  git history at build time) over the full text record, plus a short teaser
  on the landing page, so it can't go stale on its own the way a
  hand-maintained page would.
- The legal notice and privacy pages are now in English (headings kept as
  "Legal notice (Impressum)" and "Privacy (Datenschutz)" for recognizability).
- The landing page counts visits with GoatCounter, a cookie-less,
  privacy-friendly analytics service; the privacy page explains what it does
  and does not collect.
- **The mobile app reviews Explain cards with the keypoint checklist.** After
  the reveal you tick which key points your reconstruction covered (the cached
  keypoints augment, else the card's own answer lines) and the tally derives
  the grade, exactly like the web. An optional collapsed "type it first" field
  lets you write your attempt before revealing; it never leaves the device.
- **The review header shows a dim "N left" count**: how many cards the session
  still holds, updated after every card. It can honestly tick up when a card
  you missed cools back in for its retry. (The card pile already hinted at
  this but flattens at 3, so a long backlog and a nearly-done session looked
  the same.)
- **The adult theme gallery's Kids group now offers Sunrise, Ocean, and
  Berry** — the same three themes the kids app ships, re-derived as full adult
  palettes (every token the picker needs, contrast-checked for the adult UI's
  denser text), so a kid transitioning to the grown-up app can keep the look
  they learned to love.
- **Tutor: make this a card.** In a review exchange, "Make this a card" asks the
  tutor to distill the conversation into a draft front/back; you edit it, then Add
  lands it as a free-standing card on the current deck, drilled like any other and
  promotable to the deck file. Adult review only; a non-parseable draft errors
  rather than inventing a card.
- Your decks folder is self-contained: drop it in a cloud drive (Dropbox, iCloud,
  Syncthing) for roaming multi-device (one device at a time), no accounts.
- **The Augment screen redesign: one card per target, not a row.** Each of the
  six targets (choices, notes, questions, key points, format, topology) shows
  a plain description of what it does and a small neutral before/after
  preview next to its coverage count and action. You can also tick several
  targets and press "Generate selected" to run them as one
  batch: a rough estimate of how many generations that will take shows up
  front, then each ticked card tracks its own status, queued, generating,
  done, or failed, as the batch runs, and one target failing doesn't stop the
  rest.
- **The Augment screen now opens on a workspace (or folder).** The same six
  target cards run across all member decks at once: Generate fills a target's
  gaps in every member, Remove clears it everywhere, and an Order generated
  here is one workspace-wide pedagogical path that a workspace review session
  picks up. A workspace also gets an **Icon** card that draws (or redraws) the
  emblem shown on its picker row, steered by the card's guidance input.
- **A Select all button on the Augment screen** ticks every target that can
  run, so a full batch is two clicks.
- **Every augment card carries its own guidance input.** Instead of one shared
  guidance box in the footer, each target card has a compact steer field with a
  kind-specific example as its placeholder (choices: "use common
  misconceptions", notes: "add a mnemonic", ...), so you can see per target
  what a steer is good for, and a batch sends each ticked card's own guidance.
- **`alix doctor --grading`: is your model good enough to grade exams?** An
  opt-in spot-check (three real, costed calls) that runs six hand-labeled
  grading probes against the configured backend: wrong, empty, off-topic, and
  incomplete answers that must not pass, and correct answers that should. It
  reports the two directions with different weight, since a model that passes
  a wrong answer makes "mastered" overstate understanding, while one that
  misses a correct answer is only harsher than intended.
- **The review screen's up/down navigation is now rebindable.** The
  multiple-choice and key-point lists move with `k`/`j` by default (the arrow
  keys always work too); rebind them under `[keys.review]` as `up`/`down`, like
  any other review action.
- **An experimental native app lives in the tree (`apps/mobile`)**: a Flutter
  shell that embeds the same Rust core (the real parser, scheduler, and store)
  and reviews a real decks folder offline, on Android and as a Linux desktop
  window. It browses workspaces and loose decks (titles plus a due dot), picks
  a session depth, and runs the flip, multiple-choice, line-by-line, and typing
  checks, with progress landing in the same per-workspace and root stores the
  web and CLI use. Still early and source-built only (`make phone` / `make
  tablet` / `make desktop`; `make push-decks` copies a host folder into the
  emulator; the desktop window reads `ALIX_DECKS_DIR`), not part of the
  released binaries, and its data layout may still change without migration.
  Explain-mode cards render as plain flip cards for now.
- **For library consumers: a `full` cargo feature** (on by default) now gates
  the AI backends and the web server. Depending on `alix` with
  `default-features = false` compiles just the lean core (decks, scheduling,
  sessions, store) with no behavior change for anyone using the defaults; this
  is the half the mobile app embeds.

### Removed
- The placeholder **"Fun" kids theme**, superseded by the three real kids
  themes above.

### Changed
- **Augment batches on the Claude backend now share one conversation**: the
  first target sends the cards once, each later target runs as a short
  follow-up that references them by index, cutting prompt cost and latency on
  multi-target (and workspace-wide) batches. Other backends and single-target
  runs keep their self-contained one-shot per call; a failed target starts a
  fresh conversation for the rest of the batch.
- **The acquire cooldown is configurable and defaults to 5 minutes** (was a
  fixed 1 minute): `[review] acquire_cooldown` (`"90s"`, `"10m"`, `"1h"`; a
  bare number is minutes, `"0"` disables it), also overridable per workspace
  in `alix.local.toml`. One knob paces both gaps it always governed: the
  settle before a new card's first graded quiz, and the floor before any
  just-seen card (a miss, a wrong pick) may return. With the longer default
  a short session can now end while a missed card is still cooling; it slots
  back in on its own (the summary keeps polling), or next session.
- **Breaking:** `POST /api/check` no longer reads the client-sent `ordered`
  flag; whether typed lines pair by position (`typeline`) or match in any
  order is derived server-side from the card's mode. Send `{lines}` only;
  an `ordered` field in the body is ignored.
- The mobile app renders note code blocks verbatim (fenced ``` blocks are
  no longer sentence-split), through the same structured note units the web
  serves.
- **The tutor's "Save note" is now "Make this a note"**, matching "Make this a
  card", and both distill actions are rebindable: **Breaking:** the
  `[keys.review]` key `save_note` is renamed `make_note` (still `ctrl-n`), and
  the new `make_card` (default `ctrl-d`) triggers "Make this a card" from the
  keyboard.
- **Leaving the tutor now asks first when the conversation is unsaved**, the
  same pause as leaving a session: the transcript survives on the current
  card, but moving on to the next one would drop it before it became a note
  or a card. Enter leaves, Esc stays.
- While the tutor is thinking, the panel shows the looping alix logo next to
  "Thinking…" (the header logo already looped; this one sits where you're
  looking), and the transcript no longer rebuilds on every poll tick.
- **The review tutor is now offered during a card's first encounter (acquire),
  once you reveal the answer.** It stays hidden during the blind attempt,
  matching the after-reveal rule the rest of review follows, so you can ask about
  a brand-new card (and make a card from it) without waiting for its first graded
  review.
- **Breaking:** the progress store now lives with your decks
  (`<decks_dir>/progress.json`) instead of the platform data dir
  (`~/.local/share/alix/progress.json`); bare `alix` and `alix <decks_dir>` share
  one store. Move an existing store once:
  `mv ~/.local/share/alix/progress.json ~/decks/progress.json`.
- **Breaking:** the `/api/augment/generate` request body now takes a
  `targets` list of `{target, with?}` entries (each with its own optional
  guidance) instead of a single `target`, and the augment poll response
  (`AugmentDto`) also reports `queued`, `done`, and `failed` targets for
  batch progress.
- The topology augmentation now defaults to a pedagogical (foundations-first)
  order when you give no guidance, named `pedagogical order` rather than `auto`;
  a guidance steer still overrides it.
- `alix generate` and its review pass now keep each card's answer to exactly
  what its front asks, moving extra context into the note instead of
  over-answering the question.
- `alix generate` and its review pass now turn a mapping of pairs into one
  cloze card (one line per pair, the recalled half blanked) instead of a
  "match each X to its Y" card that asks to recall the whole table at once.

### Fixed
- The tutor's **Save note** (and the new **Make this a card**) now stay disabled
  until the tutor has actually answered, instead of looking active and silently
  doing nothing on an empty conversation.
- The formatting augmentation no longer strands already-clean cards as a
  permanent gap: a card the formatter checks and leaves as-is is now recorded as
  done, so coverage completes instead of a Generate that appears to do nothing.
- The exam overlay now hides its scrollbar, matching the augment and review
  surfaces, instead of showing one and reserving a gutter for it.
- Lenient exam grading no longer downgrades an incomplete-but-correct answer
  to partial: the grading criteria now say outright that covering only some
  key points still passes when what is said is right, reserving "partial" for
  an actual error (caught by the grader-calibration suite).
- **The trace walk screen now shares the session chrome.** It still rendered
  pre-re-skin chrome: no ☰ menu in the header, and a footer that packed
  Missed it/Partly/Got it/Ask/Leave into one centered row with a dead
  `0✓ 0◐ 0✗` per-checkpoint counter (that readout was deliberately removed
  from review's footer, but the walk kept its own copy). The walk now gets
  the ☰ menu (Ask Tutor only — Remove/Promote don't apply to a checkpoint),
  a zoned footer (Leave left, grade actions center, Ask tutor right, matching
  review), and the counter is gone.
- A keypoint click on an Explain-mode card that also carries a `% at:`
  citation could silently swap the whole answer region to the cited source
  excerpt instead of marking the point — the click bubbled into the answer
  region's own answer/source toggle. The keypoint `<li>` now stops that
  propagation.
- Opening the web app in the first moments after a server start could paint a
  blank page (the page booted before the server was ready to answer); the boot
  now retries briefly instead of giving up on the first failed fetch.

## [0.4.0] - 2026-07-11

### Added
- **A kids web client** (touch-first, aimed at roughly age 10): a calm,
  consumption-focused frontend over the same engine, served at `/` when
  `[serve] audience = "kids"`. An adult builds and augments a box (workspace)
  of decks in the regular web app, then a kid opens it here: pick a box, pick
  a deck, pick a depth — **Tap the answer** (Recognize) or **Say it yourself**
  (Recall) — and review, with a mascot's short "why" on reveal and a kid-safe
  Ask Alix tutor. v1 is consumption only: augmenting a deck, the AI exam, and
  traces stay adult-only for now. Self-hosts the Baloo 2 font (SIL OFL, see
  `NOTICE`). No API or contract change — it's a second frontend over the same
  `/api/*` endpoints documented in `docs/API.md`.
- **`[serve] audience` config key** (`"adult"` default, or `"kids"`) — which
  frontend `/` serves, and which voice the tutor uses.
- **Ask tutor on Recognize.** The tutor button now appears on a Recognize
  (multiple-choice) card's feedback, the same as Recall and Reconstruct. It's
  most useful after a wrong pick ("why is the highlighted option right, not the
  one I picked?"). The key already worked there; this makes it visible and
  tappable.
- `/api/decks` rows now carry `selectable` — whether the row's `name` can be sent to
  `/api/select` (decks: yes; workspace/folder rows: no). Clients no longer have to infer
  it from `is_workspace`.
- On a first-seen (acquire) card, `h` (or a tap on the answer) hides / un-hides
  the revealed answer in place, so you can self-test the fresh encoding (conceal
  it, try to recall, show it to check) before "Seen" moves on. It only flips the
  answer's visibility: the note, the footer, and the layout stay put, nothing
  reflows. Shown as a small corner cue like the source⟷answer swap on a cited
  card, not a separate button. A first-encounter aid only: an ordinary review
  drills a card by failing it, which brings it back spaced.
- A multi-line front now renders as centred lines instead of one run-on line, so
  a dual-direction card's reverse side (its several alternatives, shown on the
  question side) reads clearly.
- An end-to-end smoke suite for the alix web clients — both adult and kids
  (`make e2e`, Playwright): a click must produce the expected request,
  response, and screen, with no uncaught page errors, covering session
  select/grade, the picker, and a multi-line answer rendering as separate
  lines rather than one joined string.
- **A live Codecov badge on the README**, backed by a real-server HTTP
  round-trip test suite (`tests/api.rs`) that drives `/api/*` over the wire
  rather than calling handlers in-process — the deterministic half of
  contract hardening. Line coverage crossed 90%; a handful of functions a
  deterministic test can't meaningfully drive (a live OS route lookup,
  print-only QR output, a two-call AI workspace build) are marked
  `#[cfg_attr(coverage_nightly, coverage(off))]`, each excluded one function
  at a time with a stated reason, so the number stays honest.
- **Web picker self-sufficiency: the ☰ menu gains Add deck… (generate from a
  URL, import .tsv/.txt, receive a wormhole code or .zip), Share… (wormhole
  code or .zip download), Reset… (typed-name confirm), Doctor, and Pair a
  device (QR)** — all additive `/api/*` endpoints, pinned in the contract
  suite and documented in `docs/API.md`.
- **`docs/API.md` — the web JSON API is now a written, tested contract.**
  Endpoints, DTO field tables with nullability, the flows (select→state→grade,
  walk, exam, augment, ask), auth, and the stability rules clients may rely on
  (unknown fields must be ignored; enum vocabularies are open sets unless
  marked closed). Every response shape is pinned by full-object snapshot tests
  (`mod contract`), which also emit `tests/contracts/*.json` — canonical
  examples and a codegen corpus for client models.
- **Cram is back — as a tick-box in the picker's Learn ▾ menu** (key `c`,
  rebindable as `[keys.picker] cram`), combining with any depth; plain Learn
  never crams. Its semantics got honest and due-aware: cram only changes
  which cards are queued — a card that was genuinely due grades exactly like
  a normal review (full credit, recorded, Reconstruct→Recall propagation
  included), while a pass on a not-yet-due card only re-anchors its due date,
  so grinding can't inflate intervals; misses always count. At Recognize,
  cram serves every card — the repeatable quiz a badged deck otherwise
  wouldn't offer. `/api/select` accordingly takes `cram` plus optional
  `max_new`/`limit` overrides, closing the thin-client pacing gap. A failed
  session start now also shows a brief notice instead of failing silently.
- **Scoped roots: `alix <dir>` serves that folder as a self-contained
  instance** — its own catalog plus its own `progress.json` and `recent.json`
  kept inside the folder, so several instances run side by side without
  sharing state (one per family member: `alix ~/decks-maria --lan --port
  7781`). A workspace folder opens the picker drilled into it, over its own
  store.
- **`alix doctor` — one health command.** Checks the config parses, the
  progress store is readable, the decks folder scans (broken decks point at
  `alix deck check`), and the backend CLI is installed — each problem with a
  one-line fix. `--backends` adds a real end-to-end probe of the configured
  AI backend (`--all-backends`: all four).
- **A scannable pairing QR in the `--lan` startup output**, alongside the
  pairing URL — which now shows the machine's actual IP instead of a
  placeholder. A phone or tablet pairs by pointing its camera at the
  terminal.
- **`[review] max_new` and `limit` config keys** for session pacing, with
  per-instance `--new`/`--limit` overrides on bare `alix` (precedence:
  flag > config > built-in 10 / no cap).
- **`alix share <path>` and `alix receive <code>` — send decks to someone
  over magic-wormhole** (shells out to the `wormhole` binary; install it
  separately). Share takes a deck, folder, or workspace and stages a copy
  with the personal state left home (progress, recent list, local pacing);
  receive lands a deck in the decks dir (or `--workspace <dir>`) and a
  folder under its own name, stripping any leaked personal files. The code
  mnemonic and transfer progress come straight from wormhole. No wormhole
  installed? `alix share <path> --zip [--output <path>]` writes the same
  staged copy as a `.zip` instead, and `alix receive <file.zip>` integrates
  one. The open picker re-scans its catalog when the browser tab regains
  focus, so a deck received (or generated) from the terminal shows up when
  you switch back — no manual refresh.
- **`alix workspace init <dir>`** scaffolds an empty workspace — an
  `alix.toml`, a personal `alix.local.toml`, and an `assets/` folder, no
  decks. Both TOML files are written fully commented, every key explained
  inline, so they document themselves. Grow the workspace with
  `alix generate … --workspace <dir>` or `alix deck import … --workspace
  <dir>`, which write their deck into the workspace.
- **`stats`, `list`, and `reset` take a deck, a folder, or a workspace.**
  A folder or workspace expands to its member decks against the store that
  serving uses; `reset` on a workspace clears card progress, virtual cards,
  and mastered flags together, under one blast-radius confirmation.
- **The web exam launch pre-flights the backend's ability to reach the
  deck's source**, failing at start instead of mid-exam.

### Fixed
- Group rows in `/api/decks` no longer report `reviewable` unconditionally: a workspace/folder
  row now aggregates its members, and a deck that fails to parse reports nothing reviewable.
  (The kids app's box line and the adult picker's dim states are honest now.)
- `docs/API.md` described `DeckItemDto.name` as a key you can always send to
  `/api/select`. Only deck rows are selectable: a workspace or folder row is
  a container and `/api/select` rejects it (400) — drill into its `members`.
  A group row's `reviewable_*` flags aggregate its members rather than
  inviting a select. (Found when the kids client believed the doc and shipped
  a button that did nothing.)
- `docs/API.md` documented `/api/walk/grade`'s `delta` keys as `"g"|"p"|"m"`;
  the server and web client have always used `"n"|"p"|"f"`. The doc now
  matches the wire (caught by the new HTTP round-trip suite).
- A wrong Recognize pick now shows which option was right before moving on
  (Continue grades it failed) — the silent instant-demote skipped the
  corrective moment.
- A just-finished card can no longer come straight back: its re-serve clock
  now floors at the card transition, so time spent on the feedback screen or
  the next card never eats the gap.
- The same-card floor now covers Recognize too: a failed pick used to
  resurface instantly (a deliberate exclusion at the time); with one card
  left, that meant an instant boomerang. It now re-queues but stays floored
  like every other depth.
- Multiple-choice options now reshuffle on each appearance of a card, instead
  of sitting in the same positions every time — a retry could otherwise be
  solved by position memory rather than actually recalling the answer.
- The picker's ⟳ now re-reads the config, so a changed `decks_dir` takes
  effect without a restart (scoped `alix <dir>` instances stay pinned to
  their folder).
- A sequence card (`% reveal: line`) at Recognize is now quizzed as one whole
  answer among the cached distractors, instead of a meaningless pick-one-step
  choice built from the card's own lines (falls back to the self-report chips
  when no distractors are cached).
- The acquire view's badge no longer names a check — a brand-new card shows
  just `NEW` (the attempt-first reveal is ungraded).
- The tutor's "couldn't find the source" reply, for a frozen card whose live
  source root is gone, now comes back immediately instead of round-tripping
  through the model to have it echo the same fixed sentence.
- `alix generate <dir>` (workspace build) no longer blocks on, or touches, a
  populated destination. The build always stages into a scratch dir first,
  then merges the new files in one by one: a name already present in the
  destination keeps your original untouched and reports the new version's
  location (in the kept-around staging dir) instead of failing the whole run
  or overwriting anything. `--force` overwrites collisions. A leftover staging
  dir from a previous conflicted build now asks for confirmation before it's
  wiped and rebuilt, and dot-prefixed folders are hidden from the picker's
  scan so a kept-around staging dir never shows up as a bogus workspace.
- A taken port now errors immediately with a `try --port` hint — the server
  binds before printing its URL, so a clash no longer shows a
  success-looking line first.

### Changed
- **Breaking: an ambiguous bare deck name is rejected instead of silently
  resolving.** The same file name occurring in two containers now fails
  with 400 from every name-taking endpoint instead of silently resolving to
  one of them; use the qualified `<workspace>/<file>` name.
- **Breaking (API): three contract shapes normalized before the freeze.**
  `WalkDto.verdict` sends `passed`/`partly`/`failed` machine tokens instead
  of English display labels; `POST /api/walk/leave` returns the picker
  `StateDto` like every other closer (was a bare 204); a trace exam's re-sit
  cooldown is an `ExamDto` in a new `cooldown` phase with `cooldown_ms` set
  (was an untagged `{cooldown_ms}` object).
- **Breaking: one `generate` verb for all AI authoring — `explore`, `trace`,
  `deck generate`, and `deck check` are removed.** `alix generate <source>`
  routes by the source: a URL/file becomes one deck; a directory is explored
  first and the plan's size decides (one item → a deck, more → a workspace,
  shown and confirmed before building; `--plan` previews; `--deck` forces a
  single deck; `--workspace <dir>` names the destination); `--trace` authors
  a trace over a source (`--trace --plan` = the suggestions menu); naming an
  existing `% trace:` stub builds its checkpoints in place. The terminal
  trace walk is gone — traces are walked in the web picker, and the old
  `--grade` flag becomes the `[trace] auto_grade` config key (opt-in AI
  grading per hop, now available in the browser walk too). `alix doctor
  <deck>` lints a single deck (was `deck check`); `import` moves under
  `deck` (`alix deck import`).
- **Breaking: the CLI collapses to `alix [dir]` plus task subcommands —
  every review starts from the picker.** Removed outright (pre-1.0, no
  aliases): `alix <deck>` direct-deck launch, the `review` and `workspace`
  subcommands (`alix <dir>` covers workspaces), `alix backend check` (use
  `alix doctor --backends`), the `--serve` flag (everything is served since
  the terminal UI was removed), and the per-session flags
  `--cram`/`--depth`/`--topology`/`--region`/`--order` — depth, topology,
  and region are picked in the web picker; order is the deck's `% order:`
  directive. Bare `alix` keeps only `--lan`, `--port`, `--token`,
  `--config`, `--new`, `--limit`.
- Redesigned the web UI (IBM Plex typography, borderless/hairline dark theme).

## [0.3.0] - 2026-07-07

### Added
- **Session depths: Recognize, Recall, Reconstruct — replacing the retired
  `% mode:` checks.** Every review now happens at one of three independent
  depths, chosen per session (`--depth`, or the web picker's split **Learn**
  button and its ▾ menu — on the keyboard, `v` opens it and `1`/`2`/`3` pick a
  depth, rebindable in `[keys.picker]`) instead of authored per card or set in
  config; plain
  Learn reuses the deck's own last-used depth (first use: Recall). **Recall** is the
  familiar reveal-and-self-grade flashcard. **Reconstruct** keeps its own FSRS
  schedule per card, independent of Recall — no cross-crediting, two separate
  practices — and has you type a short answer or a cloze gap, type each line
  in turn (`% reveal: line`), or explain a longer one. **Recognize** is
  unscheduled and boolean (no FSRS state at all): a multiple-choice pick where
  there's material to build one, the same attempt-then-reveal a first
  encounter gets otherwise. A card no longer climbs or descends between
  depths on its own.
- **A full Reconstruct pass credits a due Recall schedule — downward only,
  pass-only.** Getting a card fully right at Reconstruct (outside cram) now
  counts for its Recall schedule too: full credit if recall was due at that
  moment (recorded in the card's history, marked as propagated), only a
  pushed-out due date if it wasn't (memory untouched, nothing recorded).
  Partial and failed answers never propagate, and a card drilled only at
  Reconstruct never gains a recall schedule from this. Alongside it, any
  full pass at any depth — cram included — marks the card recognized if it
  wasn't yet.
- **Two quiet overrides give the learner the final say.** A typed
  Reconstruct check normalizes both sides (case, whitespace, trailing
  punctuation) and compares exactly, then shows the diff — but *you* grade it,
  so a typo you recognize as one can still be Got it (no edit-distance
  tolerance guessing at "close enough"). A correct Recognize pick can be
  quietly walked back with an **"I guessed"** link, which un-marks the card
  and re-queues it.
- **Per-deck badges for Recognize/Recall/Reconstruct**, shown in the picker.
  A deck earns a depth's badge once every card is currently solid at it
  (recognized; or at/past 21 days of FSRS stability) — solid while it still
  clears the bar, dotted once a card has lapsed below it (a badge keeps its
  earn date once won). Only the highest badged depth shows, and a deck that
  gains cards after being badged gets a small "new" chip. Informational only:
  badges never gate anything — passing the AI exam is still the only thing
  that unlocks a dependent deck.
- **`alix list`/`alix stats` report per depth.** `list` now shows each card's
  Recall and Reconstruct schedule state plus a ✓ once it's recognized;
  `stats` adds a per-depth due count.
- **`% reveal:` — the authored presentation axis.** How a card is *presented*
  is now its own directive (deck or card, default `flip`): `flip`, `cloze`
  (`{{spans}}`), or `line` — while how deeply it's *checked* is the session
  depth above, never authored into a shared deck. The web review UI shows a
  small badge naming the check (`flip` / `line` / `typing` / `explain`) so how
  you'll interact is clear up front. See the **Changed** note for the
  `% mode:`/`#?` break.
- **Remediation cards are now virtual cards in the store.** A failed source
  exam's remediation cards live in alix's store instead of being written into
  your deck file. They drill like normal cards and count toward a deck's *due*
  total (not its card count), dedupe on regeneration, archive when their FSRS
  interval reaches `retire_after`, and revive if the same gap fails again. See
  the **Changed** note below for the behavior break.
- **Promote a virtual card into its deck.** A review-time action appends a
  remediation card to the deck file and drops the virtual copy — "Promote to
  deck" in the web review menu (rebindable `[keys.review]` `promote`, default
  `ctrl-p`). Offered only while reviewing a virtual card. The promoted card
  keeps its review schedule; it doesn't restart.
- **Exam-fail remediation count, and a "remediation card" review label.** The
  post-remediation exam screen now reports how many remediation cards the
  failure created or revived. While drilling a still-virtual card,
  the review screen's existing mode badge reads "remediation card" in place of
  "new card" — it reverts once the card is promoted.
- **`[review]` config section — FSRS pacing.** `retention` (target recall
  probability, 0.70–0.99, default 0.9; higher = shorter intervals) and
  `retire_after` (a duration `"1y"` / `"6m"` / `"2w"` / `"30d"`, or `"never"` to
  disable retirement; default `"1y"`).
- **Per-workspace pacing via `alix.local.toml`.** A workspace can override the
  global `[review]` retention / retire_after in a personal `alix.local.toml`
  beside its `alix.toml` — kept separate from the shared manifest, so it never
  travels when you share the workspace.
- **`alix deck check` warns on a non-gating prerequisite** — when a sourced deck
  `% requires:` a source-less deck, that edge can't gate its exam; the lint names
  it and suggests adding a `% source:`.
- **Pairing token for `alix serve --lan`.** Serving to the network now
  auto-generates a token (printed at startup) and requires it on `/api/*`, so the
  LAN endpoint is no longer wide open. Pin your own with `--token` or
  `[serve] token`; the browser picks it up from the printed `…/?token=…` URL, and
  native clients send it as a bearer token. Opening the web UI without a valid
  token shows a prompt to paste it (then reloads) instead of a blank page.
- An `examples/gh-review-prep.rs` showing how to compose the library into an ephemeral, goal-scoped workspace for understanding a change you must read closely (a GitHub PR or issue) before acting on it. Read-only; a demonstration of composability, not a GitHub feature.
- **`[ask] backend` selector.** All AI calls now route through a pluggable
  backend. Set `backend` in `[ask]` to choose among `claude` (default), `gemini`,
  `codex`, or `copilot`. Auth is each CLI's own login — alix stores no API keys.
- **`alix backend check [--all]` health probe.** Sends a trivial tool-free
  request to the configured backend (or all four with `--all`) and reports
  whether each is installed, signed in, and responding. The only reliable way
  to confirm the whole path works end-to-end. Errors are the same actionable
  messages the rest of alix shows (rate limit, not signed in, not installed).
- **Gemini backend (`[ask] backend = "gemini"`).** alix's AI calls can now run
  through the Google Gemini CLI (`gemini -p`, headless). Tool access maps to
  Gemini's read-only tools via an `--allowed-tools` allowlist (the standard
  read-only file tools plus web fetch and search) — no write or shell tool is
  granted, and none is auto-approved. Trace building picks each backend's own strong model (Claude
  still defaults to `opus`; other backends inherit the CLI's default), so leaving
  `[trace] model` unset does the right thing per backend.
- **Codex backend (`[ask] backend = "codex"`).** alix's AI calls can now run
  through the OpenAI Codex CLI (`codex exec`, headless). Codex takes the prompt
  as a command-line argument rather than on stdin, and its access is governed by
  a sandbox rather than a tool allowlist: it runs `--sandbox read-only` with
  `--ask-for-approval never`, which permits reading local source but blocks
  writes, shell escalation, and the network — so a fetch/search grant can't be
  honoured under this backend (source reading still works).
- **Copilot backend (`[ask] backend = "copilot"`).** alix's AI calls can now
  run through the GitHub Copilot CLI, authenticated via `gh auth login`.
- **Backends degrade gracefully.** An AI feature now checks the selected
  backend's capabilities *before* doing any work and refuses cleanly when they
  don't match — e.g. an exam or `deck generate` over a URL `% source:` under a
  backend that can't fetch the web says so and names the fix (point the source
  at a local file, or switch `[ask] backend`), instead of crashing or
  fabricating a result. Trace `build`/`suggest` and `explore` gate the same
  way. A failed AI CLI now also leads with actionable guidance: a rate-limit or
  quota error suggests waiting or switching backend; an unauthenticated error
  suggests running the CLI's login once (the raw detail is still shown).
- **Pre-flight source-size guard.** Before `deck generate`, `trace --build`,
  `trace --suggest`, and `explore` read a large source, alix measures the
  estimated size and asks for confirmation. Pass `--yes` to skip the prompt in
  non-interactive scripts. `exam` instead truncates the source to 100 KB and
  prints a notice so the exam can still run unattended.
- **Web picker: a workspace's goal shows in its drill-in.** Opening a workspace now
  shows its goal (the one-line description) under the title eyebrow, the same goal the
  top-level list shows on the workspace row — so the context stays visible while you
  pick a member deck.
- **Web picker: a drawer indicator.** A quiet `▾` at the bottom-centre of a picker
  row marks a deck that has a focus drawer (a cached topology), so you can see at a
  glance which rows expand on focus instead of discovering it only after selecting.
  The marker lights to the accent colour on the focused row.
- **Draw input (web).** Answer a `flip`/`explain` card by drawing or
  handwriting on a canvas, then self-grade — either authored per card/deck with
  `% input: draw` (for answers that can't be typed, e.g. diagrams) or via a
  per-device "Draw answers" toggle in the review menu. Web-only; the drawing is
  ephemeral (never persisted or sent to the server).

### Changed
- **Breaking (store): per-depth schedules replace the single `fsrs` field.** A
  card's progress is now stored per depth (`recall`/`reconstruct`), plus a
  `recognized` flag, instead of one shared FSRS state. Pre-1.0, no migration:
  an existing store loads fine, but every card starts with empty schedules at
  every depth — a one-off rewrite for anyone who wants to seed it otherwise.
- **Breaking: `--max-typos` and the `fuzzy` check mode are gone.** A typed
  check now normalizes (case, whitespace, trailing punctuation) and compares
  exactly, then shows the diff and leaves the pass/fail call to you — no
  edit-distance tolerance guessing at "close enough" (it used to let "affect"
  pass for "effect" within tolerance).
- **Breaking (store):** dropped the legacy Leitner `stage`/`stage_entered_ms`
  fields now that FSRS is the sole scheduler; `stage_entered_ms` is renamed
  `acquired_ms`. Pre-FSRS cards lose their stage-derived interval carry-over.
  Pre-1.0 — no migration path. `alix stats` no longer prints a per-stage
  histogram and `alix list` shows the FSRS state instead of a Leitner stage.
- **Breaking: the terminal frontend is removed — alix is web-first.** Bare
  `alix` (or `alix <deck>`) now opens the local web app and prints its URL,
  instead of a `ratatui` TUI; `ratatui` and `crossterm` are dropped as
  dependencies (~129 transitive crates gone). The standalone `alix exam` and
  `alix browse` commands are removed — both are reached through the web
  picker instead (pick an `exam due` deck to sit its exam; press "Browse" on
  a deck to read through it). The `% frontend:` directive (`any`/`tui`/`web`)
  is removed — every card just renders in the web app. `alix reset` and deck
  dependency editing (the old `alix deps`/`alix require`) are now
  non-interactive: name a deck, or pass `--card <id>`/`--all` to `reset`;
  edit `% requires:` lines by hand. `alix trace`'s walk is unaffected — it
  still runs in the terminal (a plain stdin loop, never a TUI).
- **Breaking: `% mode:`, the `#?` cloze marker, and the `--mode` flag are
  removed** — replaced by `% reveal:` and the session depths above. A cloze card
  is now `% reveal: cloze` (was `#?`); a deck's presentation is
  `% reveal: flip|cloze|line` (was `% mode:`); and how deeply you drill is the
  session depth (`--depth` or the split Learn button), not a per-card mode or a
  CLI flag. **Cards once authored `% mode: typing`/`explain` review as
  reveal-and-self-grade in Recall sessions** — start a **Reconstruct** session
  to get the producing (typing/explain) checks. **Card ids are preserved**: the retired markers were never part of a
  card's identity hash, so progress carries over. Upgrade an existing deck with a
  one-off textual rewrite (`#?` → a `% reveal: cloze` line; `% mode:` → `% reveal:`
  where a reveal-method applies, dropping `typing`/`fuzzy`/`choice`/`explain`), or
  re-generate it — either way ids and history survive.
- **Breaking: a failed exam no longer appends remediation cards to your deck
  file.** Remediation cards are now created as virtual cards in alix's store, so
  the deck `.txt` stays byte-for-byte unchanged. Drilling, due counts, and the
  exam re-sit are otherwise the same; use the new **promote** action to move a
  remediation card into the deck if you want it there permanently.
- **A virtual (remediation) card's retirement is now fully derived**, matching a
  deck card: purely its FSRS interval vs. `retire_after`, no stored archive
  flag. Raising `retire_after` later un-retires a previously archived virtual
  card whose interval now sits below the new cap, the same as it would for a
  deck card.
- **Breaking: a review session scores each card once per appearance.** A missed
  card is no longer re-drilled immediately in the same sitting; it keeps its
  short spaced step and re-appears once that step has elapsed, interleaved behind
  other due cards (so every scored review is genuinely time-separated). When
  nothing is due right now the session ends — a card still cooling is picked up
  the next session, or re-enters on its own if you keep the window open. Fixes
  cards graduating too early from same-session re-drills.
- **Breaking: a card graduates only after two spaced correct recalls.** A single
  Good after a miss no longer promotes a card to the long-term review phase faster
  than two clean passes would; two full Goods graduate it, a miss resets that
  progress, and a *partly* is neutral. (A lapsed card still re-graduates on one
  Good.)
- **A just-seen card starts drilling in the same session.** Acquiring a new card
  (the ungraded "Seen" first exposure) now settles for ~1 minute (was ~5) and the
  card stays in the sitting, so its first graded quiz comes back interleaved a
  minute later instead of waiting for a new session.
- **Breaking: FSRS is now the only scheduler.** alix schedules with FSRS-5 (via
  the `rs-fsrs` crate) for every review; the Leitner and SM-2 schedulers are
  gone, along with the `% scheduler:` directive and the `--scheduler` flag that
  chose between them. Grades map to FSRS ratings (missed it / partly / got it →
  Again / Hard / Good) and the next interval comes from FSRS, not a fixed stage
  ladder. The old per-card `stage` is kept only as a seed for a card's first FSRS
  review, so existing progress isn't reset. No compat shim, pre-1.0.
- **Breaking: `% unlock-stage:` removed** (the directive, the `--unlock-stage`
  flag, and the `[defaults] unlock-stage` key). A deck's exam is now gated on
  **graduation**: it turns *exam due* once every card has reached FSRS's review
  phase (past the initial learning steps), rather than at a configurable stage bar.
- **Retirement is now interval-based.** A card retires (rests until `alix reset`)
  once its FSRS interval reaches `retire_after` (default 1 year, configurable,
  `never` to disable) — previously it retired at the top Leitner stage.
- **`--cram` refreshes without rewarding.** A correct answer under `--cram` now
  re-anchors the card's due date by its current interval — no FSRS update, no
  reward — so cramming keeps cards fresh without inflating their schedule; a cram
  miss still lapses normally. Previously `--cram` fully rescheduled every answer.
- **`alix serve --lan` now requires the pairing token** on `/api/*` (auto-generated
  unless you set one). The HTML shell, theme assets, and images stay open — only
  the JSON API is guarded; localhost serving is unchanged (open).
- **Breaking: `alix trace --serve` removed.** Trace walking in the browser now
  goes through `alix serve`'s deck picker (pick the trace) — the standalone
  single-trace web server is gone, so there's now exactly one web server. `alix
  trace <deck>` still walks in the terminal.
- **The tutor is now backend-agnostic ("Ask Tutor" / "Tutor").** The in-session
  tutor was labelled "Ask Claude" in the UI and docs. It works with every
  supported backend (Claude, Gemini, Codex, Copilot), so it is now called
  "Tutor" throughout — in the ☰-menu button, the hint text, the README, and
  the book. The `[ask]` config section name is unchanged (it was already
  neutral).
- **Breaking — `alix check` is now `alix deck check`.**  Deck validation moved
  under the `deck` noun-group for consistency with `alix deck generate`/`alix
  deck augment`. The command is identical; only the path changed: `alix check
  <deck>` → `alix deck check <deck>`. No compat shim, pre-1.0.
- **Multi-turn tutoring works on every backend.** Claude keeps a running
  conversation with its session flags (`--session-id`/`--resume`); other CLIs
  don't have those, so alix drops them for a backend without a session mechanism.
  To restore cross-turn memory there, the tutor now re-inlines the accumulated
  Q&A transcript into each prompt — a follow-up on a non-Claude backend carries
  the prior questions and answers, so the tutor no longer forgets what you just
  asked. Claude's efficient `--resume` path is unchanged.
- **Breaking — config keybindings are namespaced under `[keys]`.** Every key table
  is now a `[keys.*]` subtable: `[keys]` → `[keys.review]`, `[picker]` →
  `[keys.picker]`, and `[browse]` → `[keys.browse]`. This groups all bindings in one
  place and disambiguates the shared `remove`/`quit` action names per surface. Update
  your `~/.config/alix/config.toml` — the old top-level `[picker]` / `[browse]` and a
  bare `[keys]` section now error (no compat shim, pre-1.0). `alix config --init`
  writes the new layout.
- **Review cards settle a short answer below the midline.** The answer region now
  grows to fill the space between the question and the note and centers its content
  when it fits — so a short answer sits just below the card's middle instead of
  clustering under the question, and the lower half no longer reads as empty. Long
  answers and cited excerpts still top-align and scroll (using the whole card), and
  the question never shifts when the answer is revealed. Applies in browse too.
- **Web picker: cleaner dependency-tree lines.** The workspace drill-in's tree
  connectors (`├─` / `└─` / `│`) are now drawn as subtle dotted CSS guides in the row
  border colour — aligned under each parent's label and stopping at each row's border
  rather than crossing the gaps between rows — instead of single-line box-drawing
  glyphs that broke into disconnected segments on the tall rows.
- **Multi-line review answers left-align by default.** An answer with more than one
  line (a list, or several sentences) now renders as a left-aligned block, centered
  as a whole, instead of each line being independently centered (which read as ragged
  — especially for lists). Single-line answers stay centered, and reshaped-list
  bullets are unchanged.
- **The web UI header shows an animated `alix` wordmark.** The lightning-bolt mark in
  the review/picker and trace-walk headers is now a self-contained `<alix-logo>` web
  component — a flat orange "mitosis" wordmark that plays a one-time reveal on load
  (and on reload / `r`) and loops as a calm loading indicator while a Claude/server
  call is in flight. The shared header chrome — the `<head>` boilerplate and the
  brand mark — is now single-sourced (`_head.html` / `_brand.html`, filled in by the
  server) so all pages stay consistent.
- **Trace walk is now an in-page mode of the web review UI.** Picking a trace from
  the deck-selection screen no longer navigates to a separate `/walk` page — the walk
  runs inside `review.html` with no page reload, and trace cards match fact-card
  sizing and layout.

### Fixed
- **The web app can no longer be served stale from the browser cache.** alix
  sent no cache headers, so after an upgrade the browser could keep showing
  the previous version's page on the same address. The app shell and its
  assets now demand revalidation (`no-cache`) and live JSON state is never
  cached (`no-store`).
- **Browse left-aligns multi-line answers like review.** The read-only browser
  decided the left-aligned-block layout from the reshaped-list flag alone, so an
  unaugmented multi-line answer rendered centered (ragged, each line on its own axis)
  instead of as a left-aligned block. It now uses the same rule as review — any
  multi-line answer is a left-aligned block centered as a whole; only reshaped lists
  get bullets.
- **Web: Backspace leaves the augment view.** The augment screen accepted only `Esc`
  to return to the picker, while browse and the picker also honour `Backspace` — so
  `Backspace` felt inconsistent across views. It now leaves the augment view too, while
  the guidance box still edits its own text with `Backspace`.
- **Web picker: the header buttons are legible on light themes.** The ☰ menu and the
  ← / ⟳ nav buttons used the muted `--dim` colour, too low-contrast on some light
  themes (e.g. Solarized Light); they now use the main text colour, so they read on
  every theme.
- **Web picker: clicking empty space keeps keyboard focus.** A click anywhere in the
  picker area that isn't a row or control — including the margins around the centered
  list, not just inside it — no longer drops focus to `<body>` (where the row-nav keys
  go dead); it re-homes to the current (or first) row so arrow-key navigation stays live.
- **`alix explore --build` freezes cited excerpts more reliably.** When a generated
  `% at:` locator dropped (or added) a leading subdirectory — e.g. `chapter.md`
  when the file is at `src/chapter.md` — freezing couldn't find it and skipped it
  (`cited file not found, not frozen`), leaving a checkpoint without its source.
  Resolution now falls back to a basename search under the source root to recover
  the excerpt, and the fill prompt pins every locator to one consistent root so the
  mix is less likely to arise.
- **Workspace icons draw fast, without timing out.** The `explore --build` icon
  prompt now caps the emblem at a few compact primitive shapes instead of letting
  the model emit long `<path>` coordinate data — the token-heavy part that made the
  draw slow enough to time out (`could not draw a workspace icon: 'claude' timed out
  after 120s`). The draw also retries once. (Supplying `--icon`, or dropping a
  conventional `assets/icon.*`, still skips generation entirely.)

## [0.2.0] - 2026-06-30

### Added
- **Web picker: browser-style back + refresh buttons in the header**, for people
  who reach for the mouse/touch over keybindings. The **←** button goes back a
  view (disabled at the top level; the keyboard equivalent is `Esc`/`Backspace`,
  since the `←` *key* steps the focus drawer's regions) and `⟳` re-scans the deck
  list (also bound to the new `r` key). Refresh moved out of the burger menu, and
  the drill-in's footer "Back" chip is gone — the header **←** replaces it.
- `alix deck augment --target format` — a non-destructive pass that reshapes a
  badly-shaped card (e.g. a list crammed into one prose answer) into clean
  display lines, a tidier front/note, and a suggested answer mode, applied at
  review without touching the deck file or card identity. Also available from the
  web Augment screen. The reshaped output drops noisy inline backticks and puts a
  code snippet in a fenced block, rendered as a monospace code box on the card.
- **Augment decks from the web picker — no CLI needed.** Press **`a`** on a deck
  (or its new **Augment** button) to open a screen of what its augmentation cache
  holds: one row per target (choices, notes, questions, key points) with a
  coverage bar, plus its topologies. **Generate** fills only the cards a target is
  still missing — a costed background call, with a live spinner — **Remove** clears
  one target, and the topology row adds or drops named topologies. A shared
  guidance box feeds the `--with` steer. It writes the same `augment.json` the CLI
  does, so review reads it unchanged. Decks only; workspaces don't show it. (The
  terminal surface comes later — the library and server logic are shared.)
- **New cards are introduced as an *attempt*, not a cold quiz (acquire).** A
  never-seen card no longer drops you into a quiz you can't pass — its first
  encounter is a low-stakes try, then the answer, then one key ("Seen") files it on
  the ladder at stage 1, *ungraded*, with its first real quiz a later session. By
  default it's **recall** (the front shows first — try, then reveal); for a deck
  augmented with AI distractors (`--target choices`), an **atomic** card instead
  greets you as a **multiple-choice** question (pick one, see which was right). A
  guess never promotes or punishes — stage 1 either way. Start another session to
  drill what you've met (the per-session `--new` cap is unchanged — 10 per session).
  Terminal and web. The **acquire** step of the acquire → explain → maintain card
  lifecycle (the explain step shipped below).
- **Explain-mode key points — a checklist that derives the grade.** A new
  augmentation, `alix deck augment <deck> --target keypoints`, has Claude break
  each card's answer into the few load-bearing claims a reconstruction must hit
  (cached beside your progress, like distractors/notes). In **explain** mode the
  reveal then becomes a **checklist**: you tick the points you covered and the
  grade is *derived* — all → passed, some → partly, none → failed — turning the
  self-grade from a vibe into a per-claim check (TUI and web). An *atomic* answer
  (a single fact/term/date) is left without key points and keeps its plain reveal,
  the same way choice mode skips cards with no usable distractor. Tune the maximum
  with `[ai] keypoint_count` (default 5). First step toward an acquire → explain →
  maintain card lifecycle.
- **Web picker header.** The deck filter moved into the header — a compact box
  centered on the list — and a **burger menu (☰)** there holds **keyboard
  shortcuts**, **refresh decks**, **about** (the version, via a new `/api/version`
  endpoint), and **Theme…**. The **Mastered** jump moved to the header too.
- **Workspace icons in the web picker.** A workspace can show a small emblem next
  to it in the picker for quick recognition. Generated as an abstract SVG by
  `alix explore --into <dir> --build` (grounded in the workspace's topic), or
  supplied yourself with `--icon <file>` or an `icon = "assets/<file>"` key in
  `alix.toml` (else a conventional `assets/icon.*`). SVGs are tinted to the active
  theme; rasters show as-is.
- **Topology-ordered review (experimental).** `alix deck augment <deck> --target
  topology` derives a graph of how a deck's cards relate — labeled edges, a
  suggested walk, and coarse named **regions** — cached beside your progress (a
  deck can hold several, one per `--with` principle, keyed by it). `alix review
  <deck> --topology <name>` then serves the **due** cards in that walk's order
  instead of at random — SRS still decides *which* cards are due, the topology
  only reorders them — and review shows a thin **region breadcrumb** ("where am
  I", current emphasized) so the sequence reads as a path, not a shuffle. A
  single cached topology is picked automatically. Terminal and web; the edge
  labels (which would reveal answers) stay under the hood. The breadcrumb
  doubles as a **strength heatmap** — a per-card bar under each region, red
  (weak) → green (learned) — so a region greens up as you master it.
  `alix review <deck> --region <name>` **drills one region** (SRS still picks
  what's due within it). In the **web picker**, selecting a deck that has a
  topology opens an inline **focus drawer** (sliding open/closed): pick which
  topology orders the session and pick a region to scope the launch — by click or
  with **← / →** — with the selection's **due/new count** shown at the right end,
  all before the session starts (the in-card breadcrumb stays read-only).
- **The ask-Claude tutor grounds a frozen card in its live source.** For a card
  in a frozen workspace (`alix explore --into --build`), the tutor now reads the
  **original crate** for context — explaining how the cited code fits the
  surrounding source — with the **frozen snapshot excerpt as the anchor** (what
  the learner sees stays the ground truth, so the tutor never reasons about a
  drifted copy). It no longer cites opaque asset names (`01.rs`). The live source
  is found via a new `% origin:` directive (below); if it's gone, the tutor
  replies *"I couldn't find the source material of this card to provide a grounded
  answer."* so you can update or drop the card. The **trace-walk tutor**, which
  had no grounding at all, gets the same treatment. Gated by the existing
  `[ask] source_access` opt-in.
- **`% origin:` — the live source root a frozen deck's snapshots came from.**
  Written into a workspace's `alix.toml [defaults]` at build time and cascading
  **workspace → deck → card** like every other directive (a card may override it
  for a cross-repo source), it lets the tutor and drift detection find the real
  crate even though `% source:` points at the opaque `assets/`.
- **`alix check` flags drifted frozen cards.** When a frozen card's snapshot no
  longer appears in its live source — the lines changed, or the file is gone — it
  warns (`card at line N — frozen excerpt no longer found in the source`), so you
  can refresh or remove that card. A snippet that merely *moved* within the file
  is not flagged.
- **Ask Claude during a trace walk.** The web walk now has an **Ask** button on
  each reveal (and the `?` key) — the same tutor a card review offers, scoped to
  the current checkpoint (its question, key points and the live source excerpt).
  Send questions, **Save note** to append a `!` line to that checkpoint, Esc to
  close. The ask machinery is now a shared component used by both the review and
  the walk, so one CLI conversation spans the session. Hosted walks only (the
  picker → walk flow); the standalone `alix trace --serve` is unaffected.
- **A "⌵ N more" marker when a source excerpt overflows the card.** A reveal
  whose excerpt is taller than the card shows a small `⌵ N more lines` pill at the
  cut edge (counting the hidden lines), in both the trace walk and a fact card's
  `% at:` citation — and it appears immediately on an overflowing excerpt, not
  only after the first scroll. The subtle edge-fade stays underneath it.
- **A trace's exam is its compression — AI-graded.** A trace's `% trace:` is a
  question ("how X becomes Y"); its **exam** is to answer it — retrace the whole
  path in a sentence or two from memory — and Claude grades that *holistically*
  against the path's checkpoints (no question generation, no source read: the
  checkpoints already paraphrase the source). **Passing masters the trace**
  (unlocking its dependents), exactly like a fact deck. Reached three ways:
  `alix exam <trace>` (which no longer refuses a trace), the **capstone** offered
  at the end of a walk (`Take the exam?`), or the picker's **"Take exam"** button
  (terminal and web) — and, like a fact deck, you can sit it **early to test
  out**, gated only by `% requires:`. A **failed** trace exam is **re-walked**
  (not remediated into cards — a trace is a path, not a card pile; its weak
  checkpoints already resurface through SRS), and after a fail it **cools down**
  before a re-sit so the graded feedback can't be pasted straight back into the
  one fixed question — `[exam] retry_cooldown_secs` (default 3600; `0` disables
  it). Built on the existing exam engine (`Sitting::start_trace` +
  `grade_compression`), so the TUI `ExamApp` and the web exam overlay drive it
  unchanged.
- **Browse a deck straight from the web picker.** A deck row's primary action is
  now **Review** (Enter), with a new **Browse** button (the go-right key, `l`/→)
  that opens a read-only walk through its cards — the review server hosts the
  browse page at `/browse`, so you no longer need a separate `alix browse`
  server. A workspace/folder still opens (drills in) on `l`/→; leaving a browse
  returns to the picker (and re-opens the launching workspace). Browse-from-the-
  picker is view-only (card removal stays a feature of `alix browse --serve`).
- **Web UI theme gallery — alix's own themes plus popular editor/slide palettes.**
  The web frontend (`--serve`) ships a gallery of colour themes: the alix
  **Dark**/**Light** originals and a playful **Kid** theme, plus crowd-favourite
  editor palettes — GitHub, Dracula, Nord, Solarized, Gruvbox, Catppuccin, Tokyo
  Night, Monokai, One Dark, Ayu, Rosé Pine, Everforest (light + dark where they
  have both). Pick one from the **Theme…** popover (the ⋮ menu, or a bar button on
  the trace walk): a grid grouped Light/Dark that **previews on a small sample card
  as you hover** (the app re-themes only when you click one) and remembers your
  choice per browser. The palette lives in a shared,
  server-served `theme.css` so every screen themes together; the default stays the
  original dark, so nothing changes unless you choose.
- **`alix deck augment` — deliberate AI deck augmentation.** A new command that
  enriches an existing deck with Claude and **caches the result** beside your
  progress (`augment.json`, keyed by card id); review reads the cache, so study
  stays instant and fully offline (Claude is never called mid-session). Three
  targets: `--target choices` writes plausible multiple-choice distractors (used
  automatically in choice mode, with the offline sampler as fallback — so choice
  now works even on a deck too thin to sample from); `--target notes` writes a
  short trivia/mnemonic note per card, shown *alongside* the card's own deck note
  on reveal (the deck file is never modified); and `--target questions` writes a
  pool of reworded phrasings of each question (same answer), a fresh one of which
  review rotates in each time a card comes up so it can't be passed by
  recognizing one fixed wording (plain, non-cloze cards). `--with "<guidance>"` steers how. Tuned under
  `[ai]` (`model`, `distractor_count`, `variant_count`, `timeout_secs`).
- **`alix check` rejects a cloze whose entire answer is one hole.** A `#?` card
  whose only hole spans the whole answer (e.g. `` `{{IdentStr}}` ``, with nothing
  but formatting around it) is a plain front→back card in disguise — blanking the
  lone hole leaves no surrounding text to recall it from. `check` now flags it
  (`cloze answer is one hole with no surrounding text … use a plain '#' card`),
  the sibling of the existing "cloze with no holes" error. Answers with literal
  context around the hole, or with two or more holes (each hole's siblings show
  as `[…]`), are unaffected.

### Changed
- **Web picker: the primary action is Learn, bound to Enter.** The focused row's
  primary action — **Learn** a deck (review or walk), Open a workspace, or Take
  exam — is bound to **Enter**, replacing the old Review/Walk split. `l`/`→` no
  longer launch a deck (they step the focus drawer's regions and enter a
  workspace). The intro prose and the "select decks" label are gone, and the list
  fills the space.
- **Browse is now an in-page mode of the web app — no separate `/browse` page.**
  Hitting **Browse** in the web picker (or `alix browse <deck> --serve`) opens a
  read-only overlay right in the main app — step through every card with
  Prev/Next/Leave, seeing the reshaped answers, notes, and images — instead of
  navigating to a separate page with its own older picker. The standalone
  `browse.html` page and the `/browse` route are gone; terminal `alix browse` is
  unchanged. **Breaking:** the web browse is read-only (card removal stays a
  terminal `alix browse` feature).
- **Reshaped list answers show as a left-aligned bullet list.** When the `format`
  augment turns a crammed prose answer into a multi-item list, the web review and
  browse views render each item with a `•`, **left-aligned** (the list block is
  centered as a whole). Single-line tidies and a card's own authored back lines (a
  poem, typing answers) are left as-is.
- **Bigger cards in the web review and browse views.** The card was capped small
  (≤820/720px wide), so it sat in a sea of empty space on a normal screen at 100%
  zoom and long questions/answers wrapped early. It now caps at ~1200px wide (94vw)
  and ~780px tall, filling far more of the viewport.
- **Web picker: `←`/`→` (and `h`/`l`) now step the focus drawer's regions, and
  going back is `Esc`/`Backspace` only.** The drawer needs left/right to move
  between regions, so those keys no longer double as "back out"; with no drawer
  open, `→` still enters a workspace / launches a deck and `←` is inert.
- Browse now shows the same display augmentations as review — the `format`
  reshape and `notes` trivia — so the two views render a card the same way
  instead of browse falling back to the raw deck.
- `alix deck generate` now shapes cards better: it splits enumerations into
  one-idea cards (or uses `% mode: line` for ordered lists) and structures
  answers and notes instead of producing prose blobs — the same shaping now
  applies to `alix explore --build` decks.
- **Breaking:** card identity is now whitespace-insensitive — an answer's id no
  longer depends on line breaks, indentation, or repeated spaces (only its
  words). Cards whose answers span multiple lines or use irregular spacing get a
  new id once and reset their review progress.
- **Leitner stage 1 now has a ~5-minute relearn/settle cooldown** (was 0). A newly
  acquired or freshly failed card becomes due ~5 minutes out for the *next*
  session, so starting another session right away no longer re-serves a card you
  just saw or just missed. In-session drilling is unchanged — a failed card still
  comes back the same run (the queue is served by position, not by due time).
- **Web picker keys.** Clicking a deck now **selects** it (opening its focus
  drawer when it has a topology) rather than launching outright — **Review** or
  Enter launches. **Browse** moved to **`b`**, freeing **← / →**: they step the
  focus drawer's region selection when one is open, and otherwise enter / leave a
  workspace. Up / down still move between decks. (The drawer is new this release,
  so only the Browse-key and click-to-select changes affect existing muscle
  memory.)
- **`alix deck augment` says what it's doing.** It now prints which augmentation
  it's generating, for which deck, and with which model before the (foreground,
  possibly slow) Claude call, instead of hanging silently until the result.
- **Breaking — one deck per session.** `alix review` and `alix browse` now take
  exactly one deck *file*: merging several loose decks into a combined session is
  gone, and a whole workspace is no longer reviewed at once. Workspaces stay an
  organizing layer — review their members one at a time (the picker drills in;
  `alix workspace <dir>` opens that picker), and a member still inherits the
  workspace's directives and store. `stats`/`list`/`reset` still take multiple
  decks (they're per-deck operations, not a merged session).
- **Breaking — review grades are now `failed` / `partly` / `passed`, replacing
  `again` / `good` / `easy`** (shown in the UI as **Missed it / Partly / Got it** —
  an honest self-report of understanding, not a pass/fail verdict; the real
  pass/fail is the AI exam). Fact-deck review and the trace walk now share one
  three-outcome grade: **failed** resets the card to stage 1, **partly** drops it
  *one* stage (a soft miss — it returns sooner but you keep most of your
  progress), and **passed** advances one stage. The old `easy` (+2 stage jump) is
  gone, and `partly` is a genuinely new middle — previously the trace walk's
  "partial" scheduled identically to a miss (full reset); now it is a distinct,
  gentler outcome on both surfaces. A `partly` does not advance the streak (it
  can't retire a card). **The `[keys]` config keys renamed** — `again`/`good`/
  `easy` → `failed`/`partly`/`passed` (defaults `1`/`f`, `2`/`p`, `3`/`n`); an
  existing config with the old keys is rejected with an error naming the valid
  keys (`alix config --init` shows the new template). Pre-1.0, no shim. Progress
  files are unaffected — grades were never stored by name.
- **Breaking — the freeze format records provenance on the `% at:` line, not a
  note.** Freezing a workspace now writes `% origin:` (the live crate root) and
  appends each card's original location to its locator
  (`% at: 29.rs from src/caching.rs:46-66`), instead of smuggling it into a hidden
  `! from …` note that the display then stripped back out. Notes are the
  learner's again. **Existing frozen workspaces keep working for review and the
  exam, but the tutor can't ground them until re-frozen** (re-run
  `alix explore … --build`). Pre-1.0, so no compatibility shim. Card identities
  are unaffected (`% at:`/`% origin:`/notes are not hashed).
- **The review header no longer shows the stage ladder.** The always-on
  `new|s1|s2|…` stage histogram is gone from the review header (TUI and web) — it
  was noise; the per-stage breakdown stays in the end-of-session summary.
- **Returning to the picker keeps your place.** After a review/browse/walk/exam,
  the deck picker re-lands the cursor on the deck you just launched (rather than
  jumping to the top), so you can step straight to the next — often dependent —
  deck. Both the terminal picker and the browser picker (the top list and a
  workspace drill-in).
- **The Mastered window shows when a deck was mastered and how much is left to
  drill.** A mastered deck's badge now reads e.g. `mastered 🎉 · 3w ago · 8 to
  drill` — the time since it passed (the `deck_mastered` timestamp was already
  stored) and how many of its cards aren't yet retired (so a deck you *tested
  out* of without drilling shows the work remaining). Both TUI and web.
- **Web picker draws the dependency tree like the TUI.** A workspace's members
  now show `├─`/`└─`/`│` branch lines (muted) instead of plain indentation, and
  the 🕒 "nothing due" glyph moved from the start of the row to the end (with the
  status), so the left gutter is just tree + title. (The server already computed
  the prefix for `depth`; it's now sent to the browser.)
- **`alix explore` generates short, title-cased deck/trace titles.** The plan
  prompt asks for a terse title, but the model ignored it and appended the deck's
  contents after a colon — so the title is now **condensed deterministically in
  code** rather than left to the prompt: the enumeration is cut (at the first
  `:`/`;`/dash, or by a word cap when there's no separator), and the result is
  title-cased with code spans (`` `grpc` ``, `snake_case`, `CamelCase`,
  `ACRONYM`s) left intact. Workspace decks read as `The Crate Surface`, not `the
  crate surface: three-part Store/Execute/Inspect model, the three feature flags
  …`, and stop truncating in the picker. The condensed title also drives the file
  name, so slugs no longer trail a stray word from the cut enumeration.
- **Web trace walk: the leave button reads "Leave" and confirms an unfinished
  walk.** The hosted walk's return chip was "Decks"; it's now "Leave" (matching a
  fact-deck session), and leaving before the last checkpoint shows a "Leave the
  walk before finishing the path?" prompt (Enter leaves, Esc stays) — the same
  guard as review and exam. A finished walk still leaves immediately.
- **Web exam: leaving mid-answer asks to confirm.** Pressing Esc (or Quit) while
  answering now shows a "Quit the exam? Your answers won't be graded" prompt —
  Enter abandons it, Esc keeps going — so a stray Esc no longer throws away an
  in-progress exam, matching the review-session leave guard. (Other phases close
  immediately; the typed answer is preserved if you keep going.)
- **Reviewing a deck no longer pulls in its prerequisites' cards.** A review (in
  the TUI/CLI) now holds exactly the deck(s) you picked — `% requires:` decks are
  not auto-added "foundations-first" — matching what the web already did.
  Dependencies are about *order and gating* (the picker tree + the exam gate),
  not what a session contains. (Removed the `resolve_deck_order`/`dep_ranks`
  machinery; book + README updated.)
- **Breaking — a trace masters by passing its exam, not by finishing the walk.**
  Walking a trace is now the *drill*: completing the walk no longer masters it
  (the earlier "mastered once every checkpoint retires" behavior is gone). A
  fully-walked trace becomes **exam due**; passing the new trace exam — the
  compression (see Added) — is what masters it and unlocks its dependents, just
  like a fact deck. The ungraded walk-end "compress" step is removed (and its
  `/api/walk/compress` endpoint), and the progress store bumps to **v2** (an
  older alix now cleanly refuses a v2 store with an "upgrade alix" message rather
  than mis-reading the new deck-progress shape).
- **`% requires:` now gates the exam, not drilling.** You can review/drill any
  deck at any time, in any order — a prerequisite-locked deck is no longer
  blocked in the picker (it stays bright and startable; the lock is named
  explicitly when it's focused — the TUI footer says "🔒 Exam locked", the web
  shows its "Take exam" button disabled with a 🔒 — rather than a per-row lock
  glyph that read as "the deck is locked"). The dependency order applies to **exams**: to sit a sourced
  deck's exam you must have passed each *sourced* prerequisite's exam. A
  **source-less** prerequisite has no exam, so it never gates — its edge is
  informational in the dependency tree, seen *through* to the nearest sourced
  ancestor. (`is_locked` counts only sourced prereqs; both pickers and the
  exam-due review shortcut respect the new gate.)
- **`alix deck` is now a command group: `alix deck generate` + `alix deck
  augment`.** **Breaking:** `alix deck <source>` is now `alix deck generate
  <source>`.
- **Choice-mode offline distractors are shape-aware.** Number-like answers now
  only compete with the same shape (a 4-digit year vs other years, not a `1,5`
  ratio or a 2-digit count), so an obviously-wrong option no longer slips in.
- **Ask-Claude (web): Enter now inserts a newline and Shift+Enter sends.** The
  ask box is a multi-line textarea, so plain Enter composes freely and a
  deliberate Shift+Enter submits the question (the Send chip and placeholder
  show the hint). Previously Enter sent and Shift+Enter made the newline.
- **Web exam: Shift+Enter advances** to the next question (or submits, on the
  last), matching the ask box — Enter still inserts a newline so multi-line
  answers compose freely, and the Next/Submit button now shows the binding.

### Fixed
- **The picker labels a trace by its description, not its filename.** A trace row
  in the picker (web tree and TUI drill-in) showed the raw file stem — a clipped
  kebab slug like `08-how-a-workout-starts-logs-a` — even though the trace already
  carries a readable name in `% trace:`. It now labels the row from that
  description (`How a Workout Starts, Logs a Set, and Advances to the Next`),
  condensed to a label-sized head so a long `--build`/hand-written path-question
  doesn't overrun the row. Plain decks (a `% title:` or neither) are unaffected.
- **A trace `--grade` reply that isn't a real verdict now errors instead of being
  scored as a miss.** The per-hop grader expects the model to answer
  `NAILED`/`PARTLY`/`FAILED`; an unrecognized reply (a weaker model ignoring the
  instruction) used to silently fall through to a failing grade — fabricating a
  verdict the model never gave. It now surfaces an error and falls back to
  self-grading, so a correct prediction is never quietly marked wrong.
- **`alix explore --into --build` now actually freezes its `assets/`.** The
  generated `% source:` paths were silently doubled: when `--source` is a
  subdirectory (a crate) but the plan writes a scope relative to the project root
  above it (`crates/x/src/lib.rs`), the write-time join produced
  `…/crates/x/crates/x/src/lib.rs` — a path that doesn't exist. Every citation
  read failed, so the freeze step copied nothing and the workspace was left with
  no `assets/` **and no warning**. Generation now anchors the scope overlap-aware
  (the write-time twin of the `% at:` read fix), so the citations resolve and the
  excerpts freeze.
- **A multi-file `% source:` (`a.rs + b.rs`) now freezes every cited file.**
  Snapshotting treated the whole ` + `-joined line as one literal path, so a
  multi-file source froze nothing; it now splits the source exactly as the review
  path does (shared `SourceBase`), so freeze and review can't disagree.
- **A missing or stale `% source:` base fails with a clear message.** A directory
  `% source:` that no longer exists used to have the locator joined onto it,
  yielding a baffling `…/README.md/src/lib.rs` "no such file"; it now reports the
  real cause — the source base doesn't exist (the path is likely stale or wrong).
- **A cited deck that can't be frozen is reported, not swallowed.**
  `alix explore --build` now warns which deck's source couldn't be read instead
  of silently leaving an empty `assets/`.
- **A `% at:` locator written relative to a project root above `% source:` now
  resolves.** When a deck scopes `% source:` to a subdirectory or file (e.g.
  `…/crate/src/executor`) but writes its `% at:` paths from the crate root
  (`src/executor/local_vm.rs`), joining them doubled the overlap
  (`…/src/executor/src/executor/local_vm.rs`, "no such file"). Resolution now
  walks up the base directory's ancestors until the cited file is found.
- **Frozen-snapshot excerpts show the original file and line numbers.** A walk or
  fact card whose `% source:` is a frozen `assets/` snapshot showed the asset
  (`30.rs`, lines 1-N) instead of the real source; the cited excerpt now relabels
  to the original `caching.rs:106-120` (from the location recorded on its `% at:`
  line) — in the walk, the fact-card citation and the terminal walk.
- **A long (hand-crafted) deck title no longer reflows the header.** The
  review/browse/walk headers truncate an over-long title with an ellipsis instead
  of wrapping to a second line and growing the header's height.
- **No stray blinking caret across the web app.** The caret is suppressed on
  card/slide prose everywhere — review, browse, and the trace walk — appearing
  only inside a real text input or a source-code excerpt (e.g. with the browser's
  caret-browsing on).
- **Ask-Claude (web): the input re-focuses when a reply lands**, so you can type
  a follow-up immediately instead of clicking back into the box.
- **A trace/fact citation against a single-file `% source:` no longer doubles the
  path.** When `% source:` is one file, every `% at:` reads *that* file; a locator
  that repeats the path relative to a different root (e.g. the crate root,
  `% at: src/executor/env.rs:44-64` against `% source: …/src/executor/env.rs`)
  was joined onto the file's own directory, yielding
  `…/src/executor/src/executor/env.rs` ("no such file"). Both the walk reveal and
  `alix check` now share one `locator_path` resolver, so they can't disagree.
- **Opening a deck with nothing due no longer bumps it to the top of the recent
  list.** A review now records the deck as "recent" only when the session
  actually has cards to review (`!session.is_finished()`), so merely entering a
  fully-drilled / all-on-cooldown deck leaves the recent order untouched.
- **A fact card's `% at:` citation resolves against a multi-file `% source:`.** A
  deck whose `% source:` joins several files with ` + ` (the generator's format,
  e.g. `<crate>/README.md + src/lib.rs`) now reads each card's cited excerpt from
  the right file. Previously the whole joined string was treated as one directory
  and the `% at:` file appended to it, so the reveal showed `cannot read the
  source …/README.md + src/lib.rs/README.md`. `SourceBase::for_deck` now bases on
  the first source file (matching `source_paths`); with several files a bare-line
  locator is rejected (ambiguous) rather than silently reading the first.

## [0.1.0] - 2026-06-23

### Changed
- **Renamed the project `flash` → `alix`.** The binary, the crate, the workspace
  manifest (`flash.toml` → `alix.toml`), and the data directory
  (`~/.local/share/flash` → `~/.local/share/alix`) all move to the new name.
  Existing progress is **auto-adopted on first run**: if the legacy `flash` data
  dir exists and the new one doesn't, it's moved across, so your history carries
  over untouched. (The cards are still "flashcards" — only the tool's name
  changed.)

### Added
- **Fact cards can cite their source (`% at:`), shown on reveal.** A plain fact
  card may now carry a `% at: file:lines` locator into its deck's `% source:`
  (the same form a trace checkpoint uses — `file:lines`, or just `lines` for a
  single-file source). On reveal a `</>` marker appears on the answer; in the
  web you **click the answer** (or press `s`) to swap it for the line-numbered
  source excerpt and back, and in the terminal you press **`s`** — one view at a
  time, so the card stays compact. The excerpt is read live, so a moved/missing
  source shows "source unavailable" rather than a stale quote, and `% at:` is not
  part of the card's identity hash (adding it never resets progress). Reuses the
  trace walk's excerpt machinery via a shared `trace::SourceBase`/`excerpt_at`.
  The deck **generator writes these citations for you** — `alix deck` on a local
  source and `alix explore --build` add a `% at:` to each fact that maps to
  specific lines — and **`alix check` validates** a fact deck's citations,
  warning about one that no longer resolves (a moved or shrunk file). A workspace
  built with **`alix explore --into --build` freezes** every cited deck's
  excerpts into its `assets/` (fact decks now, not just traces), so the citations
  don't drift and the workspace travels without the upstream source; a frozen
  fact deck's `% source:` then points at the excerpts, so its exam grades against
  them. (Snippet names are workspace-unique now, so multiple frozen decks no
  longer collide in `assets/`.)
- **`% unlock-stage: N` — unlock a deck before its cards retire.** A `% source:`
  deck becomes *exam due* (its exam opens), and a source-less deck *finished*
  (its dependents unlock), once **every card reaches Leitner stage N** — without
  retiring them, so they keep drilling to the top stage; the directive only lowers
  the unlock bar. Default (unset) keeps the old gate: every card retired at the top
  stage. Settable per deck, in a workspace `alix.toml`
  `[defaults]`, or via `alix explore --into --unlock-stage <1–5>`. Generalizes
  the completion gate (`Deck::state`).
- **Browse a deck from the session-end summary** (terminal). When a deck turns
  *exam due* at the end of a review, the summary now offers `b` to **browse** it
  (a read-only walk through its cards) right next to `x` to sit the exam — useful
  for a last skim before the exam. Both the offer line and the footer show the
  keys. (`App` returns an `AfterReview::{Exam,Browse}` for `main` to launch.)
- **The progress store is now version-checked.** A `progress.json` written by a
  newer alix is refused on open with a clear "upgrade alix" message instead of
  being silently rewritten at the old version (which could drop data the newer
  format added); the file on disk is left untouched. A store with no `version`
  field still loads as the original format. This lays the groundwork for safe
  schema migrations.
- **The ask-Claude tutor can read the card's source to verify its answer
  (opt-in).** A new `[ask] source_access` flag (off by default) lets the tutor
  run with `Read`/`Glob`/`Grep` and its working directory at the deck's
  `% source:` **project root** (resolved up to the nearest `Cargo.toml`/`.git`/
  …), and instructs it to check the real files before answering instead of
  relying on memory — so a question about a generated deck is grounded in the
  same source the deck was built from. Off by default because it grants the
  (possibly LAN-served) tutor file-read access. A **workspace can override it**
  per-folder with `source_access` in its `alix.toml` (so you can enable it for
  one trusted crate without turning it on globally). The web ask panel also now
  shows **which model and effort** are answering (`model: … · effort: …`) — a
  reminder that the tutor uses the CLI default unless `[ask]` pins a stronger one.
- **`alix explore --title` shapes the scaffolded workspace; the goal becomes its
  description.** `alix explore --into <dir>` now takes an optional `--title` for
  the workspace's `alix.toml` `title` (omitted, the folder name is used). It also
  writes the `--goal` as a new `alix.toml` **`description`** field instead of an
  ignored `goal` key; a
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
  opens the focused row. Its header is just `alix`; rows are grouped into
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
  both in the top-level drill-in and `alix workspace <dir>`. An explicit
  `alix review <trace.txt>` still flattens it (honoring the literal command). The
  multi-select machinery is retained in the code but unused for now. The web picker
  follows in a later phase.
- **Per-workspace progress store** — a deck inside a workspace (a folder with a
  `alix.toml`) now tracks its progress in a **`progress.json` inside that
  workspace**, not the one global `~/.local/share/alix/progress.json`. So a
  workspace is a self-contained, portable unit (decks + `assets/` snapshots +
  progress in one folder), its history is isolated, and same-named decks in
  different workspaces no longer collide in one store. Loose decks (and plain
  folders without a manifest) keep the global store; `--store <path>` overrides
  either; a workspace can redirect its store with a `store = "..."` line in the
  `alix.toml`. Resolution: `--store` > the single workspace all the session's
  decks share > global. Applies across the CLI/TUI (`review`, `trace`, `exam`,
  `browse`, `stats`/`list`, `reset`, `alix workspace`); the web frontend follows
  with the picker revamp. (No migration — workspace decks start fresh in the
  workspace store; existing global progress for them is left in place.)
- **Trace source snapshots** — creating a workspace by exploring a source
  (`alix explore --into <dir> --build`) now **freezes the cited excerpts** into
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
- **`alix import <file.tsv>`** — import an Anki "Notes in Plain Text" export
  (tab-separated `front<TAB>back`) into an alix deck, no Claude needed. It skips
  Anki's `#`-prefixed header lines, turns `<br>` tags into separate answer
  lines, decodes the common HTML entities, and backslash-escapes a back line
  that would otherwise read as a `%` comment or `!` note; rows missing a side
  are dropped. The result is validated and written to `~/decks/` (`-o`/`--print`/
  `--force`, like `alix deck`). Conversion lives in the lib
  (`import::tsv_to_deck`).
- **`alix check` now validates trace `% at:` locators.** A trace deck is linted
  like any other: `check` resolves each checkpoint's locator against its
  `% source:` and warns (advisory, non-fatal) about any that name a missing file,
  run past the end of the file, give bare line numbers without a single-file
  source, or are absent — a quick "does this excerpt still exist?" structural
  check that catches a moved or trimmed source before a walk hits it. (Frozen
  snapshots are validated the same way.) It also prints the deck's `% trace:`
  description. Logic in the lib (`trace::Trace::lint_locators`).
- **`alix deck <source>`** (renamed from `alix generate`, which no longer
  exists as an alias) — generates a facts deck with Claude from a **web page URL or a
  local file/directory path**, mirroring `alix trace`. A URL is fetched with
  WebFetch and the deck starts with a `% link:`; a local source is explored
  read-only with `Read`/`Glob`/`Grep` at its root and the deck starts with a
  `% source:` (so `alix exam` can grade against it). This gives a facts-deck stub
  from `alix explore --into` a manual fill path (point `alix deck` at its
  `% source:`).
- **Traces (`alix trace`, experimental)** — a guided predict-and-verify walk
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
  sentences. Self-judged and offline (no model call) by default; **`alix trace
  --grade`** instead has Claude judge each typed prediction against the key points
  and return the verdict + one line of feedback (a model call per hop, run at the
  lightweight `[ask]` tier — not the heavy build defaults below). **`alix
  trace <deck> --serve`** walks it in the **web frontend** (the same
  frontend-agnostic `Walk` state machine the terminal uses): a left **path rail**
  whose nodes color in by Got / Partial / Missed, each checkpoint's source shown
  in a line-numbered excerpt, and `--serve --grade` running the live grade on a
  background thread while the page polls; `--port`/`--lan` work as in `review`.
  `alix trace <deck> --map`
  prints the path without quizzing; the generic AI exam refuses a trace (its
  verification is the walk itself). See `docs/examples/rust-ownership/ownership-move.txt`.
  **`alix trace --build <deck>`** discovers the path for you: declare just the
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
  **`alix trace --suggest <source>`** recons a source (read-only, one pass) and
  prints a ranked menu of candidate traces to author — a path-question, a spine
  sketch, and a suggested scope each, no checkpoints — closing the "what's worth
  tracing?" gap before `--build`.
- **`alix explore <source>` (experimental)** — goal-driven exploration:
  prints an ordered **learning plan** toward a `--goal` (default
  "understand the whole source"), the fact **decks** and **traces** worth
  authoring. Each item is tagged `[trace]`/`[deck]` (chosen by shape — edges
  become traces, node-shaped fact tables become decks), carries its `% requires:`
  prerequisites (the list is a valid topological order, foundations first), and a
  `% source:` scope. The goal scopes coverage — a broad goal spans every
  subsystem, a narrow one collapses to its slice (and traces it deeper). By
  default read-only (prints the plan); **`--into <dir>`** materializes it into a
  **workspace** folder — an `alix.toml` (the goal) plus a stub deck/trace file per
  item, `% requires:`-wired in dependency order with absolute `% source:` paths,
  ready to `alix trace --build` / author (refuses a non-empty dir unless
  `--force`). Add **`--build`** to fill them: `alix explore … --into <dir>
  --build` explores the source **once**, then resumes that same CLI session to
  write the full content of every item — predict-verify checkpoints for traces,
  fact cards for decks — so the workspace is review-ready in one command, with the
  items coherent (written from one understanding) and facts decks filled too.
  **`--walk`** instead builds an **explore walk** — a predict-verify
  trace over the source's *shape* (what it is → its domain nouns → entry point →
  spine → the first paths worth tracing), each hop revealing real structural
  evidence (the manifest, the module list, the entry enum). It's written to a file
  (`-o`, default `explore.txt`) and walked immediately, reusing the `alix trace`
  walk; re-walk later with `alix trace <file>`.
- **Workspaces** — a folder of decks reviewed together with shared directives.
  A folder is a **workspace** when it has an `alix.toml` manifest (a scoped
  `config.toml`) setting a `title` and a `[defaults]` table of directives that
  fill in what each deck leaves unset (precedence CLI > card > deck > workspace >
  default); a folder of decks *without* a manifest is a plain **folder** — still
  reviewable, but not a workspace. Both appear as their own rows in the picker
  (terminal and web, labeled "workspace" vs "folder") and drill into their decks
  (review all, or tick a subset); `alix review`/`browse <folder>` reviews the
  whole cluster. **`alix workspace <dir>`** opens a workspace into its own picker
  and routes each member to the right thing — a **facts deck** → review, a **trace
  deck** → predict-verify walk — returning to the picker when done. Great for
  clusters like a vocabulary set that should all be `direction = "both"` without
  repeating it per file.
- `% title:` deck directive (also usable in a `workspace.alix` manifest): a
  display name shown in the picker, session header, `alix list` and `alix stats`
  instead of the file name. Display-only and never part of card identity.
- **`alix exam <deck>`** — the AI exam, which *verifies understanding* and
  gates progression (rung 3 of the AI-exam direction). A deck declares its
  ground truth with `% source: <url-or-file>` (repeatable); the exam asks Claude
  for fresh open questions generated **from that source** (never from the cards,
  which would be circular), reads your typed answers, and grades them
  Pass/Partial/Fail against per-question rubric points. Passing marks the deck
  **mastered**, which is what now unlocks dependent decks — drilling a `% source:`
  deck to the top stage leaves it *exam due* (a new deck state, shown in the
  picker and `alix stats`) rather than finished; source-less decks keep the
  mechanical "finished = drilled" unlock. On a fail, the missed concepts can be
  turned into remediation cards appended to the deck — the card type is chosen
  per gap (cloze/plain for a missed fact, `% mode: explain` for a missed
  concept), and overlapping gaps are consolidated into a single card — then
  re-drill, re-sit. **Grading strictness is per deck** —
  `% strictness: strict | balanced | lenient` (or `alix exam --strictness`, or
  the `[exam]` default) — because some material needs every point recalled while
  other is about grasping the idea: `strict` treats an omitted rubric point as a
  gap, `balanced` (default) judges understanding and forgives terse phrasing,
  `lenient` only flags clearly wrong answers (orthogonal to `pass_threshold`,
  which sets how many answers must pass). New `[exam]` config section (`model`,
  `timeout_secs`, `num_questions`, `pass_threshold`, `strictness`, `extra`);
  reuses the `[ask]` command/permission/tools (WebFetch reads a source URL).
  `alix reset` of a deck also clears its mastered state. A URL `% source:` also
  doubles as an ask-Claude reference link (no duplicate `% link:` needed); a
  `% link:` never becomes an exam source.
  The exam is **fully interactive in both frontends** (rung 3b): answer one
  question at a time (Back/Next), then see a per-question breakdown — `alix exam`
  and `alix serve` share one engine (`exam::Sitting`) that runs Claude on a
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
- Deck completion states and unlocks. Each deck has a state derived from its
  cards' stages — not started / started / finished (all cards at the top stage)
  — shown in the deck picker (terminal and web) and `alix stats`. A deck is
  **locked** while any of its `% requires:` prerequisites isn't finished
  (finishing a foundation unlocks what builds on it); locked decks are dimmed
  with a 🔒 but stay selectable (advisory). Derived live from progress, with no
  new directive or storage.
- Repeated `TAB` in typing mode progressively reveals the answer: each press
  uncovers two more characters until the line is fully shown (still counts the
  card as failed); typing or deleting resets the reveal.
- In-browser deck selection: `alix --serve` (and `alix browse --serve`) with no
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
  directive (per card or deck-wide) controls this explicitly; `alix check` warns
  about missing image files. `/img/<key>` URLs are opaque hashes of registered
  deck paths, so the server never joins request input to a filesystem path.
- `alix reset` clears stored progress: for one or more decks, a single card
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
  button (and the `x` key); `alix exam <deck>` does the same from the terminal.
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
  (was advisory), but stays fully browsable (`alix browse` ignores locking) and
  resettable; the `alix reset` / `alix deps` pickers keep their plain
  multi-select. The shared badge / lock / dependency-tree logic now lives in the
  library (`picker::deck_status` and the exposed dependency-forest helpers),
  consumed by both frontends. A **trace** picked from the in-browser picker now
  **walks** (predict → verify, just like the terminal), hosted by the review
  server at `/walk`; a **Back to decks** (or `Esc`) returns to the picker.
- **A card that reaches the top Leitner stage now retires** (rests, no longer
  scheduled until `alix reset`) instead of recurring at the stage-5 weekly
  cooldown, so a *finished* deck stays finished.
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
- `alix check` no longer fails on warnings: duplicate-answer warnings are
  advisory, so it exits non-zero only when a deck won't parse, and prints a
  `N error(s), M warning(s)` summary.
- Web review now shows the expected answer whenever a typed line differed —
  including a fuzzy pass within tolerance — matching the TUI, so typos aren't
  reinforced.

### Fixed
- **Generated decks now put a blank line between cards.** `alix deck`'s output
  cleaner (`generate::clean_output`) inserts a blank line before each card front
  (`#`) after the first, so a generated/`--review`ed deck is readable instead of
  cards running together. The first card stays attached to its `%` header, and an
  already-separated deck is left untouched.
- **A note saved from the ask tutor shows on the card right away.** Saving a
  note appended it to the deck file but left the in-memory card unchanged, so the
  new note only appeared after the deck was re-read (a later session). The just-
  saved lines are now mirrored onto the in-memory card (`Card::append_note`) — no
  deck re-read — so returning to the card shows the note immediately, on both the
  web and the terminal (the web previously never reflected it; the terminal only
  updated the ask view's recap, not the card on return). On the web, closing the
  ask panel now re-pulls the card state (keeping the reveal position) so the
  saved note appears on return instead of only after a manual page reload.
- **The web ask panel shows the card above the conversation, matching the
  terminal.** The card under discussion (its front + answer) now sits at the top
  as the reference, with Claude's conversation flowing below it (answer under
  question), instead of the card being tucked beneath the conversation — so the
  question you're studying reads above the answer, the same order the TUI already
  used. The card and conversation now share one scroll region and the card
  **sticks to the top**, staying in view as a long conversation scrolls under it.
- **The grounded tutor no longer breaks the conversation with "No conversation
  found with session ID".** Claude scopes its conversation history by working
  directory, but the grounded tutor (`[ask] source_access`) runs each card's
  questions with the working directory set to that card's `% source:` root. A
  follow-up `--resume` that ran in a *different* directory than the
  `--session-id` that created the session — moving between cards grounded in
  different roots, switching a grounded and an ungrounded deck, or the
  "save note" condense (which ran ungrounded) after a grounded question — landed
  in the wrong project and failed. The CLI session is now **cwd-aware**
  (`CliSession::args_in`): a working-directory change starts a fresh
  conversation there (a clean first prompt) instead of a doomed resume, and a
  card's condense uses the **same grounding** as its questions so the directory
  stays stable. Same-directory follow-ups still resume as before.
- **Exam remediation is faster, can't fail silently, and shows progress.** Three
  problems when "Add remediation cards" was slow or produced nothing: (1) the
  remediation call inherited the tutor's `WebFetch`/`WebSearch` tools and could
  wander off researching the gaps — it now runs tool-free (it only needs the gap
  list), so it's a quick, deterministic text-generation call; (2) if the model
  replied in prose instead of cards, the prose was appended to the deck as a
  bogus "card" (so "no new cards" appeared on re-drill) — the reply must now
  contain at least one `#` card front or remediation fails with a clear message
  instead; (3) a failed call (timeout, empty/unparseable reply, write error) was
  easy to miss — both the web and terminal exam views now show a **prominent
  error banner at the top** (web offers "Try remediation again"; the terminal
  scrolls it back into view), and every in-flight Claude call (generating,
  grading, remediating) shows a **live "Claude is working… Ns" counter** so a
  long call no longer looks frozen.
- **A `% source:` that names several files no longer breaks the exam.** The deck
  generator sometimes writes a multi-file source as `<root>/README.md + src/lib.rs`
  (first a full path, the rest relative to it). The exam read the whole string as
  one path and failed with "cannot read source file …"; sources are now split on
  ` + `, each part resolved (relative parts anchored to the first file's
  directory) and read, with unreadable parts skipped rather than aborting the
  exam. The grounded tutor's project-root resolution handles the same format.
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

## [flash 0.1.0] - 2026-06-16

First release of `alix`: a terminal spaced-repetition flashcard trainer with
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
- `alix generate <url>` builds a deck from a web page via Claude (WebFetch),
  with a prompt that spreads cards across four layers of understanding, uses
  cloze and notes, and self-reviews for redundancy; `--review` adds a second
  refinement pass. Output is validated and saved (or `--print`ed); `--cards`
  and `[generate]` config tune it. Claude only returns text — alix writes the
  file — so no extra tool permissions are needed.

### Other commands
- `alix browse` — read-only walk through cards (no grading, no writes).
- `alix deps` (alias `require`) — edit a deck's prerequisites with a checkbox
  picker.
- `alix stats`, `alix list`, `alix check`, `alix config`.
- Startup **deck picker** (recent decks + the decks directory) when run with no
  arguments.

### Configuration
- `~/.config/alix/config.toml` with `[keys]`, `[browse]`, `[ask]`,
  `[generate]` sections and `decks_dir`. `alix config --init` writes a
  self-documenting template (every option commented at its default);
  `alix config` prints the active settings. Key bindings are rebindable.

### Storage
- Card identity is a stable XxHash64 over the deck file name plus the back
  lines (a test pins the value so upgrades never orphan progress). Progress is
  stored at `~/.local/share/alix/progress.json`, created on first use.

### Desktop
- `assets/install-desktop.sh` installs an icon, launcher, and `.desktop` entry.
