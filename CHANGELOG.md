# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Personal notes and cards now live in a file of their own beside the deck
  (`spanish.md` gets `spanish.personal.md`), leaving the authored deck
  byte-identical. Tutor notes, tutor-made cards, and exam remediation cards
  all land there; a `for:` frontmatter key names the deck the file belongs to,
  a `<!-- note: <card-id> -->` marker followed by `>` lines adds notes to one
  of its cards, and an ordinary card block adds a card of your own. A card
  closes with its `<!-- id: -->` line as it does in a deck, while a note's
  marker opens its block, because a note has to name the card it belongs to
  before anything below it means anything. Nothing in the file copies the
  deck, so nothing in it can go stale.
  A personal file is never listed as a deck, `alix deck init` refuses one
  rather than stamping it with an `id:` of its own, and `alix share` leaves
  it at home in both directions: it is never bundled, and an arriving bundle
  cannot overwrite yours.

- The example decks gained one for cloze inside a formula, showing that
  `\blank{...}` works within `$...$` and `$$...$$`.

- `alix doctor` warns when a cloze hole that stays typed holds a LaTeX
  command, because the hole's content is the expected answer and
  `\blank{\pm}` then asks for the spelling of `\pm`. A hole inside a
  formula is silent: it is drawn rather than typed.

- Deck authoring has a written rule for which card shape suits which
  material, and one worked example per shape. `docs/card-shapes.md`
  marks each row structural (the material has a property the shape
  exploits, so any other shape wastes it) or judgement (more than one
  shape is defensible), the manual includes it, and
  `docs/examples/shapes/` demonstrates every row. Each example is held to
  the shape it advertises, so a deck that stops producing what it
  documents fails the suite.

- Each running web-server instance now keeps an always-on local diagnostic log
  outside the decks directory. The current and one rolled file are capped at
  5 MiB each by default, contain operational timings and minted IDs but no
  learning content, names, titles, or paths, and are never uploaded or read
  back by Alix. `alix doctor` and the web Doctor sheet name the profile's
  readable, collision-safe log path; `[log]` configures its cap and verbosity.

- The session summary places the sitting against the whole deck: the
  "introduced" row now also reads how many of the deck's cards have ever
  been met, out of how many it holds. `met_total` and `deck_total` carry it
  on the JSON API.

- Card tables: a GitHub-flavored pipe table in a deck is a compact card
  source, one card per row (front | back | optional note). The header row
  is shown as the card's context, and a `##` heading directly above the
  table titles it and leads that context. Each row carries a minted
  six-character stamp at its end and the table one container id line, so
  sorting, inserting, and editing rows preserves review history.

- Table cards draw their Recognize options from their own column, so a
  vocabulary table needs no AI augmentation and no authored options
  (both still take precedence where present). The new `sampling: on|off`
  key turns that off or on again, in the frontmatter for a whole deck, in
  a workspace's defaults, or in one table's directive comments, which win
  over the deck in either direction. `alix doctor` reports a `sampling:`
  key that can affect nothing.

- Tables accept the card directives (`direction`, `reveal`, `input`,
  `sampling`) between the table and its id line. Anything the format
  cannot hold (a fourth column, cloze markers or images in cells, stray
  content after the table) is a loud parse error rather than a guess.

- The adult web picker can remove a focused loose deck, workspace member, or
  whole workspace from its secondary menu. A type-the-name sheet previews the
  irreversible stakes and Alix-owned artifacts first; source files Alix does
  not own remain in place, active sessions block removal, recent and retained
  state are invalidated, and a partial failure names completed and failed
  artifacts with an `alix doctor` recovery step. The JSON API exposes the same
  preview and removal contract for other clients.

- Deck lifecycle completes with removal and restore. `alix deck remove
  <deck>` deletes a deck and everything that is its alone (file, review
  history, frozen assets, augmentations, and any `.bak` backups) after one
  confirmation that states the stakes: cards with progress, the
  reviewed-since date, the exact file list. Total by design: nothing is
  backed up and it cannot be undone. `alix deck restore <deck>` is the
  counterpart for overwrites: it swaps a deck with its `.bak` backups and
  is its own inverse, so restoring again swaps back. Overwrites (a forced
  import, trace or workspace regeneration) now back up the full trio (deck
  file, review history, augmentations), where they previously kept only
  the deck text, so a restore brings the history back too. `alix doctor`
  counts accumulated backups and `--remove-backup-files` deletes them
  after a confirmation.

### Changed

- `alix doctor` now reports a frozen source excerpt whose lines moved, and
  `--repair-source-locators` rebases it. Previously a moved excerpt was found
  anywhere in the file and passed silently, so a card could point a reader at
  unrelated code while doctor said nothing; on the maintainer's own decks that
  was true of 29 citations. A rebase corrects the `at:` line numbers only: the
  frozen bytes and their fingerprint are untouched, so no card gains a claim it
  did not already have. An excerpt whose content actually changed is still
  reported and never repaired automatically.

- **Breaking:** remediation and tutor cards are no longer stored inside the
  progress file. They are Markdown blocks in the deck's personal file, so you
  can read, edit, and delete them yourself. A progress file written by an
  earlier version fails to load; there is no conversion (pre-1.0).

- **Breaking:** the "Promote to deck" review action is gone, with its
  `/api/promote` endpoint, its `StateDto.promotable` field, and its
  `[keys.review] promote` binding. A minted card is already a Markdown block
  in a file you own, so there is nothing left to promote it into. The review
  badge no longer carries a `remediation · ` prefix.

- `alix reset` clears a personal card's schedule and leaves the personal file
  alone, the same treatment the authored deck gets.

- Generated decks now pick a card shape from the material rather than
  choosing between plain and cloze. `alix generate` can produce card
  tables, line-by-line reveals, both-direction cards, and draw cards,
  which it previously had no way to emit, so a vocabulary source becomes
  a table and a procedure becomes an ordered reveal. Its prompt is built
  from the same rule the manual publishes, so the two cannot drift.
  Setting `card_style` still pins one shape for a whole run.

- `make web-debug` now runs the ordinary server with `--log http`, which keeps
  verbose request timings in the local log and mirrors them to stderr.

- A new card's badge now names its interaction too, not just "new":
  `new · choice` when it offers options, `new · draw` on a sketch card,
  `new · reveal` otherwise. It still never names the graded check the
  card's schedule will use once the card has been met.

- The session summary no longer starts the next card by itself. When a
  settle gap passes while the summary is open it says it is ready and arms
  Continue, leaving the choice to the learner instead of dropping them into
  a graded card.

- `alix deck init` writes the prepended frontmatter block closed
  directly after the `id:` line, with the blank line following the
  block instead of sitting inside it.

- Old deck formats are no longer recognized anywhere. The dedicated
  retired-key errors (`alix-id:`, `origin:`), the doctor's "un-converted"
  classification of decks and state documents, and the workspace manifest's
  `origin` rejection are gone, along with every "deck conversion tool"
  suggestion (no such tool ships). An old artifact now fails like any other
  invalid input: unknown keys lint, ids and locators fail the current
  grammar, unreadable documents error, and nothing suggests a remedy. The
  dedicated errors for a `" + "`-joined `source:` value (deck frontmatter
  and workspace manifest) and the stray-aggregate-state-file warning are
  gone on the same ruling; a joined source is an ordinary path that fails
  to resolve. The retired `%`-prefixed directive spelling is likewise gone
  from the config template `alix config --init` writes and from the
  backend source-reachability errors, which now say plain `source:`. The
  dead `Reveal::Cloze` enum variant is deleted outright, and the cloze
  image hash preimage now uses the current `![alt](src)` spelling (stored
  hole fingerprints regenerate via a store-internal version bump; review
  history is untouched).

- The retired `reveal = "cloze"` value is rejected in a workspace
  manifest's `[defaults]` too, the one scope that still accepted it, and
  the `workspace init` template stops advertising it: `\blank{...}` holes
  are the only cloze trigger, at every scope.

- `generate`'s `max_cards` is now a soft ceiling with a default of 100
  (was a hard-worded 30): the prompt aims for the configured count, and a
  generation that comes back larger is kept in full with a warning
  instead of being constrained.

### Fixed

- `alix deck init` refuses a deck whose code fence never closes, instead of
  writing an id line inside the fence. The card then still read as unstamped,
  so every later stamp appended another id and the file grew one each time.
  `alix doctor` already named the unclosed fence and its line.

- Stamping a deck that ends in a card table with no final newline put the
  row's stamp on a line of its own instead of in the row. The row then read
  as unstamped, so every later stamp minted it a new one and the row's card
  id changed with it, detaching that row's review history. Found by the new
  stamper fuzzing.

- A draw card is answerable on the mobile client. It asked for typed text
  before, including for a formula's piece, because the client dropped the
  card's input kind on the way in from the core. Pen, eraser, undo, and
  clear; a stylus locks out touch so a resting palm cannot draw; the
  sketch survives a rotation and stays on screen beside the answer.

- A cloze hole cut out of a formula is now sketched rather than typed,
  because the hole's content is the expected answer and a formula's piece
  has no keyboard spelling. An `input:` written on the card or the deck
  still wins: the rule only fills in where nothing was authored.

- A cloze card whose hidden hole sits directly against the next token
  renders again. `\blank{n}x^{...}` failed the whole formula, because the
  mark standing in for a hidden hole was a bare control sequence and
  welded onto whatever followed it.

- A cloze hole cut out of a formula now reveals as that formula's piece
  rather than as its source: `$x = -b \blank{\pm} \sqrt{d}$` reveals a
  typeset ± where it used to show the characters `\pm`. What you type at
  Reconstruct is unchanged, so blank something typable.

- The example images now show the card as the reader should read it. A
  choice example is photographed answered, because the cursor rests on
  option 1 and a picture of that reads as if option 1 were the answer:
  the table example appeared to pair "to advocate" with "widerlegen".
  The draw example is sketched on, since an empty canvas photographs as
  an empty box. Every image also waited on a fixed pause that expired
  before the header logo finished forming, so all of them were captured
  mid-animation.

- The summary's "Next due in N min" counts down while the page stays open
  instead of freezing at the value it had when the sitting ended.

- A new card whose answer was revealed before the "Seen" acknowledgment no
  longer returns as a new card for the rest of the sitting. It came back
  after every acquire cooldown offering only "Seen" again, so it could
  never reach a graded review. Cards whose Recognize options need no AI
  (table rows, authored option lists) hit this on every acquire, because
  picking an option is what reveals the answer.

- A filesystem error after an atomic rename no longer strands progress or
  augmentation behind a stale in-memory revision: the error still surfaces,
  while retry bookkeeping follows the replacement that already committed.
  Retrying a virtual-card promotion after its deck write also recognizes the
  stable card ID instead of appending a duplicate, preserving its review
  history across the deck/progress failure boundary.

- The kids app behaves on touch screens: a long-press no longer opens the
  browser's context menu, page text cannot be selected or grow a blinking
  insertion cursor, taps skip the double-tap-zoom delay, and the answer
  and rating buttons grow to a child's finger size on coarse-pointer
  devices (the tutor's text box keeps selection and its menu).

- A kids card with a picture asks the question above it, matching the
  authored front order and the adult app, instead of hoisting the image
  over the question.

- A Recognize sitting no longer ends in "you reviewed 0 cards": the
  session tallies recognize answers (right / almost / missed) separately
  from FSRS reviews, the review state (`StateDto`) carries `recognized`,
  `recognize_partly`, and `recognize_missed`, and both web summaries show
  every kind of work done (introduced, recognized, reviewed). A sitting
  with no work at all no longer celebrates.

- The `alix generate --review` and `alix deck augment` help texts name
  the AI backend neutrally instead of hardcoding Claude, the augment
  progress line names the configured backend, and `alix receive`'s help
  mentions workspaces alongside decks and folders.

- Multiple-choice option text was near-invisible on the gruvbox-light and
  catppuccin-latte themes: ten themes omitted the text/faint/accent-ink/
  brand-text tokens and silently inherited the dark default's light gray.
  Every theme now defines the full token set (dark themes keep their
  current rendering), pinned by a token-parity test.

- The augment dialog's footer no longer places the destructive "Remove
  all" between "Generate selected" and "Close"; it now sits first, so a
  reach for Close cannot land on it.

- Saving a tutor note onto a stamped card appended it after the card's
  closing `<!-- id -->` marker, tripping doctor's misplaced-marker
  warning on every noted card. Notes now land before the card's trailing
  comment markers, keeping the id line last.

## [0.7.0] - 2026-08-02

### Added

- Desktop release downloads are now verifiable: every release asset uploads
  with a `.sha256` checksum beside it, and releases are gated on a clean
  RustSec advisory scan of both lockfiles (`make audit`, backstopped by a
  nightly advisory-drift workflow between releases).

- Revealing a new card's answer now counts as the encounter: leave the
  session right there, without pressing Seen, and the card still will not
  re-introduce as new next time (it cools and returns as a regular review).
  Leaving before the reveal keeps the card new. Both web clients report the
  first reveal (and the first pick on a new choice card) via the new
  `POST /api/reveal`, which records the engagement without advancing the
  session. The drained-Recognize summary's Augment chip also gained its `a`
  key binding, matching the picker.

- An exhausted Recognize sitting now says what it was hiding instead of
  "Nothing due — come back later": a deck whose pick-capable cards are all
  recognized reports how many cards wait at Recall and how many have no
  choices yet (`StateDto.recognize_gap`), and the adult summary points at
  both exits, with "Continue at Recall" (Enter) and "Augment" actions.
  Previously a deck with a
  few authored choice cards defaulted to Recognize, served only those, and
  then looked permanently empty while the rest of the deck was untouched.
  The done summary also no longer prints zero-valued stat rows, an
  instant-empty select no longer announces "session complete" for a session
  that never happened, and "N still due" beside a disabled Continue now says
  the cards are cooling and when one opens (`next_due_ms` carries the
  acquire-floor instant, previously absent at Recognize).

- Decks carry a `format-version:` in frontmatter, written above `id:`. `alix
  deck init`
  writes it, it stays `1` before 1.0, and alix refuses any other number with a
  message telling you to upgrade rather than guessing at a format it does not
  know. This is the same detect-and-refuse guard the progress documents already
  had, which decks lacked: a deck from a newer alix previously misparsed into
  confusing lint noise.

- Frontmatter now reads `authors`, `license`, `tags`, and `created-at`.
  `authors` and `tags` accept one value or a list, `license` and `created-at`
  are single strings (an SPDX identifier and an ISO 8601 date by convention).
  They were accepted but discarded before. alix stores and
  never rewrites them.

- `CardDto` now carries the card's `id` on the wire (`card-<token>`, or the
  `-N` / `-r` sub-id for a cloze hole or reversed twin), the same spelling
  `CreateCardResp` returns. Clients could not previously tell whether a served
  card had actually changed except by comparing rendered text. Null for a card
  with no id marker yet.

- `alix generate` now reports calm, live progress from structured Claude and
  Codex events while keeping partial deck text private until validation
  succeeds. Deck-drafting calls have a one-hour absolute safety limit.
  Generation paths using structured events also have a separate five-minute
  inactivity limit that resets whenever the agent emits real activity and can
  be disabled with `idle_timeout_secs = 0`. For Gemini, Copilot, and
  unstructured wrappers, the same setting is a nonrenewing absolute fallback:
  by default they retain the prior five-minute ceiling rather than waiting the
  full hour when wedged. They report generic activity and forward bounded
  stderr diagnostics.

- `alix generate` now accepts `--language`, `--audience`, and `--card-style`
  (`mixed`, `plain`, `cloze`, or `authored-choices`), and applies `--goal` to
  single decks as well as directory plans. The same controls reach generated
  workspaces; card style governs facts-deck items while trace items retain
  their checkpoint shape. Explicit styles are parsed and checked before a deck
  is written, and the optional review pass preserves the requested contract.

- A standalone `orchestrate` research CLI runs Claude Code and Codex against one
  frozen spec in isolated worktrees. Symmetric differential review accepts only
  mechanically reproduced, user-relevant defects; asymmetric runs pair an
  implementation with an independent property suite. Atomic state, raw
  transcripts, live durable heartbeats, concurrent independent agent calls,
  bounded fixes, serialized mutation gates, correctness-gated scoring, and
  test-first landing make every run resumable and inspectable.

- The kids client and the mobile app now show the persistent "progress isn't
  saving" banner the adult web client already had. On mobile a failed
  per-grade save no longer aborts the grade behind the scenes: the review or
  walk continues in memory, the banner names the failure, and the next
  successful save carries every earlier grade (the finished-review payload
  gains an additive `save_error`).

- The picker's focus drawer shows a nested progress funnel pinned to its
  top-right: `N cards` always, then `s seen`, `k learned`, and `r retired`
  appended as each count becomes non-zero, so a fresh deck reads as a plain "N
  cards" and a worked deck fills in. The `/api/deck-drawer` payload gains the
  additive `total`, `seen`, `graduated` (labelled "learned" in the drawer), and
  `retired` counts, which nest `retired <= graduated <= seen <= total`; the
  per-card `heatmap` lists only stamped cards and cannot stand in for them.

- The empty "Nothing due." session screen now shows one quiet line saying when
  the next card is due (for example "Next due in 4 min." during an acquire
  cooldown), so an empty sitting explains itself instead of only reading
  "Nothing due." Adult web and mobile; the finished session payload
  (`StateDto` / `ReviewState`) gains an additive `next_due_ms` instant.

- `alix deck copy <deck> <workspace>` and
  `alix deck move <deck> <workspace>` transfer one initialized workspace member
  with its owned frozen assets and augmentation. Both reuse the same public
  bundle boundary as wormhole sharing; copy excludes progress, while confirmed
  move carries progress between distinct user roots and removes the source only
  after the destination is complete.

- `alix workspace update <dir>` reconciles frozen source-backed members with
  their live local sources. It stages an exact sibling workspace for
  review, then `--apply` publishes those same bytes without another model call
  or `--discard` removes them. Retained IDs require unchanged learning content;
  changed and obsolete cards retire with their IDs, while replacements receive
  fresh IDs during staging.

- Plain fact cards can carry multiple `<!-- at: ... -->` citations. The adult
  source view resolves every locator and stacks the editor-style excerpts in
  authored order inside one scrollable answer region.

- `alix doctor --repair-source-locators` explicitly fingerprints reviewed
  source citations and rebases a uniquely relocated exact excerpt while
  preserving deck and card IDs. Plain doctor remains read-only.

- A private vulnerability-reporting policy and a tracked threat model covering
  local files, LAN pairing, AI providers, sharing, persistence, mobile, and
  release boundaries.

- `alix profile`: define and launch a named alix instance per person (its own
  decks, port, and adult/kids frontend), reachable on your LAN with a stable
  token so phones can bookmark it. `alix profile add/list/remove`, `alix
  profile <name>` to launch, `alix profile default` to pick what bare `alix`
  launches, and `alix --launch-all` to boot every profile at once.

- Multiple-choice cards you author directly: write the answer as a GitHub task
  list (`- [x]` correct, `- [ ]` distractors). It renders as a checklist in any
  Markdown previewer and drives the Recognize quiz from your own options, with
  no AI distractor pass needed. Task lists in notes and card fronts render as
  checkboxes too.

- card text now renders inline Markdown: `**bold**`, `*italic*`/`_italic_`, and
  `` `code` ``.

- LaTeX math in cards: `$...$` renders inline and a whole-line `$$...$$`
  renders as centered display math in adult web, kids web, and mobile. Formula
  clozes support `\blank{...}`, and RaTeX chemistry remains available through
  `\ce{...}`. The Rust core produces one self-contained SVG shared by every
  graphical client while decks, grading, fingerprints, and progress retain only
  the authored source.

- Committed manual-QA examples for graphical math rendering and a
  self-contained workspace with a frozen Rust ownership trace.

- the picker's focus drawer now shows a deck's preamble (the prose written
  under its `#` title), which was parsed but never surfaced before

- `alix doctor` flags a dangling `requires:` (one naming a deck that does not
  exist), so a renamed or deleted prerequisite is reported instead of silently
  dropping the gating edge.

- `alix doctor` warns when a card's `<!-- id: -->` marker is not the card's
  closing line (the position stamping mints at), so a hand-placed marker drifts
  back to the canonical shape instead of scattering through the deck.

### Changed

- Breaking: an initialized deck (one with an `id:`) must now also declare
  `format-version: 1`. A deck without it fails to load and `alix doctor` names
  the
  conversion tool. Decks with no `id:` are unaffected: they are uninitialized,
  and `alix deck init` writes both keys. Pre-1.0, existing decks are converted
  by a disposable external script rather than by a compatibility path in alix.

- Breaking: every card-relative review POST (`/api/grade`, `/api/skip`,
  `/api/acquire`, `/api/check`, `/api/choose`, `/api/remove`,
  `/api/promote`, `/api/restart`, and the four tutor POSTs) must echo the
  `StateDto.study_revision` in an `X-Alix-Study-Revision` header. Missing
  or malformed is 400; stale is 409 and mutates nothing, so retrying a
  grade whose reply was lost cannot grade the next card. Both web clients
  echo it and refetch the state on a 409.

- Breaking: `POST /api/choose` now takes `{index, card}`, where `card` is the
  `card.id` of the card the pick was made on. A pick naming any other card is
  refused with 409 and grades nothing. The revision header proves the client
  saw a transition; the id proves it is answering the card it rendered, which
  the revision alone cannot. Both web clients send it.

- Session pacing is now two `[review]` keys: `max_session` (cards a single
  sitting serves, default 10) and `new_cards_percent` (the new-card share of
  that cap, default 30). The old `max_new` / `limit` keys and the `--new` /
  `--limit` launch flags are gone; `--session N` overrides `max_session` for one
  launch. Each sitting first selects its capped set (new and due each get a
  share, whichever pool is short lets the other backfill to the cap) and only
  then orders that slice for serving, so a deep overdue card no longer starves
  behind shallow ones. Per-card cooldown floors now survive a restart, so a
  chained sitting skips a card that is still cooling. The `/api/select` body
  drops `max_new` / `limit` for a single `session?` field, and the done-phase
  `StateDto` / `ReviewState` gain additive `due_left` / `new_left` backlog
  counts (the summary now says "N still due" or "Start N new"). Testers: delete
  any commented `max_new` / `limit` lines from your config and `alix.local.toml`
  (a stale key is now a loud error naming the replacement, and `alix doctor`
  flags it in a local manifest).

- "Seen" now means the card was shown to you at least once, right or wrong: the
  first time a card becomes the displayed card in any session, the store
  records a one-time presentation stamp (`presented_ms`), so a card you met
  and failed, or merely opened a session on, no longer looks untouched. The
  drawer funnel's `s seen` count follows this meaning. `acquired_ms` is now
  absent until a card is acknowledged or answered correctly at least once.

- The drawer and breadcrumb heatmaps paint five tiers instead of a
  retrievability gradient: neutral untouched, grey seen, white acquired, a
  learned (graduated) card green/yellow/red banded by its current Recall
  retrievability (strong `>= 0.9`, weak `< 0.7`, fading between), and purple
  retired. On the wire, `DeckDrawerDto.heatmap`, its topology `cells`, and
  `CrumbDto.cells` carry tier names (strings) instead of numbers.

- A picker row no longer prints a right-aligned status counter for a new or
  started deck: "new" duplicated the NEW chip, and a started deck's `k/N`
  graduated counter was cryptic and redundant beside it. New and started rows
  now
  show the title and state chip only, so the chip is the row's single state
  signal; finished, mastered, and exam-due rows keep their status word. The
  graduated count moved to the focus drawer's progress funnel (as "learned").

- **Breaking (pre-1.0):** deck ids are now self-describing and prefixed. A deck
  declares `id: "deck-<token>"` (the `id:` frontmatter key replaces `alix-id:`),
  and every card marker is `<!-- id: card-<token> -->` (`card-<token>-N` for a
  cloze hole, `card-<token>-r` for a reversed twin). The same prefixed id names
  the deck's state documents (`progress/deck-<token>.json`,
  `augment/deck-<token>.json`) and asset directory (`assets/deck-<token>/`), and
  `requires:` accepts a `deck-<token>` id so a prerequisite survives a rename.
  Source citations use named fields, `<!-- at: <src>:<lines> fingerprint:
  xxh64-<hex> asset: sha256-<hex>.<ext> -->`, replacing the old ` @ xxh64:`
  form: `at:` is always the real source path, and `asset:` (present only on a
  frozen citation) holds the cited excerpt exactly. Freezing no longer rewrites
  `source:` and stamps nothing; a frozen deck keeps its real source. The
  `origin:` key in deck frontmatter and the workspace manifest merged into the
  multi-valued `source:`, and the card-level `origin:` directive was retired
  with no replacement; a workspace manifest may declare a
  top-level `source` (the material the workspace is about), which the tutor and
  examiner receive as layered supporting context under the deck's own sources
  and which `has_exam` counts. A `source:` value is one expression (a URL, a
  file, or a directory); the `" + "`-joined form is retired and now fails to
  parse with a one-source-per-list-entry hint. Old-format decks fail to load
  loudly and the deck conversion tool rewrites them; there is no runtime reader
  for the old shape. `alix doctor` flags an un-converted bare-token state
  document or a `source:` that points into `assets/`.

- Progress and augmentation documents now reject an unrecognized field instead
  of ignoring it. Before, renaming or removing a field in a future build would
  let old documents load with that field silently dropped, losing that data on
  the next save; now such a document fails to load loudly and is left on disk
  for external conversion, with no format version bump.

- Review progress now persists as it happens: every grade, acquire, exam
  flag, badge, and card mutation writes the deck's progress document before
  the response returns, so closing the browser or killing the server
  mid-session no longer loses the sitting. The former session-batched flush
  (one write per transition) is gone; transition flushes remain as backstops.

- The server drains its workers, flushes any unsaved state, and exits cleanly
  on Ctrl-C or SIGTERM instead of dying mid-request.

- Dependency changes now pass a reviewed duplicate-family gate through
  `make deps-check`, preventing an avoidable second compiled version from
  entering the graph unnoticed.

- **Breaking (pre-1.0):** initializing a source-backed workspace member now
  freezes each cited source excerpt and every local card image before success.
  A citation's frozen object holds exactly its excerpt under
  `assets/deck-<token>/sha256-<digest>.<ext>` (uncited lines never enter an
  asset), and a citation-less member initializes with no freezing at all;
  `source:` stays the deck's real material, and runtime source consumers fail
  closed on live, cross-deck, missing, or corrupted assets. A URL-valued
  `source:` grounds the exam and tutor but holds no freezable bytes; citations
  must land in a local source.

- Single-deck sharing now carries and validates the complete deck-owned asset
  directory plus matching augmentation while continuing to exclude progress.
  Generated workspaces freeze in hidden staging before publication, and merges
  add immutable objects without replacing unrelated decks' assets.

- **Breaking (pre-1.0):** typed `WorkspaceFiles` and `UserFiles` owners now
  separate shareable deck material from private learning state. Assets and
  per-deck augmentation stay with the workspace content; `--store` and a
  workspace `store` setting relocate only progress and recent history. Sharing
  carries matching assets and augmentation while excluding progress and local
  configuration.

- Tutor and exam grounding now combines frozen evidence with the live
  `source:` values, deck and workspace layered apart (deck sources are the
  primary grounding, the workspace source supporting context). URL sources are
  fetched only when the backend and tool grant permit it; local sources still
  require explicit source access. The tutor continues from frozen evidence with
  a visible warning when the full current source is unavailable.
  `alix generate --source-url <URL>` records a portable public URL as an
  additional deck source or the generated workspace's `source`.

- **Breaking (web/mobile APIs):** `CardDto` and the shared mobile `CardView`
  replace the single `at` citation with ordered `citations`; web citation
  entries carry their resolved excerpt or per-locator error. Both views also
  gain `back_units`, the core projection used to render ordinary answer prose
  independently of authored physical line wrapping.

- **Breaking (pre-1.0):** every complete `<!-- at: ... -->` citation now carries
  a named `fingerprint: xxh64-...` field. Review, trace, tutor grounding, and
  grading fail closed when the addressed text does not match, showing a warning
  instead of unrelated source. Generated and frozen citations are stamped at
  creation; hand-authored citations remain incomplete until explicitly
  reviewed and stamped.

- **Breaking (pre-1.0):** workspace member decks now live only under direct
  `decks/*.md` children. Manifests, assets, and augmentation remain at the
  workspace root; private progress is colocated there by default but may use a
  separate user-files root. Relative member sources and images anchor
  at the workspace. Workspace creation and every generation/import/receive
  surface write the new shape, while `alix doctor` reports initialized root
  decks that are not discovered. Existing workspaces must move their deck files
  into `decks/` without changing its `id:` or card ids.

- **Breaking (pre-1.0):** progress and AI augmentation now live in independently
  versioned documents per initialized deck:
  `progress/deck-<token>.json` and `augment/deck-<token>.json`. Renaming a deck
  keeps
  its state, reviewing different decks on different synced devices no longer
  rewrites one shared file, and stale local revisions fail instead of silently
  replacing a newer document. This is a clean pre-1.0 format break: production
  reads only version-1 per-deck documents and contains no runtime converter for
  preceding layouts. Same-deck concurrent offline review is still unsupported;
  use `alix doctor <folder>` to surface incompatible or orphaned documents and
  synchronization conflicts.
  Sharing carries matching per-deck augmentation and recursively excludes all
  progress, temporary, backup, and conflict material.

- **Breaking (pre-1.0):** a hand-authored Markdown file must be explicitly
  initialized with `alix deck init <file>` before it appears in the picker or
  can be reviewed or augmented. `alix deck init` stamps a fresh
  `id: "deck-<token>"`; a frontmatter `id:` whose value is not a `deck-<token>`
  id is rejected rather than adopted. Opening an initialized deck still assigns
  ids to newly added cards, while ordinary `.md` prose with `##` headings is
  ignored and never modified.

- Production CI and release workflows now select exact Rust, Flutter, Java,
  Node, Android NDK, FRB codegen, mdBook, and coverage-tool versions. Every
  directly referenced GitHub Action uses an immutable commit SHA, a blocking
  check prevents movable pins from returning, `make install` explicitly uses
  the repository's exact Rust pin and lockfile, and scheduled drift jobs remain
  the explicit non-publishing path for testing current upstream toolchains.

- **Breaking (pre-1.0):** grounded tutor filesystem access now requires an
  explicitly declared deck or workspace `source`; its root is the deck's first
  local-path source (workspace source as fallback). A citation still supplies
  the card's evidence, but alix no longer guesses a wider project root from
  `Cargo.toml`, `.git`, or other markers. A public URL source can supply current
  context when `WebFetch` is available.

- Trace source excerpts now highlight exact, case-sensitive terms that the
  checkpoint author marked as inline code in its key points.

- **Additive (web API):** card display projection now comes from the shared
  Rust core. `InlineRun` gains optional `math`, `CardDto` gains `context_runs`,
  and `StateDto` gains `choice_runs` and `keypoint_runs`; every run list stays
  in index lockstep with its existing text field. `CardDto` continues to expose
  text fallback for clients that ignore the new fields.

- **Additive (web/mobile APIs):** trace walk state now carries inline-run
  projections for its description, checkpoint prompt, givens, key points, and
  note alongside the existing raw strings.

- Mobile review now consumes the core's shared inline runs for bold, italic,
  code, and LaTeX math instead of rendering raw card strings separately.

- Android release builds now size-optimize the embedded Rust core with fat LTO,
  one codegen unit, and stripped symbols. `make aab` produces the Android App
  Bundle for Google Play while `make apk` remains the GitHub-release and
  phone-smoke artifact.

- Inline code (`` `like this` ``) now renders with a distinct, theme-aware
  color for readability.

- **Breaking (pre-1.0):** inline `*`/`_`/`**` in existing card text now renders
  as emphasis; a deck that used them literally (e.g. `2*3*4`) will render/grade
  with the markers stripped. Escape with a backslash (`\*`) or wrap in inline
  code (`` `2*3*4` ``) to keep them literal. Run `alix doctor <deck>` to find
  affected cards.

- **Breaking (web API):** `CardDto.front` and `CardDto.back` now contain
  inline-marker-stripped content, while the new `front_runs` and `back_runs`
  fields carry display formatting. Sentence-shaped `NoteUnit` values also gain
  `runs`.

- the picker's focus drawer now opens for every deck, not only decks with a
  topology augmentation, and shows a per-card retrievability heatmap: a single
  whole-deck bar for a plain deck, split into named regions when the deck has a
  topology. Cards you have never reviewed render as a neutral cell rather than
  red, so a fresh deck reads as unlearned instead of failing

- **Breaking (web API):** the drawer no longer shows a raw due count. `POST
  /api/deck-topology` is renamed `POST /api/deck-drawer`; its response
  `DeckTopologyDto` becomes `DeckDrawerDto` (gains `preamble` and a flat
  `heatmap`, drops `deck_due`), and `RegionInfoDto` drops its `due` field

- `alix profile list` now shows each profile's config file path, so an
  unexpected
  decks directory or port can be traced to the file that set it.

### Fixed

- `POST /api/choose` read its request body straight off the socket with no
  size cap, the one JSON route that bypassed the central 256 KiB body cap; a
  client could grow the server's memory without bound. It now reads through
  the same cap as every other JSON body.

- Resetting a workspace deck over the API now reaches the deck listing
  immediately. Previously the listing could keep serving the pre-reset
  progress (deck "started", nothing startable) from a retained store
  snapshot until the workspace was next opened, leaving the picker's row
  actions disabled for a deck that had just been cleared.

- `alix --port 0` (OS-assigned port) now announces the port the kernel
  actually bound instead of printing an unreachable `http://127.0.0.1:0`
  URL. Explicitly requested ports are unaffected.

- The blank first load. A connection burst against a fresh or idle server
  (typically a page reload right after the server starts) could leave one
  accepted connection's request unread inside the HTTP layer's task queue,
  stalling a single asset or API call for about two minutes while its
  siblings completed instantly; the page shell rendered but the app never
  booted, and the focus drawer could silently fail the same way. The server
  now pumps its own listener once a second, which releases any stranded
  connection within about a second. Measured before/after: a
  restart-and-reload loop in a real browser stalled on the first cycle
  without the pump and completed 30 consecutive cycles with it.

- `alix deck init` no longer writes a second `format-version:` into a deck that
  already declares one. The duplicate is an invalid YAML mapping key, so the
  deck stopped loading entirely: a file copied from the manual's frontmatter
  example hit this on its first initialization.

- The picker no longer stays blank forever when a start-up request stalls. Boot
  already retried a *failed* request, but a request that never answers is not a
  failure: nothing settled, so nothing retried, and only a manual reload
  recovered. Start-up now gives up on a stalled request after four seconds and
  retries, which is the recovery the existing retry was meant to provide. This
  bounds the symptom; it is not a fix for whatever strands the request.

- Cloze cards read as fill-in-the-blank. The gap you are answering is a chip
  with a single rule on the baseline instead of four separated underscores, the
  other holes are quiet chips instead of `[…]`, and the gapped sentence now
  leads the card while the front line above it steps back to a topic label. The
  sentence is the question, but it was set smaller than the topic. Plain cards
  are untouched: the topic styling applies only to a card that has context
  lines.

- The picker keeps keyboard focus when you click outside the deck list, so the
  row-navigation keys keep responding instead of going silently dead. Only
  clicks inside the stage were handled before; a click anywhere else stranded
  focus on the page body.

- Jumping to the last deck with `G`/`End` now reveals its drawer. The
  drawer is fetched after the jump, so it opened below the fold and looked like
  it had not opened at all.

- A submenu's Cancel now sits where Back does, on the left, instead of trailing
  the depth chips on the right. It is the same "leave this level" action Esc
  performs, so it no longer moves depending on which menu is open.

- The tutor panel names the model that actually answered instead of showing
  `model: default`. When `[ask] model` is unset the backend CLI chooses, and
  alix could not name it; it now reads the model out of the backend's own
  startup event and reports that. Unset stays unset: alix does not pin a model
  on your behalf, it just stops pretending not to know once the backend has
  said. Still shows `default` before the first answer of a session, which is
  the only point at which nothing has been reported yet.

- Reading session state no longer moves you to a different card. Sitting on one
  card for longer than the five-minute acquire cooldown (asking the tutor, for
  example) let an earlier failed card come off its floor, and because the server
  re-picked the first servable card in roster order on every state read, closing
  the tutor landed you on the card before the one you were working on. Polling
  now reports the session rather than reshuffling it: the card you are on is
  kept while it remains servable. Grading, skipping, acquiring, and removing
  still move on exactly as before.

- `alix generate` now checks the output destination before calling the AI
  backend. Pointing it at a workspace that does not exist, or at a deck name
  already taken without `--force`, spent a full generation (minutes, and a paid
  call) before reporting a failure that was knowable up front. Both the deck
  and the `--trace` walk paths resolve the destination first.

- `alix reset --orphans <folder>` now finds orphans when the folder holds
  exactly one live deck. It had opened only that deck's own progress document,
  so a leftover `progress/<id>.json` stayed invisible: the command reported "No
  orphaned progress to reset." while `alix doctor` kept reporting the orphan. A
  folder target now scans its store root's whole aggregate, the same documents
  `doctor` reads; a deck-file target judges only its own document, so it cannot
  reach a sibling's progress. A folder whose last deck was deleted is a valid
  target.

- `alix reset --orphans` now aborts when a deck-like file in the target cannot
  be parsed, instead of judging that deck's live progress orphaned and deleting
  it. Decks still awaiting an `id:` line count as live.

- `alix reset --orphans` names a target that is neither a deck file nor a
  folder instead of reporting it as unlistable.

- The picker no longer reopens an inactive workspace's progress from disk once
  that workspace has been studied this run: the progress owner retains a
  projection of every document it has owned, so a progress file briefly parked
  or replaced by an editor or sync tool cannot resurrect a mid-study deck as
  "new" in the listing.

- A rejected augment open (for example over a duplicate card token) no longer
  swaps in the progress store it validated against: like exam start, select,
  and browse before it, validation runs on a candidate and the store installs
  only when the augment session actually opens, so subsequent grades keep
  saving through the still-active document.

- Cancelling an AI call now kills the backend's whole process tree, not just
  the CLI itself: the child starts as its own process-group leader and cancel
  signals the group, so helpers the backend spawns (node, browsers, git) die
  with it instead of surviving with API quota and source access. On Windows
  only the direct child is killed, as before.

- Shutting down while a paired client's tutor request is in flight now cancels
  that subprocess too: the remote ask path kept no cancellation handle, so the
  server could report shutdown complete while the AI process kept running.

- A rejected exam start, deck selection, or browse no longer swaps in the
  progress store it validated against: validation runs on a candidate and the
  store is installed only when the transition actually happens, so an accepted
  grade after a refusal keeps saving through the still-active document instead
  of silently writing progress into the wrong one.

- The picker no longer reopens progress from disk for the workspace that is
  actively being studied: the listing reads the study owner's own view, so a
  deck mid-review cannot resurface as "new" while an editor or sync tool
  briefly parks or replaces its progress file.

- Shutting the server down now cancels an in-flight tutor call and reaps its
  subprocess (advancing past the card or replacing the question does the
  same), so a Ctrl-C while the AI is thinking no longer leaves an orphaned
  process burning quota with source access.

- A catalog listing can no longer be served from a build whose inputs went
  stale while it ran: a refresh carrying newer progress leads its own build
  instead of joining the in-flight one, and changing the decks folder mid-build
  discards that build instead of publishing a listing of the old root.

- A tutor exchange that finishes after the card has already advanced is now
  discarded: the late answer no longer appears under the next card's
  transcript, and a late note or draft is no longer applied (previously the
  note was written to the earlier card's file even though the screen had
  moved on).

- Two imports of the same deck name no longer race each other (or a
  concurrent receive or generate landing) for the destination: every
  destination write now runs on one owner, so exactly one same-name import
  lands and the landed file is intact.

- A passed or failed exam's progress write no longer saves silently outside
  the save-error accounting: a transient failure now shows the "progress
  isn't saving" state and the result is retried by the next flush instead of
  being lost when the session closes.

- A progress save that keeps failing no longer lets a deck switch silently
  discard the unsaved session: select, browse, deselect, reset, exam start
  and close, walk leave, and augment open and close now answer 500 and keep
  the current session active while the store cannot be flushed. Repairing
  the disk and repeating the same request retries the flush; there is no
  force-discard path.

- `GET /api/doctor` no longer holds the application state lock while it
  probes the backend and wormhole binaries for their versions: the lock is
  held only to snapshot the store path and decks root, so a slow or hung
  version probe cannot freeze every other request (grades, pickers, state
  polls) for its duration.

- An unreadable decks folder (deleted, renamed, or pointing at a plain file)
  no longer masquerades as an empty catalog: `GET /api/decks` now answers 500
  and logs the cause instead of returning "no decks". The adult picker shows a
  calm loading line while fetching and, on failure, a quiet "Couldn't read the
  decks folder." notice with a Retry button; the kids client's existing
  "couldn't find your boxes" notice now actually appears in this case (the
  empty-success response used to show "No boxes yet" instead). Selecting by
  name is unaffected: an unknown name still answers 400.

- The server no longer dies when the consumer of its stdout goes away (for
  example `alix ~/decks | head`, or a supervisor closing the pipe after the
  URL line): the startup announcement lines now tolerate a closed stream
  instead of panicking mid-serve.

- Picker rows (decks and workspaces alike) no longer light their frame on mouse
  hover; keyboard selection is the single frame highlight.

- A multiple-choice card starts with its first option focused, so Enter or the
  nav keys act immediately instead of needing a first keypress to focus.

- A Recognize session now paces first contact like every other depth: at most
  `max_new` never-met cards enter per session, and an explicit `--new` is
  honored there (it was silently ignored). Already-met cards still awaiting
  recognition all enter, uncapped, so the recognition sweep keeps its contract.
  Previously a fresh authored-choice deck, which defaults to Recognize, opened
  its whole deck in one session.

- A cached review order (`alix deck augment --target order`) now applies at
  Recognize; the walk sorts the session before any `limit` truncation, so a
  limit keeps the topologically first cards.

- The multiple-choice quiz no longer highlights a hovered option like the
  keyboard-focused one; keyboard focus is the single highlight and the mouse
  gives no visual state.

- The picker's focus-drawer heatmap no longer renders a card met in an acquire
  pass (seen, not yet graded) the same as a card never touched. Such cards now
  show a dim "seen" cell instead of the neutral no-data cell, so first-pass work
  is visible. The `DeckDrawerDto` heatmap gains a `-2` value for it (the mobile
  crumb heatmap matches).

- A malformed virtual card in a progress document no longer vanishes silently
  (it was decoded best-effort and dropped on failure); the whole document now
  fails to load loudly instead, so a corrupt or out-of-shape card is surfaced
  rather than quietly discarding that card and its progress.

- Saving a document that first has to create its `progress/` or `augment/`
  directory now flushes the new directory entry to disk, closing a window where
  a power loss right after the save could drop the freshly created directory and
  the document inside it even though the save reported success.

- `alix doctor` no longer tells you to "restore from a backup" that Alix does
  not
  make; it now points at moving the file aside or restoring the folder from your
  own backup (see the manual's Backing up section).

- Hard-wrapped Markdown answer prose no longer renders each source line as a
  separate, widely spaced answer line. Ordinary flip and acquire views join
  soft wraps; line reveal, typing, fenced code, and generated lists retain
  their line structure.

- State, deck, and manifest writes now sync the file's data before the atomic
  rename and the directory entry after it, so a power loss right after a save
  can no longer leave the only copy of a document empty; previously the bytes
  could still sit unflushed in the OS cache when the rename was already
  durable.

- A review session whose progress document was replaced by another writer
  (for example a synced device) now reports it: the review state carries a
  `save_error` and the adult web client shows a persistent banner advising to
  reopen the deck, instead of failing every save silently into the server log.

- Adult review notes now use the same content width and text size as the answer
  or choice column instead of shrinking into a narrower, smaller box.

- The adult tutor's unsaved-conversation prompt no longer binds
  <kbd>Enter</kbd>, which remains available for composing a newline;
  <kbd>Escape</kbd> stays in the tutor, and leaving returns to the card that
  opened it without an unnecessary session-state refresh.

- Visible adult-client scrollbars now use slim, theme-aware tracks and thumbs
  instead of the browser's bright native chrome.

- Short fact-card source excerpts now keep the answer region's centered
  vertical alignment; excerpts still top-align when they overflow.

- Fact-card citations and trace walks now share the same editor-style,
  path-labelled source excerpt instead of using two visually inconsistent
  renderers.

- Trace walks now render authored inline Markdown in their description,
  checkpoint prompt, givens, key points, and note on adult web and mobile
  instead of showing raw markers such as backticks.

- Multiple-choice options now receive a fresh shuffle seed for each study
  session, so a card's correct answer does not return to the same memorized
  position every time the app is reopened; repeated state polls still keep the
  current question stable.

- `alix doctor` now reports malformed recognized LaTeX with its deck, card
  line, source snippet, and renderer error. CLI, desktop-server, and
  paired-mobile generation reject malformed math before placement without
  damaging an existing deck; generated text that does not parse as a deck keeps
  the previous lenient saved-draft behavior.

- Authored checkbox answers and distractors now retain their inline formatting
  source for display while duplicate detection and typed grading continue to
  use delimiter-free content.

- A formula that is the only inline run on its logical line now renders larger
  across adult web, kids web, and mobile, while math embedded in prose keeps
  its previous text-sized scale.

- Editing a card's content now invalidates its cached AI augmentations
  (distractors, note, questions, key points, and the reshaped answer).
  Previously a cached output generated from the old content was served until
  you cleared or regenerated it.

## [0.6.0] - 2026-07-20

### Fixed

- promoting a remediation card to a real one no longer risks a duplicate: the
  promotion wrote the card into the deck file but the matching removal from the
  in-session store was not persisted, so the card could reappear as both a real
  and a virtual card; the store change is now saved with the rest of the session

- the web listing no longer re-parses unchanged decks on every request: the
  server keeps a per-file cache keyed on (mtime, size) and re-reads only files
  that actually changed, so a warm `/api/decks` over a large collection stops
  re-reading every deck

- starting a review or walk no longer re-parses the whole collection to resolve
  the deck name: name resolution (`/api/select`, `/api/browse`, `/api/generate`,
  share/receive, exam start, augment open, and more) now reuses the same warm
  deck cache the listing does, instead of rebuilding the catalog from scratch on
  every call

- the picker no longer shows an empty page on the first (cold) load, needing a
  reload to appear: static assets (the page shell, fonts, css, js, and the key
  endpoints) are now served without waiting for the shared server state lock, so
  a slow cold deck listing can no longer stall the fonts the picker text is
  drawn with. The web server also handles connections with a worker pool now, so
  no single slow request blocks the accept loop

- the deck listing was quadratic in collection size (each loose deck probed its
  parent folder as a workspace, and that probe read every sibling deck); a
  325-deck folder took seconds per listing, now milliseconds

- **The session summary no longer reads all zeros after a first pass.** A
  fresh deck's first sitting is acquire-only (attempt-first exposure, no
  grades), but the summary said "Nothing due." with 0 reviews right after
  every card was introduced. The wire state now carries `acquired`
  (`StateDto`), and the summary leads with "New cards planted." and an
  "introduced" count, hiding the grade rows when nothing was graded.

### Added
- **A cloze card's blanks now keep their progress when you edit its gaps.**
  Inserting, deleting, or reordering `\blank{…}` gaps, or rewording the text
  around one, no longer shuffles or resets the review schedules across a card's
  blanks: each schedule follows its hidden word, matching first by word and
  surrounding context, then by word alone. A gap whose word *and* context both
  change starts fresh (it can't be told from a new gap), and a deleted gap's
  progress is discarded rather than inherited by a different word. Any cached
  choice-mode distractors and notes move with their gap.

- **`alix doctor` now lints a decks folder for identity problems, and `alix
  reset --orphans` clears the leftovers it finds.** Over a folder, doctor
  reports duplicate deck and card tokens (naming which copy keeps the earned
  progress), store keys matching no live card or deck (orphans, including
  pre-1.0 numeric ids), a non-canonical token, an unspliceable frontmatter that
  can't be stamped, the entries that are card content still without an id, and
  any stray `.txt`-era file that no longer parses. Orphans are never auto-pruned
  (they are evidence); `alix reset --orphans` is the explicit opt-in that clears
  them, scoped to a folder/workspace or the decks root.

- **A quiet Support line in the About dialog, on both the web and mobile
  clients.** Leads with the free alternative (telling someone who studies),
  a sponsors link second; About only, never on a study surface.

- **A paired phone can borrow the desktop's AI backend for the tutor and the
  exam, over `/api/remote/*`, including a trace deck's compression exam.**
  The client re-sends its own card, transcript, and answers with every call
  and keeps its own progress; the server only computes replies, it never
  writes its own store, session, decks, or recent list. A remote trace
  sitting checks no re-sit cooldown either, since that state is the
  browser's own store, not the phone's (`RemoteExamDto.is_trace` tells a
  trace sitting apart from a fact deck's). This is the server side only:
  the phone app's own pairing screen ships in a later mobile release.

- **A paired phone can also generate a deck from a URL through the desktop's
  AI backend, over `POST /api/remote/generate`.** The server returns the full
  deck text and a suggested file name; placing the file, and any collision
  handling, is the client's job, same iron rule as the tutor and exam. Server
  half only.

- **A paired phone can also condense its tutor conversation into note lines,
  over `POST /api/remote/ask/note`.** The server condenses up to three lines
  the same way the web's own note-save does; appending them to the deck is
  the client's job, same iron rule as the rest of the remote surface. Server
  half only.

- **A tutorial deck on first run.** A brand-new decks directory is seeded
  with "The alix tutorial": ten cards that teach alix by being reviewed —
  honest grading, spacing, depths, where decks come from, the AI features
  and what they send, and that your decks are plain files you own. Its
  last card says to delete it, and a deleted tutorial never comes back
  (seeding happens only when the decks directory itself is first created).
  The mobile app seeds the same deck into a fresh app-private folder.

### Changed
- **Breaking: media embedding is now standard Markdown `![alt](src)`; the
  `\image`/`\audio`/`\video` markers and the `<!-- img: -->`/
  `<!-- img-back: -->` directives are removed.** Write a plain Markdown image
  where you want one to appear: position decides the side (the question
  region for a front image, the answer region for a back image), a card can
  carry more than one per side, and the `src` is a standard Markdown path
  resolved relative to the deck file (the `image-dir:`/`img-dir:` frontmatter
  key is removed). The payoff over the old markers:
  a deck's images now render in any Markdown viewer that opens the file
  directly (GitHub, Obsidian, a plain preview pane), not just alix's own web
  app. The retired `math:` directive is also removed (it never had any
  effect). `\blank{…}` text occlusion is unchanged.

- **Breaking (web API): a card's images are now lists, not single fields.** The
  `CardDto` wire shape drops the scalar `img` / `img_back` strings and replaces
  them with `images` / `images_back`, each an ordered list of `{ src, alt }`
  (`src` the same `/img/<key>` URL as before, `alt` the `![alt](src)` embed's
  alt text or null). This lets a card carry several images per side, in
  source order, with alt text. Empty list when a side has no image. Both web
  clients render every image in the list.

- Grading no longer rewrites the progress store on every answer: in-session
  progress stays in memory and is written once, when the session ends
  (leaving the deck, switching decks, or opening an exam or the augmenter).
  Administrative actions (reset, deadlines) still write immediately. If the
  server process dies mid-session, that session's unwritten grades are lost.

- **Breaking: deck-level `strictness` removed: grading strictness is a learner
  setting (config or workspace defaults); a deck cannot ship grading rigor.**
  A `strictness:` key in a deck's frontmatter is now an ordinary unknown-key
  lint, not a recognized directive; only the global config default and a
  workspace `alix.toml`'s `[defaults]` feed `exam_strictness`.

- **Breaking: decks are now Markdown files (`.md`), and a card's
  identity is a minted token, not a content hash.** A card front is `## `, its
  answer lines follow plainly, a note is `> `, deck metadata (`source:`,
  `requires:`, `link:`, `trace:`, `reveal:`, `direction:`, …) lives in a `---`
  YAML frontmatter block, and a cloze gap is `\blank{…}`. The old `.txt` format
  (`# ` fronts, tab-indented answers, `! ` notes, `% key:` directives, `{{…}}`
  clozes) is **removed**: `.txt` files no longer enumerate or parse. Every
  card and deck carries a minted identity token, written into the file the first
  time it is opened (a per-card `<!-- id: … -->`, a deck `id:` in frontmatter);
  a card's id **is** that token verbatim (`token`, `token-N` for cloze hole *N*,
  `token-r` for the reversed half), so editing a card's front, note, or answer
  no longer changes its id (only re-stamping does). On the wire an id stays a
  JSON string (it always was); its value is now the token, never a decimal
  number, so clients must treat it as opaque (`docs/API.md`). Consequences
  (pre-1.0, no migration): progress stores reset, since their old content-hash
  keys are unreachable under the new token ids; an `augment.json` cache
  regenerates, because a topology-carrying cache fails the stricter load and is
  rebuilt while a topology-free one loads with now-unreachable keys (harmless);
  the internal `% requires:` rewriter (`set_requires`) is gone; and a lone
  whole-answer cloze (`\blank{…}` with no surrounding text) now parses, where
  the
  old format errored. There is no bundled converter: existing decks must be
  regenerated or hand-converted before opening. When alix writes an id it now
  goes on its own line at the end of the card's block (after its last answer
  or note line, before the next card or end of file), and every frontmatter
  block alix emits carries a blank line before its closing `---`; the old
  inline id, below-the-front id, and blank-less closer all stay valid input.

- **A copied deck no longer silently shares one card's progress across two
  files.** When two decks in a folder claim the same identity token (a copied
  file, or a card copied with its `<!-- id: … -->` comment), the undecorated
  original keeps the progress and the copy is re-minted the next time it is
  opened (`deck.md` beats `deck (1).md` / `deck copy.md`); unrelated same-token
  files fall back to scan order. File-sync and backup copies
  (`*.sync-conflict-*`, `* (conflicted copy…)`, `*.bak`/`*.orig`/`*~`) are
  excluded from every deck scan, so they never list, stamp, or error.

- **Breaking: a multiple-choice pick now requires a cached AI augmentation;
  options are never sampled from other cards.** Distractors were previously
  topped up by sampling the rest of the session's answers, which produced junk
  options (unrelated ones that gave the answer away, or near-duplicates). A pick
  now renders only from a deck's cached distractors (`alix deck augment --target
  choices`); without them there is no pick. Run the augment to keep
  multiple-choice on a deck that relied on sampling.

- **Breaking: the Recognize depth is now pick-only.** A Recognize session
  schedules only *recognizable* cards (ones a cached pick can be built for); an
  un-augmented card is no longer served there as a plain flip, which had blurred
  Recognize into Recall. A deck with no recognizable card can't be drilled at
  Recognize at all, so the picker greys the depth out (even under cram) — run
  `alix deck augment --target choices` to enable it. The deck-list API gains
  `can_recognize` (per row; group rows aggregate their members) for the gate.

- **The web app names your configured AI backend instead of assuming
  Claude.** The tutor header and the "working…" progress lines during
  augment and the exam now show the `[ask] backend` you actually use, so a
  Gemini, Codex, or Copilot user no longer reads "Claude is working…"
  (`AskInfoDto` gained a `backend` field).

- The CLI `--help` text is modernized to the Markdown deck wording: a trace
  stub now
  declares `trace:` in its frontmatter (no `% trace:`), replacing the stale
  old-format references.

- **Breaking: the `alix deck augment --target` value `topology` is renamed to
  `order`** (matches the web app's "Order" card); pre-1.0, no alias. This is a
  user-facing rename only: the internal type/field names and the `/api/deck-
  topology` endpoint are unchanged.

## [0.5.0] - 2026-07-15

### Fixed

- **A Recognize card with no buildable multiple-choice question no longer
  strands the review.** A deck too small for distractors (or without cached
  AI ones) reported the choice mode with no options to show, leaving the
  card with no way forward; it now falls back to a plain reveal-and-grade,
  so the reported mode is honest for every client.

- **`alix doctor <dir>` now actually runs its workspace lints** (the
  missing-icon warning, the deadline-key check) for a directory target; a
  routing gap meant they never printed.

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

- **Workspaces can carry a personal deadline.** Set `deadline = "YYYY-MM-DD"`
  and `deadline_ramp` in a workspace's `alix.local.toml` (CLI: `alix workspace
  deadline <dir> <date>`; also the API and the web picker's **Ready by…**
  action, key `d`), and scheduling bends toward it: intervals cap at the days
  left, target retention ramps up in the final stretch, and nothing schedules
  past the date, releasing back to normal once it passes. The picker shows a
  chip (date, days left, ready percent) on the workspace row and its drawer;
  `alix doctor` warns about a malformed deadline key.

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
  tutor to distill the conversation into a draft front/back; you edit it, then
  Add
  lands it as a free-standing card on the current deck, drilled like any other
  and
  promotable to the deck file. Adult review only; a non-parseable draft errors
  rather than inventing a card.

- Your decks folder is self-contained: drop it in a cloud drive (Dropbox,
  iCloud,
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

- **An experimental native app now lives in `apps/mobile`**: a Flutter shell
  embedding the lean Rust core to review decks offline on Android (and as a
  Linux desktop window). It has its own release track (`mobile-vX.Y.Z` tags,
  a signed APK on GitHub Releases) and its own changelog
  (`apps/mobile/CHANGELOG.md`); it is not part of the crate's released
  binaries.

- **For library consumers: a `full` cargo feature** (on by default) now gates
  the AI backends and the web server. Depending on `alix` with
  `default-features = false` compiles just the lean core (decks, scheduling,
  sessions, store) with no behavior change for anyone using the defaults; this
  is the half the mobile app embeds.

### Removed

- The placeholder **"Fun" kids theme**, superseded by the three real kids
  themes above.

### Changed

- **A never-drilled deck now defaults to Recognize when it already has AI
  distractors.** Plain Learn used to always start a fresh deck at Recall; now
  it starts at Recognize if the deck's augment cache has cached choices for at
  least one card (a genuine multiple-choice pick is ready), else Recall as
  before. The stricter acquire-time bar already protected an unaugmented
  card from ever seeing a junk multiple-choice, so this only changes which
  depth a well-augmented deck opens at, never what it's allowed to ask.

- **Picker UX pass: quieter refresh, a footer Back chip, a clearer depth
  button.** The window-focus re-scan now repaints only when the catalog
  actually changed, so alt-tabbing back no longer visibly flickers; the header
  back arrow is replaced by a footer **Back** chip (`esc`); the depth split
  button now reads **Depth…** instead of a bare triangle; and refreshing
  (`r`, or the header button) also re-fetches workspace icon images, so a
  regenerated emblem shows without a reload.

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
  matching the after-reveal rule the rest of review follows, so you can ask
  about
  a brand-new card (and make a card from it) without waiting for its first
  graded
  review.

- **Breaking:** the progress store now lives with your decks
  (`<decks_dir>/progress.json`) instead of the platform data dir
  (`~/.local/share/alix/progress.json`); bare `alix` and `alix <decks_dir>`
  share
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

- `/api/decks` rows now carry `selectable` — whether the row's `name` can be
  sent to
  `/api/select` (decks: yes; workspace/folder rows: no). Clients no longer have
  to infer
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
- Group rows in `/api/decks` no longer report `reviewable` unconditionally: a
  workspace/folder
  row now aggregates its members, and a deck that fails to parse reports nothing
  reviewable.
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
  Learn reuses the deck's own last-used depth (first use: Recall). **Recall** is
  the
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
  `% requires:` a source-less deck, that edge can't gate its exam; the lint
  names
  it and suggests adding a `% source:`.

- **Pairing token for `alix serve --lan`.** Serving to the network now
  auto-generates a token (printed at startup) and requires it on `/api/*`, so
  the
  LAN endpoint is no longer wide open. Pin your own with `--token` or
  `[serve] token`; the browser picks it up from the printed `…/?token=…` URL,
  and
  native clients send it as a bearer token. Opening the web UI without a valid
  token shows a prompt to paste it (then reloads) instead of a blank page.

- An `examples/gh-review-prep.rs` showing how to compose the library into an
  ephemeral, goal-scoped workspace for understanding a change you must read
  closely (a GitHub PR or issue) before acting on it. Read-only; a demonstration
  of composability, not a GitHub feature.

- **`[ask] backend` selector.** All AI calls now route through a pluggable
  backend. Set `backend` in `[ask]` to choose among `claude` (default),
  `gemini`,
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
  granted, and none is auto-approved. Trace building picks each backend's own
  strong model (Claude
  still defaults to `opus`; other backends inherit the CLI's default), so
  leaving
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

- **Web picker: a workspace's goal shows in its drill-in.** Opening a workspace
  now
  shows its goal (the one-line description) under the title eyebrow, the same
  goal the
  top-level list shows on the workspace row — so the context stays visible while
  you
  pick a member deck.

- **Web picker: a drawer indicator.** A quiet `▾` at the bottom-centre of a
  picker
  row marks a deck that has a focus drawer (a cached topology), so you can see
  at a
  glance which rows expand on focus instead of discovering it only after
  selecting.
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
  to get the producing (typing/explain) checks. **Card ids are preserved**: the
  retired markers were never part of a
  card's identity hash, so progress carries over. Upgrade an existing deck with
  a
  one-off textual rewrite (`#?` → a `% reveal: cloze` line; `% mode:` → `%
  reveal:`
  where a reveal-method applies, dropping `typing`/`fuzzy`/`choice`/`explain`),
  or
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
  short spaced step and re-appears once that step has elapsed, interleaved
  behind
  other due cards (so every scored review is genuinely time-separated). When
  nothing is due right now the session ends — a card still cooling is picked up
  the next session, or re-enters on its own if you keep the window open. Fixes
  cards graduating too early from same-session re-drills.

- **Breaking: a card graduates only after two spaced correct recalls.** A single
  Good after a miss no longer promotes a card to the long-term review phase
  faster
  than two clean passes would; two full Goods graduate it, a miss resets that
  progress, and a *partly* is neutral. (A lapsed card still re-graduates on one
  Good.)

- **A just-seen card starts drilling in the same session.** Acquiring a new card
  (the ungraded "Seen" first exposure) now settles for ~1 minute (was ~5) and
  the
  card stays in the sitting, so its first graded quiz comes back interleaved a
  minute later instead of waiting for a new session.

- **Breaking: FSRS is now the only scheduler.** alix schedules with FSRS-5 (via
  the `rs-fsrs` crate) for every review; the Leitner and SM-2 schedulers are
  gone, along with the `% scheduler:` directive and the `--scheduler` flag that
  chose between them. Grades map to FSRS ratings (missed it / partly / got it →
  Again / Hard / Good) and the next interval comes from FSRS, not a fixed stage
  ladder. The old per-card `stage` is kept only as a seed for a card's first
  FSRS
  review, so existing progress isn't reset. No compat shim, pre-1.0.

- **Breaking: `% unlock-stage:` removed** (the directive, the `--unlock-stage`
  flag, and the `[defaults] unlock-stage` key). A deck's exam is now gated on
  **graduation**: it turns *exam due* once every card has reached FSRS's review
  phase (past the initial learning steps), rather than at a configurable stage
  bar.

- **Retirement is now interval-based.** A card retires (rests until `alix
  reset`)
  once its FSRS interval reaches `retire_after` (default 1 year, configurable,
  `never` to disable) — previously it retired at the top Leitner stage.

- **`--cram` refreshes without rewarding.** A correct answer under `--cram` now
  re-anchors the card's due date by its current interval — no FSRS update, no
  reward — so cramming keeps cards fresh without inflating their schedule; a
  cram
  miss still lapses normally. Previously `--cram` fully rescheduled every
  answer.

- **`alix serve --lan` now requires the pairing token** on `/api/*`
  (auto-generated
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
  don't have those, so alix drops them for a backend without a session
  mechanism.
  To restore cross-turn memory there, the tutor now re-inlines the accumulated
  Q&A transcript into each prompt — a follow-up on a non-Claude backend carries
  the prior questions and answers, so the tutor no longer forgets what you just
  asked. Claude's efficient `--resume` path is unchanged.

- **Breaking — config keybindings are namespaced under `[keys]`.** Every key
  table
  is now a `[keys.*]` subtable: `[keys]` → `[keys.review]`, `[picker]` →
  `[keys.picker]`, and `[browse]` → `[keys.browse]`. This groups all bindings in
  one
  place and disambiguates the shared `remove`/`quit` action names per surface.
  Update
  your `~/.config/alix/config.toml` — the old top-level `[picker]` / `[browse]`
  and a
  bare `[keys]` section now error (no compat shim, pre-1.0). `alix config
  --init`
  writes the new layout.

- **Review cards settle a short answer below the midline.** The answer region
  now
  grows to fill the space between the question and the note and centers its
  content
  when it fits — so a short answer sits just below the card's middle instead of
  clustering under the question, and the lower half no longer reads as empty.
  Long
  answers and cited excerpts still top-align and scroll (using the whole card),
  and
  the question never shifts when the answer is revealed. Applies in browse too.

- **Web picker: cleaner dependency-tree lines.** The workspace drill-in's tree
  connectors (`├─` / `└─` / `│`) are now drawn as subtle dotted CSS guides in
  the row
  border colour — aligned under each parent's label and stopping at each row's
  border
  rather than crossing the gaps between rows — instead of single-line
  box-drawing
  glyphs that broke into disconnected segments on the tall rows.

- **Multi-line review answers left-align by default.** An answer with more than
  one
  line (a list, or several sentences) now renders as a left-aligned block,
  centered
  as a whole, instead of each line being independently centered (which read as
  ragged
  — especially for lists). Single-line answers stay centered, and reshaped-list
  bullets are unchanged.

- **The web UI header shows an animated `alix` wordmark.** The lightning-bolt
  mark in
  the review/picker and trace-walk headers is now a self-contained `<alix-logo>`
  web
  component — a flat orange "mitosis" wordmark that plays a one-time reveal on
  load
  (and on reload / `r`) and loops as a calm loading indicator while a
  Claude/server
  call is in flight. The shared header chrome — the `<head>` boilerplate and the
  brand mark — is now single-sourced (`_head.html` / `_brand.html`, filled in by
  the
  server) so all pages stay consistent.

- **Trace walk is now an in-page mode of the web review UI.** Picking a trace
  from
  the deck-selection screen no longer navigates to a separate `/walk` page — the
  walk
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
  unaugmented multi-line answer rendered centered (ragged, each line on its own
  axis)
  instead of as a left-aligned block. It now uses the same rule as review — any
  multi-line answer is a left-aligned block centered as a whole; only reshaped
  lists
  get bullets.

- **Web: Backspace leaves the augment view.** The augment screen accepted only
  `Esc`
  to return to the picker, while browse and the picker also honour `Backspace` —
  so
  `Backspace` felt inconsistent across views. It now leaves the augment view
  too, while
  the guidance box still edits its own text with `Backspace`.

- **Web picker: the header buttons are legible on light themes.** The ☰ menu and
  the
  ← / ⟳ nav buttons used the muted `--dim` colour, too low-contrast on some
  light
  themes (e.g. Solarized Light); they now use the main text colour, so they read
  on
  every theme.

- **Web picker: clicking empty space keeps keyboard focus.** A click anywhere in
  the
  picker area that isn't a row or control — including the margins around the
  centered
  list, not just inside it — no longer drops focus to `<body>` (where the
  row-nav keys
  go dead); it re-homes to the current (or first) row so arrow-key navigation
  stays live.

- **`alix explore --build` freezes cited excerpts more reliably.** When a
  generated
  `% at:` locator dropped (or added) a leading subdirectory — e.g. `chapter.md`
  when the file is at `src/chapter.md` — freezing couldn't find it and skipped
  it
  (`cited file not found, not frozen`), leaving a checkpoint without its source.
  Resolution now falls back to a basename search under the source root to
  recover
  the excerpt, and the fill prompt pins every locator to one consistent root so
  the
  mix is less likely to arise.

- **Workspace icons draw fast, without timing out.** The `explore --build` icon
  prompt now caps the emblem at a few compact primitive shapes instead of
  letting
  the model emit long `<path>` coordinate data — the token-heavy part that made
  the
  draw slow enough to time out (`could not draw a workspace icon: 'claude' timed
  out
  after 120s`). The draw also retries once. (Supplying `--icon`, or dropping a
  conventional `assets/icon.*`, still skips generation entirely.)

## [0.2.0] - 2026-06-30

### Added
- **Web picker: browser-style back + refresh buttons in the header**, for people
  who reach for the mouse/touch over keybindings. The **←** button goes back a
  view (disabled at the top level; the keyboard equivalent is `Esc`/`Backspace`,
  since the `←` *key* steps the focus drawer's regions) and `⟳` re-scans the
  deck
  list (also bound to the new `r` key). Refresh moved out of the burger menu,
  and
  the drill-in's footer "Back" chip is gone — the header **←** replaces it.

- `alix deck augment --target format` — a non-destructive pass that reshapes a
  badly-shaped card (e.g. a list crammed into one prose answer) into clean
  display lines, a tidier front/note, and a suggested answer mode, applied at
  review without touching the deck file or card identity. Also available from
  the
  web Augment screen. The reshaped output drops noisy inline backticks and puts
  a
  code snippet in a fenced block, rendered as a monospace code box on the card.

- **Augment decks from the web picker — no CLI needed.** Press **`a`** on a deck
  (or its new **Augment** button) to open a screen of what its augmentation
  cache
  holds: one row per target (choices, notes, questions, key points) with a
  coverage bar, plus its topologies. **Generate** fills only the cards a target
  is
  still missing — a costed background call, with a live spinner — **Remove**
  clears
  one target, and the topology row adds or drops named topologies. A shared
  guidance box feeds the `--with` steer. It writes the same `augment.json` the
  CLI
  does, so review reads it unchanged. Decks only; workspaces don't show it. (The
  terminal surface comes later — the library and server logic are shared.)

- **New cards are introduced as an *attempt*, not a cold quiz (acquire).** A
  never-seen card no longer drops you into a quiz you can't pass — its first
  encounter is a low-stakes try, then the answer, then one key ("Seen") files it
  on
  the ladder at stage 1, *ungraded*, with its first real quiz a later session.
  By
  default it's **recall** (the front shows first — try, then reveal); for a deck
  augmented with AI distractors (`--target choices`), an **atomic** card instead
  greets you as a **multiple-choice** question (pick one, see which was right).
  A
  guess never promotes or punishes — stage 1 either way. Start another session
  to
  drill what you've met (the per-session `--new` cap is unchanged — 10 per
  session).
  Terminal and web. The **acquire** step of the acquire → explain → maintain
  card
  lifecycle (the explain step shipped below).

- **Explain-mode key points — a checklist that derives the grade.** A new
  augmentation, `alix deck augment <deck> --target keypoints`, has Claude break
  each card's answer into the few load-bearing claims a reconstruction must hit
  (cached beside your progress, like distractors/notes). In **explain** mode the
  reveal then becomes a **checklist**: you tick the points you covered and the
  grade is *derived* — all → passed, some → partly, none → failed — turning the
  self-grade from a vibe into a per-claim check (TUI and web). An *atomic*
  answer
  (a single fact/term/date) is left without key points and keeps its plain
  reveal,
  the same way choice mode skips cards with no usable distractor. Tune the
  maximum
  with `[ai] keypoint_count` (default 5). First step toward an acquire → explain
  →
  maintain card lifecycle.

- **Web picker header.** The deck filter moved into the header — a compact box
  centered on the list — and a **burger menu (☰)** there holds **keyboard
  shortcuts**, **refresh decks**, **about** (the version, via a new
  `/api/version`
  endpoint), and **Theme…**. The **Mastered** jump moved to the header too.

- **Workspace icons in the web picker.** A workspace can show a small emblem
  next
  to it in the picker for quick recognition. Generated as an abstract SVG by
  `alix explore --into <dir> --build` (grounded in the workspace's topic), or
  supplied yourself with `--icon <file>` or an `icon = "assets/<file>"` key in
  `alix.toml` (else a conventional `assets/icon.*`). SVGs are tinted to the
  active
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
  topology orders the session and pick a region to scope the launch — by click
  or
  with **← / →** — with the selection's **due/new count** shown at the right
  end,
  all before the session starts (the in-card breadcrumb stays read-only).

- **The ask-Claude tutor grounds a frozen card in its live source.** For a card
  in a frozen workspace (`alix explore --into --build`), the tutor now reads the
  **original crate** for context — explaining how the cited code fits the
  surrounding source — with the **frozen snapshot excerpt as the anchor** (what
  the learner sees stays the ground truth, so the tutor never reasons about a
  drifted copy). It no longer cites opaque asset names (`01.rs`). The live
  source
  is found via a new `% origin:` directive (below); if it's gone, the tutor
  replies *"I couldn't find the source material of this card to provide a
  grounded
  answer."* so you can update or drop the card. The **trace-walk tutor**, which
  had no grounding at all, gets the same treatment. Gated by the existing
  `[ask] source_access` opt-in.

- **`% origin:` — the live source root a frozen deck's snapshots came from.**
  Written into a workspace's `alix.toml [defaults]` at build time and cascading
  **workspace → deck → card** like every other directive (a card may override it
  for a cross-repo source), it lets the tutor and drift detection find the real
  crate even though `% source:` points at the opaque `assets/`.

- **`alix check` flags drifted frozen cards.** When a frozen card's snapshot no
  longer appears in its live source — the lines changed, or the file is gone —
  it
  warns (`card at line N — frozen excerpt no longer found in the source`), so
  you
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
  whose excerpt is taller than the card shows a small `⌵ N more lines` pill at
  the
  cut edge (counting the hidden lines), in both the trace walk and a fact card's
  `% at:` citation — and it appears immediately on an overflowing excerpt, not
  only after the first scroll. The subtle edge-fade stays underneath it.

- **A trace's exam is its compression — AI-graded.** A trace's `% trace:` is a
  question ("how X becomes Y"); its **exam** is to answer it — retrace the whole
  path in a sentence or two from memory — and Claude grades that *holistically*
  against the path's checkpoints (no question generation, no source read: the
  checkpoints already paraphrase the source). **Passing masters the trace**
  (unlocking its dependents), exactly like a fact deck. Reached three ways:
  `alix exam <trace>` (which no longer refuses a trace), the **capstone**
  offered
  at the end of a walk (`Take the exam?`), or the picker's **"Take exam"**
  button
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

- **Web UI theme gallery — alix's own themes plus popular editor/slide
  palettes.**
  The web frontend (`--serve`) ships a gallery of colour themes: the alix
  **Dark**/**Light** originals and a playful **Kid** theme, plus crowd-favourite
  editor palettes — GitHub, Dracula, Nord, Solarized, Gruvbox, Catppuccin, Tokyo
  Night, Monokai, One Dark, Ayu, Rosé Pine, Everforest (light + dark where they
  have both). Pick one from the **Theme…** popover (the ⋮ menu, or a bar button
  on
  the trace walk): a grid grouped Light/Dark that **previews on a small sample
  card
  as you hover** (the app re-themes only when you click one) and remembers your
  choice per browser. The palette lives in a shared,
  server-served `theme.css` so every screen themes together; the default stays
  the
  original dark, so nothing changes unless you choose.

- **`alix deck augment` — deliberate AI deck augmentation.** A new command that
  enriches an existing deck with Claude and **caches the result** beside your
  progress (`augment.json`, keyed by card id); review reads the cache, so study
  stays instant and fully offline (Claude is never called mid-session). Three
  targets: `--target choices` writes plausible multiple-choice distractors (used
  automatically in choice mode, with the offline sampler as fallback — so choice
  now works even on a deck too thin to sample from); `--target notes` writes a
  short trivia/mnemonic note per card, shown *alongside* the card's own deck
  note
  on reveal (the deck file is never modified); and `--target questions` writes a
  pool of reworded phrasings of each question (same answer), a fresh one of
  which
  review rotates in each time a card comes up so it can't be passed by
  recognizing one fixed wording (plain, non-cloze cards). `--with "<guidance>"`
  steers how. Tuned under
  `[ai]` (`model`, `distractor_count`, `variant_count`, `timeout_secs`).

- **`alix check` rejects a cloze whose entire answer is one hole.** A `#?` card
  whose only hole spans the whole answer (e.g. `` `{{IdentStr}}` ``, with
  nothing
  but formatting around it) is a plain front→back card in disguise — blanking
  the
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
  workspace). The intro prose and the "select decks" label are gone, and the
  list
  fills the space.

- **Browse is now an in-page mode of the web app — no separate `/browse` page.**
  Hitting **Browse** in the web picker (or `alix browse <deck> --serve`) opens a
  read-only overlay right in the main app — step through every card with
  Prev/Next/Leave, seeing the reshaped answers, notes, and images — instead of
  navigating to a separate page with its own older picker. The standalone
  `browse.html` page and the `/browse` route are gone; terminal `alix browse` is
  unchanged. **Breaking:** the web browse is read-only (card removal stays a
  terminal `alix browse` feature).

- **Reshaped list answers show as a left-aligned bullet list.** When the
  `format`
  augment turns a crammed prose answer into a multi-item list, the web review
  and
  browse views render each item with a `•`, **left-aligned** (the list block is
  centered as a whole). Single-line tidies and a card's own authored back lines
  (a
  poem, typing answers) are left as-is.

- **Bigger cards in the web review and browse views.** The card was capped small
  (≤820/720px wide), so it sat in a sea of empty space on a normal screen at
  100%
  zoom and long questions/answers wrapped early. It now caps at ~1200px wide
  (94vw)
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

- **Leitner stage 1 now has a ~5-minute relearn/settle cooldown** (was 0). A
  newly
  acquired or freshly failed card becomes due ~5 minutes out for the *next*
  session, so starting another session right away no longer re-serves a card you
  just saw or just missed. In-session drilling is unchanged — a failed card
  still
  comes back the same run (the queue is served by position, not by due time).

- **Web picker keys.** Clicking a deck now **selects** it (opening its focus
  drawer when it has a topology) rather than launching outright — **Review** or
  Enter launches. **Browse** moved to **`b`**, freeing **← / →**: they step the
  focus drawer's region selection when one is open, and otherwise enter / leave
  a
  workspace. Up / down still move between decks. (The drawer is new this
  release,
  so only the Browse-key and click-to-select changes affect existing muscle
  memory.)

- **`alix deck augment` says what it's doing.** It now prints which augmentation
  it's generating, for which deck, and with which model before the (foreground,
  possibly slow) Claude call, instead of hanging silently until the result.

- **Breaking — one deck per session.** `alix review` and `alix browse` now take
  exactly one deck *file*: merging several loose decks into a combined session
  is
  gone, and a whole workspace is no longer reviewed at once. Workspaces stay an
  organizing layer — review their members one at a time (the picker drills in;
  `alix workspace <dir>` opens that picker), and a member still inherits the
  workspace's directives and store. `stats`/`list`/`reset` still take multiple
  decks (they're per-deck operations, not a merged session).

- **Breaking — review grades are now `failed` / `partly` / `passed`, replacing
  `again` / `good` / `easy`** (shown in the UI as **Missed it / Partly / Got
  it** —
  an honest self-report of understanding, not a pass/fail verdict; the real
  pass/fail is the AI exam). Fact-deck review and the trace walk now share one
  three-outcome grade: **failed** resets the card to stage 1, **partly** drops
  it
  *one* stage (a soft miss — it returns sooner but you keep most of your
  progress), and **passed** advances one stage. The old `easy` (+2 stage jump)
  is
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
  (`% at: 29.rs from src/caching.rs:46-66`), instead of smuggling it into a
  hidden
  `! from …` note that the display then stripped back out. Notes are the
  learner's again. **Existing frozen workspaces keep working for review and the
  exam, but the tutor can't ground them until re-frozen** (re-run
  `alix explore … --build`). Pre-1.0, so no compatibility shim. Card identities
  are unaffected (`% at:`/`% origin:`/notes are not hashed).

- **The review header no longer shows the stage ladder.** The always-on
  `new|s1|s2|…` stage histogram is gone from the review header (TUI and web) —
  it
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
  prompt asks for a terse title, but the model ignored it and appended the
  deck's
  contents after a colon — so the title is now **condensed deterministically in
  code** rather than left to the prompt: the enumeration is cut (at the first
  `:`/`;`/dash, or by a word cap when there's no separator), and the result is
  title-cased with code spans (`` `grpc` ``, `snake_case`, `CamelCase`,
  `ACRONYM`s) left intact. Workspace decks read as `The Crate Surface`, not `the
  crate surface: three-part Store/Execute/Inspect model, the three feature flags
  …`, and stop truncating in the picker. The condensed title also drives the
  file
  name, so slugs no longer trail a stray word from the cut enumeration.

- **Web trace walk: the leave button reads "Leave" and confirms an unfinished
  walk.** The hosted walk's return chip was "Decks"; it's now "Leave" (matching
  a
  fact-deck session), and leaving before the last checkpoint shows a "Leave the
  walk before finishing the path?" prompt (Enter leaves, Esc stays) — the same
  guard as review and exam. A finished walk still leaves immediately.

- **Web exam: leaving mid-answer asks to confirm.** Pressing Esc (or Quit) while
  answering now shows a "Quit the exam? Your answers won't be graded" prompt —
  Enter abandons it, Esc keeps going — so a stray Esc no longer throws away an
  in-progress exam, matching the review-session leave guard. (Other phases close
  immediately; the typed answer is preserved if you keep going.)

- **Reviewing a deck no longer pulls in its prerequisites' cards.** A review (in
  the TUI/CLI) now holds exactly the deck(s) you picked — `% requires:` decks
  are
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
  older alix now cleanly refuses a v2 store with an "upgrade alix" message
  rather
  than mis-reading the new deck-progress shape).

- **`% requires:` now gates the exam, not drilling.** You can review/drill any
  deck at any time, in any order — a prerequisite-locked deck is no longer
  blocked in the picker (it stays bright and startable; the lock is named
  explicitly when it's focused — the TUI footer says "🔒 Exam locked", the web
  shows its "Take exam" button disabled with a 🔒 — rather than a per-row lock
  glyph that read as "the deck is locked"). The dependency order applies to
  **exams**: to sit a sourced
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
- **The picker labels a trace by its description, not its filename.** A trace
  row
  in the picker (web tree and TUI drill-in) showed the raw file stem — a clipped
  kebab slug like `08-how-a-workout-starts-logs-a` — even though the trace
  already
  carries a readable name in `% trace:`. It now labels the row from that
  description (`How a Workout Starts, Logs a Set, and Advances to the Next`),
  condensed to a label-sized head so a long `--build`/hand-written path-question
  doesn't overrun the row. Plain decks (a `% title:` or neither) are unaffected.

- **A trace `--grade` reply that isn't a real verdict now errors instead of
  being
  scored as a miss.** The per-hop grader expects the model to answer
  `NAILED`/`PARTLY`/`FAILED`; an unrecognized reply (a weaker model ignoring the
  instruction) used to silently fall through to a failing grade — fabricating a
  verdict the model never gave. It now surfaces an error and falls back to
  self-grading, so a correct prediction is never quietly marked wrong.

- **`alix explore --into --build` now actually freezes its `assets/`.** The
  generated `% source:` paths were silently doubled: when `--source` is a
  subdirectory (a crate) but the plan writes a scope relative to the project
  root
  above it (`crates/x/src/lib.rs`), the write-time join produced
  `…/crates/x/crates/x/src/lib.rs` — a path that doesn't exist. Every citation
  read failed, so the freeze step copied nothing and the workspace was left with
  no `assets/` **and no warning**. Generation now anchors the scope
  overlap-aware
  (the write-time twin of the `% at:` read fix), so the citations resolve and
  the
  excerpts freeze.

- **A multi-file `% source:` (`a.rs + b.rs`) now freezes every cited file.**
  Snapshotting treated the whole ` + `-joined line as one literal path, so a
  multi-file source froze nothing; it now splits the source exactly as the
  review
  path does (shared `SourceBase`), so freeze and review can't disagree.

- **A missing or stale `% source:` base fails with a clear message.** A
  directory
  `% source:` that no longer exists used to have the locator joined onto it,
  yielding a baffling `…/README.md/src/lib.rs` "no such file"; it now reports
  the
  real cause — the source base doesn't exist (the path is likely stale or
  wrong).

- **A cited deck that can't be frozen is reported, not swallowed.**
  `alix explore --build` now warns which deck's source couldn't be read instead
  of silently leaving an empty `assets/`.

- **A `% at:` locator written relative to a project root above `% source:` now
  resolves.** When a deck scopes `% source:` to a subdirectory or file (e.g.
  `…/crate/src/executor`) but writes its `% at:` paths from the crate root
  (`src/executor/local_vm.rs`), joining them doubled the overlap
  (`…/src/executor/src/executor/local_vm.rs`, "no such file"). Resolution now
  walks up the base directory's ancestors until the cited file is found.

- **Frozen-snapshot excerpts show the original file and line numbers.** A walk
  or
  fact card whose `% source:` is a frozen `assets/` snapshot showed the asset
  (`30.rs`, lines 1-N) instead of the real source; the cited excerpt now
  relabels
  to the original `caching.rs:106-120` (from the location recorded on its `%
  at:`
  line) — in the walk, the fact-card citation and the terminal walk.

- **A long (hand-crafted) deck title no longer reflows the header.** The
  review/browse/walk headers truncate an over-long title with an ellipsis
  instead
  of wrapping to a second line and growing the header's height.

- **No stray blinking caret across the web app.** The caret is suppressed on
  card/slide prose everywhere — review, browse, and the trace walk — appearing
  only inside a real text input or a source-code excerpt (e.g. with the
  browser's
  caret-browsing on).

- **Ask-Claude (web): the input re-focuses when a reply lands**, so you can type
  a follow-up immediately instead of clicking back into the box.

- **A trace/fact citation against a single-file `% source:` no longer doubles
  the
  path.** When `% source:` is one file, every `% at:` reads *that* file; a
  locator
  that repeats the path relative to a different root (e.g. the crate root,
  `% at: src/executor/env.rs:44-64` against `% source: …/src/executor/env.rs`)
  was joined onto the file's own directory, yielding
  `…/src/executor/src/executor/env.rs` ("no such file"). Both the walk reveal
  and
  `alix check` now share one `locator_path` resolver, so they can't disagree.

- **Opening a deck with nothing due no longer bumps it to the top of the recent
  list.** A review now records the deck as "recent" only when the session
  actually has cards to review (`!session.is_finished()`), so merely entering a
  fully-drilled / all-on-cooldown deck leaves the recent order untouched.

- **A fact card's `% at:` citation resolves against a multi-file `% source:`.**
  A
  deck whose `% source:` joins several files with ` + ` (the generator's format,
  e.g. `<crate>/README.md + src/lib.rs`) now reads each card's cited excerpt
  from
  the right file. Previously the whole joined string was treated as one
  directory
  and the `% at:` file appended to it, so the reveal showed `cannot read the
  source …/README.md + src/lib.rs/README.md`. `SourceBase::for_deck` now bases
  on
  the first source file (matching `source_paths`); with several files a
  bare-line
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

- **You can sit the AI exam early to test out of the drilling.** The exam no
  longer requires every card drilled to the top stage first — it's available as
  soon as a deck has a `% source:` and its `% requires:` are satisfied (drilled
  or not). Passing it **masters** the deck regardless of card progress, which
  **unlocks its dependents** — so a learner who already knows a topic isn't
  forced to grind its cards. Exams still flow in dependency order: a **locked**
  deck stays un-examable until its prerequisites are mastered (pass *their*
  exams
  first). In the browser picker, a focused examable deck gets a **"Take exam"**
  button (and the `x` key); `alix exam <deck>` does the same from the terminal.

- **The web deck-selection screen now mirrors the terminal picker.** It is
  **single-launch** (no checkboxes): click a deck to start it, or open a
  **Workspace** / **Folder** to drill into its **unlock dependency tree** (each
  deck nested under the prerequisite that gates it). Rows are grouped into
  **Workspaces** (each with its last-progress time) · **Recent** loose decks ·
  **Folders**, and the filter searches *every* loose deck. A deck you can't
  start
  is dimmed — 🔒 locked (`% requires:`), 🕒 nothing due — and mastered/done/locked
  decks are kept out of Recent, with a `mastered 🎉` deck tucked into a
  **Mastered
  window** (`m`); navigation honors the `[picker]` config keys (served to the
  page
  at `/api/picker-keys`). A **locked** deck can no longer be *started* for
  review
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
  than the raw braced text, so existing cloze cards' progress is reset once —
  but
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

### Added

- **Fact cards can cite their source (`% at:`), shown on reveal.** A plain fact
  card may now carry a `% at: file:lines` locator into its deck's `% source:`
  (the same form a trace checkpoint uses — `file:lines`, or just `lines` for a
  single-file source). On reveal a `</>` marker appears on the answer; in the
  web you **click the answer** (or press `s`) to swap it for the line-numbered
  source excerpt and back, and in the terminal you press **`s`** — one view at a
  time, so the card stays compact. The excerpt is read live, so a moved/missing
  source shows "source unavailable" rather than a stale quote, and `% at:` is
  not
  part of the card's identity hash (adding it never resets progress). Reuses the
  trace walk's excerpt machinery via a shared `trace::SourceBase`/`excerpt_at`.
  The deck **generator writes these citations for you** — `alix deck` on a local
  source and `alix explore --build` add a `% at:` to each fact that maps to
  specific lines — and **`alix check` validates** a fact deck's citations,
  warning about one that no longer resolves (a moved or shrunk file). A
  workspace
  built with **`alix explore --into --build` freezes** every cited deck's
  excerpts into its `assets/` (fact decks now, not just traces), so the
  citations
  don't drift and the workspace travels without the upstream source; a frozen
  fact deck's `% source:` then points at the excerpts, so its exam grades
  against
  them. (Snippet names are workspace-unique now, so multiple frozen decks no
  longer collide in `assets/`.)

- **`% unlock-stage: N` — unlock a deck before its cards retire.** A `% source:`
  deck becomes *exam due* (its exam opens), and a source-less deck *finished*
  (its dependents unlock), once **every card reaches Leitner stage N** — without
  retiring them, so they keep drilling to the top stage; the directive only
  lowers
  the unlock bar. Default (unset) keeps the old gate: every card retired at the
  top
  stage. Settable per deck, in a workspace `alix.toml`
  `[defaults]`, or via `alix explore --into --unlock-stage <1–5>`. Generalizes
  the completion gate (`Deck::state`).

- **Browse a deck from the session-end summary** (terminal). When a deck turns
  *exam due* at the end of a review, the summary now offers `b` to **browse** it
  (a read-only walk through its cards) right next to `x` to sit the exam —
  useful
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
  reminder that the tutor uses the CLI default unless `[ask]` pins a stronger
  one.

- **`alix explore --title` shapes the scaffolded workspace; the goal becomes its
  description.** `alix explore --into <dir>` now takes an optional `--title` for
  the workspace's `alix.toml` `title` (omitted, the folder name is used). It
  also
  writes the `--goal` as a new `alix.toml` **`description`** field instead of an
  ignored `goal` key; a
  workspace's `description` shows **dim under its row** in both pickers
  (terminal
  and web).

- **Confirm before abandoning a review; commit the picker filter with Esc**
  (terminal) — quitting a review **mid-session** now asks to confirm (`Enter`
  leaves, any other key stays), so a stray `Esc` no longer drops a queued
  session;
  a finished session or a hard `Ctrl-C` still leaves at once (matching the web
  frontend). In the picker, `Esc` in the filter box now **keeps the filter** and
  drops to the list focused on the first match (a second `Esc` clears it),
  instead
  of discarding what you typed.

- **Picker disables decks with nothing to review** (terminal) — a deck with no
  card due right now (fully drilled, or all on cooldown) is dimmed and badged
  with a 🕒 clock, mirroring how a 🔒 locked (`% requires:`) deck looks, and
  `Enter` on it is a **no-op** — no more starting an empty session that bounces
  you out to a "Nothing to review right now" message. Such decks also can't be
  ticked into a merged review, and (in a workspace drill-in) sink below the
  startable ones. `--cram`, which ignores cooldowns, turns the gating off;
  browse
  never gates (any deck is browsable). New lib helper `session::has_reviewable`.

- **Reworked deck picker + trace walking from the picker** (terminal) — the
  no-argument picker is a clean, **single-launch** list (no checkboxes): `Enter`
  opens the focused row. Its header is just `alix`; rows are grouped into
  **Workspaces** (each showing when it last made progress, from its own store) ·
  **Recent** (loose decks you reviewed lately) · **Folders**, a blank line
  between
  sections, with the filter searching *every* loose deck. A deck that lives
  inside
  a workspace is kept out of Recent — you reach it by opening its workspace.
  Rows
  that share a title (two workspaces named the same) get a path hint to tell
  them
  apart; over-long rows (a trace's `% trace:` sentence) are truncated with `…`.
  Rows you can't start now are dimmed and `Enter` is a no-op: 🔒 locked
  (`% requires:` unfinished), 🕒 nothing due (on cooldown); a mastered deck reads
  `mastered 🎉`. The focus is on the **list** by default with Vim-style keys,
  rebindable in a new `[picker]` config section (`j`/`k` or arrows move,
  `l`/`Enter`
  open, `h`/`Esc`/`Backspace` back, `m` opens the Mastered window, `/` or
  `Ctrl-F`
  filters); jumping to the first/last row is fixed at `g`/`G` (or Home/End),
  like
  the `[browse]` pager. Mastered/done and locked decks are kept out of Recent (a
  quick
  launchpad); **`m` opens a dedicated Mastered window** of the exam-passed
  decks,
  or the filter reaches them. Long `% title:` / `% trace:` labels are capped so
  rows stay short. The picker and the review/walk/exam it launches now share
  **one
  terminal**: opening a deck and returning to the workspace no longer tears the
  TUI down and reopens it.
  Opening a **workspace** or **folder** drills into its members drawn as an
  **unlock dependency tree** — a deck nests under the `% requires:` prerequisite
  that gates it, foundations at the roots, siblings startable-first, each badged
  `· trace ·` / `· deck ·`. Opening a workspace, stepping back
  (`Esc`/`Backspace`),
  and **returning after a review/walk/exam** all happen within **one live
  screen**
  — no TUI teardown/reopen — so you can study a deck and **land back in the
  picker** (the workspace you came from, or the top list) to pick the next; only
  an `Esc` at the picker itself quits. The big gap it closes: a **trace** opened
  from the picker now
  **walks** (predict → reveal) instead of being flattened into a card review —
  both in the top-level drill-in and `alix workspace <dir>`. An explicit
  `alix review <trace.txt>` still flattens it (honoring the literal command).
  The
  multi-select machinery is retained in the code but unused for now. The web
  picker
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
  file into the workspace's `assets/` folder (`assets/01.rs`, `02.rs`, …)
  holding
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
  are dropped. The result is validated and written to `~/decks/`
  (`-o`/`--print`/
  `--force`, like `alix deck`). Conversion lives in the lib
  (`import::tsv_to_deck`).

- **`alix check` now validates trace `% at:` locators.** A trace deck is linted
  like any other: `check` resolves each checkpoint's locator against its
  `% source:` and warns (advisory, non-fatal) about any that name a missing
  file,
  run past the end of the file, give bare line numbers without a single-file
  source, or are absent — a quick "does this excerpt still exist?" structural
  check that catches a moved or trimmed source before a walk hits it. (Frozen
  snapshots are validated the same way.) It also prints the deck's `% trace:`
  description. Logic in the lib (`trace::Trace::lint_locators`).

- **`alix deck <source>`** (renamed from `alix generate`, which no longer
  exists as an alias) — generates a facts deck with Claude from a **web page URL
  or a
  local file/directory path**, mirroring `alix trace`. A URL is fetched with
  WebFetch and the deck starts with a `% link:`; a local source is explored
  read-only with `Read`/`Glob`/`Grep` at its root and the deck starts with a
  `% source:` (so `alix exam` can grade against it). This gives a facts-deck
  stub
  from `alix explore --into` a manual fill path (point `alix deck` at its
  `% source:`).

- **Traces (`alix trace`, experimental)** — a guided predict-and-verify walk
  along a *path* through a `% source:`, drilling the connections between facts
  (the edges) rather than isolated facts. A trace deck declares a `% trace:`
  (a path description that marks it a trace) and a sequence of `explain`-style
  checkpoint cards,
  each with a `% at:` locator (`file:lines`, or just `lines` for a single-file
  source) into the real source, and optional `% given:` lines that name
  off-screen symbols the question leans on (shown as a list under the question,
  so a tight excerpt doesn't orphan the names it uses). Walking it goes hop by
  hop: you **predict**
  before anything reveals, the real excerpt is **read live** from the source and
  shown with the key points, you self-judge the **gap** (Got / Partial / Missed
  — a weak edge resets so it resurfaces sooner, via the normal per-checkpoint
  SRS), and after the last hop you **compress** the whole path into two
  sentences. Self-judged and offline (no model call) by default; **`alix trace
  --grade`** instead has Claude judge each typed prediction against the key
  points
  and return the verdict + one line of feedback (a model call per hop, run at
  the
  lightweight `[ask]` tier — not the heavy build defaults below). **`alix
  trace <deck> --serve`** walks it in the **web frontend** (the same
  frontend-agnostic `Walk` state machine the terminal uses): a left **path
  rail**
  whose nodes color in by Got / Partial / Missed, each checkpoint's source shown
  in a line-numbered excerpt, and `--serve --grade` running the live grade on a
  background thread while the page polls; `--port`/`--lan` work as in `review`.
  `alix trace <deck> --map`
  prints the path without quizzing; the generic AI exam refuses a trace (its
  verification is the walk itself). See
  `docs/examples/workspace-showcase/decks/ownership-move.md`.
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
  become traces, node-shaped fact tables become decks), carries its `%
  requires:`
  prerequisites (the list is a valid topological order, foundations first), and
  a
  `% source:` scope. The goal scopes coverage — a broad goal spans every
  subsystem, a narrow one collapses to its slice (and traces it deeper). By
  default read-only (prints the plan); **`--into <dir>`** materializes it into a
  **workspace** folder — an `alix.toml` (the goal) plus a stub deck/trace file
  per
  item, `% requires:`-wired in dependency order with absolute `% source:` paths,
  ready to `alix trace --build` / author (refuses a non-empty dir unless
  `--force`). Add **`--build`** to fill them: `alix explore … --into <dir>
  --build` explores the source **once**, then resumes that same CLI session to
  write the full content of every item — predict-verify checkpoints for traces,
  fact cards for decks — so the workspace is review-ready in one command, with
  the
  items coherent (written from one understanding) and facts decks filled too.
  **`--walk`** instead builds an **explore walk** — a predict-verify
  trace over the source's *shape* (what it is → its domain nouns → entry point →
  spine → the first paths worth tracing), each hop revealing real structural
  evidence (the manifest, the module list, the entry enum). It's written to a
  file
  (`-o`, default `explore.txt`) and walked immediately, reusing the `alix trace`
  walk; re-walk later with `alix trace <file>`.

- **Workspaces** — a folder of decks reviewed together with shared directives.
  A folder is a **workspace** when it has an `alix.toml` manifest (a scoped
  `config.toml`) setting a `title` and a `[defaults]` table of directives that
  fill in what each deck leaves unset (precedence CLI > card > deck > workspace
  >
  default); a folder of decks *without* a manifest is a plain **folder** — still
  reviewable, but not a workspace. Both appear as their own rows in the picker
  (terminal and web, labeled "workspace" vs "folder") and drill into their decks
  (review all, or tick a subset); `alix review`/`browse <folder>` reviews the
  whole cluster. **`alix workspace <dir>`** opens a workspace into its own
  picker
  and routes each member to the right thing — a **facts deck** → review, a
  **trace
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
  **mastered**, which is what now unlocks dependent decks — drilling a `%
  source:`
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
  question at a time (Back/Next), then see a per-question breakdown — `alix
  exam`
  and `alix serve` share one engine (`exam::Sitting`) that runs Claude on a
  background thread and polls, so neither blocks. You reach it by **picking an
  `exam due` deck** (it launches the exam instead of an empty review) or from
  the
  **session-end summary** when a deck you were drilling just became exam-due.
  Exam-due decks aren't tickable into a merged review (they have no due cards).

- `% mode: explain` — **understanding cards**. The front is an open prompt and
  the back lines are the *key points* a good answer should cover (not a string
  to
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
  paths are used as-is). Images render in the web frontend only, so an image
  card
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

### Fixed

- **Generated decks now put a blank line between cards.** `alix deck`'s output
  cleaner (`generate::clean_output`) inserts a blank line before each card front
  (`#`) after the first, so a generated/`--review`ed deck is readable instead of
  cards running together. The first card stays attached to its `%` header, and
  an
  already-separated deck is left untouched.

- **A note saved from the ask tutor shows on the card right away.** Saving a
  note appended it to the deck file but left the in-memory card unchanged, so
  the
  new note only appeared after the deck was re-read (a later session). The just-
  saved lines are now mirrored onto the in-memory card (`Card::append_note`) —
  no
  deck re-read — so returning to the card shows the note immediately, on both
  the
  web and the terminal (the web previously never reflected it; the terminal only
  updated the ask view's recap, not the card on return). On the web, closing the
  ask panel now re-pulls the card state (keeping the reveal position) so the
  saved note appears on return instead of only after a manual page reload.

- **The web ask panel shows the card above the conversation, matching the
  terminal.** The card under discussion (its front + answer) now sits at the top
  as the reference, with Claude's conversation flowing below it (answer under
  question), instead of the card being tucked beneath the conversation — so the
  question you're studying reads above the answer, the same order the TUI
  already
  used. The card and conversation now share one scroll region and the card
  **sticks to the top**, staying in view as a long conversation scrolls under
  it.

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
  generator sometimes writes a multi-file source as `<root>/README.md +
  src/lib.rs`
  (first a full path, the rest relative to it). The exam read the whole string
  as
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
  the "save note" condense), while the CLI conversation still spans the session
  —
  Claude keeps the full context. The card's front + answer are pinned just above
  the input, for easy reference while you type a question. (The terminal ask
  view
  already scoped per card.)

- **The TUI reflows immediately on a terminal resize.** The event loops redrew
  from a size query that could be momentarily stale right after a resize, so the
  screen sometimes stayed unchanged until the next keypress refreshed it. They
  now
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
