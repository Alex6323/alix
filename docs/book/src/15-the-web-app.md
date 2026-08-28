# 15 · The web app

alix is a web app: review, browse, and the exam all run here. `alix` opens a
small local web server and shows you its URL, writing to the **same progress
store** that `alix stats`/`alix list` read: what you grade here is exactly
what they show. It's especially handy on a tablet or
phone, where touch (and images) work naturally.

```sh
alix                                   # the deck picker, at http://127.0.0.1:7777
alix --port 8080                       # a different port
alix --lan                             # reachable from other devices on your network
alix ~/decks-maria --lan --port 7781   # serve one folder as its own scoped root
```

## Choosing decks in the browser

Run `alix` and the page opens the **deck-selection
screen**. **Up / down** move between
decks; a **search box in the header** filters the list (focus it with **`/`**).
Focus a deck and **Learn** it with **Enter** (a facts deck opens a
review, a [trace](13-trace-decks.md) opens a walk) one deck per session. **Browse**
on **`b`** opens a read-only, in-page read-through instead: step the cards with
Prev/Next, Esc to leave. Focusing any deck opens an inline **focus drawer**
beneath it: it shows the deck's frontmatter `description:`, if any, and
a per-card **tier heatmap**: neutral for an untouched card, grey for one merely
**seen** (shown to you at least once), white once **introduced** (correct at least
once), green/yellow/red for a **learned** card by how well you'd recall it right
now, purple once retired, and an unfilled **outline** for a
[sub-card](03-the-deck-format.md#sections-and-sub-cards) still gated on its
parent's graduation. When the deck has a [review order](05-scheduling.md) that
heatmap splits into named regions you can pick to drill (click one or step
through with **← / →**); otherwise it is a single whole-deck bar. On a workspace row instead,
→ enters it, and Esc or Backspace backs out. After a session, **Leave** (on the summary)
or **Esc** (also the footer's **Back** chip while inside a drill-in) returns
here, so you can switch decks without restarting. Every review starts from
this screen; there's no direct deck launch. A focused deck's split
**Depth…** button opens the depth menu ([Scheduling](05-scheduling.md))
without starting it.

A workspace row that has a personal [deadline](08-workspaces.md) set shows a
small chip: a date, days left, and ready percent, colored to flag urgency
inside the last week or past due; the same readout sits inline behind the
title once you drill in. Press **`d`** (or the row's **Ready by…** action) to
set, move, or clear it from an inline date prompt.

A deck whose own progress document cannot be read shows a red **error** line
and refuses to start: reviewing it would write fresh progress over the
document alix could not read. Its siblings are unaffected. `alix doctor`
names the damaged file; fix or remove it (`alix reset <deck>` removes an
unparseable document after confirming) and the row heals on the next
listing.

## Library actions

The picker's **☰ menu** carries six actions that used to be terminal-only:
everything below is an `/api/*` endpoint, so it's also on the wire for other
clients (see `docs/API.md`):

- **Add deck…**: one sheet, three ways in, all landing in a chosen
  destination (the library root or a workspace): **generate** a deck from a
  URL (with optional guidance) the same way `alix generate deck` does, but URL
  sources only, a local-file source stays CLI-only, since a LAN token holder
  must not be able to point the server's AI at the server's own filesystem;
  **import** an Anki `.tsv` or an alix `.md` file; or **receive**: paste a
  wormhole code, or upload a `.zip`.
- **Share…**: sends the focused row (deck, folder, or workspace; the served
  root if nothing's focused) device-to-device over a wormhole code, or
  **download as .zip** as the offline fallback. Personal state (progress,
  recent list, local pacing) stays home either way.
- **Remove from library…**: permanently removes a focused loose deck,
  workspace member, or whole workspace and its Alix-owned progress, frozen
  assets, augmentations, and backup siblings. The sheet first lists the
  stakes, then requires the exact row name. Removing a workspace preserves
  ordinary source files and uninitialized Markdown, so its folder remains if
  either is present. A partial failure stays visible with completed and failed
  artifact labels plus the `alix doctor` recovery step. There is no undo.
- **Reset…**: wipes a row's progress. Gated on typing the row's name back
  exactly, since this can't be undone; needs a focused row.
- **Doctor**: the free environment checks (config, store, decks, backend,
  share) as ✓/!/✗ rows, screenshot-able for handing to whoever set up the
  instance. The costed `--backends` probe stays CLI-only.
- **Pair a device**: a QR of the pairing URL plus the URL itself, to scan
  from a phone or tablet. Needs `--lan`; a localhost-only instance shows a
  hint instead (nothing reachable to scan).

## Augmenting a deck from the picker

Focus a deck and press **`a`** (or its **Augment** button) to open the **Augment
screen**: the browser face of `alix deck augment`. Each of six targets,
[choices](04-review-modes.md), notes, questions, [key points](04-review-modes.md),
format, and order, gets its own card: a short, plain description of what
that augmentation does, a small neutral before/after preview, its coverage
count, and its action. **Generate** fills only the cards a target is still
missing, run as a background model call while the page polls (a spinner shows
it working); **Remove** clears a target, and the order card adds or drops
named topologies. Each card has its own compact guidance input, feeding the
same `--with` steer as the command line, with a kind-specific example as its
placeholder so you can see what a steer is good for; a batch carries each
ticked card's own guidance. It writes the same
`augment/deck-<token>.json` document review reads, so this only saves you the
trip to the terminal.

Cached per-card augmentations are tied to the question and answer they were
generated from. Editing either makes that card reappear as a gap, so its
augmentations regenerate on the next augment run.

The action also works on a **workspace or folder row**: the same screen opens
over all its decks at once, so a Generate fills a target's gaps across every
member, Remove clears it across every member, and an Order generated here is
one workspace-wide pedagogical path.
A workspace additionally gets an **Icon** card: Generate draws (or redraws)
the small emblem shown on its picker row, steered by the card's guidance.

Tick several targets and press **Generate selected** to run them in one batch
(a **Select all** button at the top ticks everything that can run):
it shows a
rough estimate of how many generations that will take, then walks each ticked
card through its own status, queued, generating, done, or failed, as the
batch runs. A target failing doesn't stop the others; a single per-target
**Generate** still works the same way it always did.

On the Claude backend a batch shares **one conversation**: the first target
sends the cards once and every later target refers back to them by index,
which is cheaper and a little faster than re-sending the deck per target.
Other backends, and single-target runs, keep making one self-contained call
per target. A failed target starts a fresh conversation for the rest of the
batch.

The **format** target is a non-destructive reshaping pass: for each plain card
whose answer is poorly shaped (a list crammed into prose, a run-on sentence that
wants to be lines) it caches a tidier front, split answer lines, an optional
note, and a suggested reveal-method: applied at display time without touching the deck
file or card identity. Both review and browse show the reshape, so the two views
match. It's an AI heuristic, so it can miss or produce an unhelpful reshape;
**Remove** clears it with no lasting effect.

## Every check, at every depth, plus the AI features

Every [check](04-review-modes.md) works in the browser, at whichever session
depth you picked: a flip or cloze reveal, a line reveal (it auto-scrolls to
the newest line), a typing Reconstruct check (each line marked ✓/✗ with the
correct answer shown, then you grade), an explain Reconstruct check, and the
multiple-choice pick: a new card's attempt-first on-ramp, or a genuine
Recognize-session question. Pick-one cards submit when you tap an option;
select-all cards let you toggle each answer independently, then submit the
whole set at once. A correct pick offers the quiet "I guessed" undo. A
revealed note uses the same content-column width and text size as the answer
or choices above it. Controls are big tap targets and
follow *your* configured key bindings (the page reads them from the server).
A dim **"N left"** count in the header shows how many cards the session still
holds; it can tick up when a card you missed cools back in for its retry. The
**☰ menu** is context-aware: during review or a trace walk it holds **Ask
Tutor**; on the deck picker, the library actions above plus **keyboard
shortcuts** and **about**, with **Theme…** and **Draw
answers** (a per-device toggle, see below) in both. The ⟳ button (also key
`r`) re-reads your config, so a changed `decks_dir` takes effect without
restarting (scoped `alix <dir>` instances stay pinned to their folder), and
re-fetches workspace icon images, so a regenerated emblem shows without a
reload.

The AI features come along too: the [tutor](10-tutor.md), the
[AI exam](12-the-ai-exam.md), and [trace walks](13-trace-decks.md) all have a web
surface, each running its model call on a background thread while the page polls,
so the single-threaded server never blocks.

## Draw input

A [`input: draw`](04-review-modes.md) card, or a `flip`/`explain` card with
the ☰ menu's **Draw answers** toggle switched on, swaps the usual typed/reveal
input for a small canvas: **Pen** · **Eraser** · **Undo** · **Clear**, then
**Reveal**. The drawing stays on screen (frozen, not editable) while you
self-grade against the card's normal reveal, then it's discarded; nothing you
draw is saved or sent anywhere beyond rendering it in the browser. It's
honored on `flip`/`explain` cards only, and there's no OCR or vision model
reading it back: grading is on you, same as any other self-graded card.

## Themes

The web UI ships a **gallery** of colour themes: the alix **Dark**/**Light**
originals and a **Kids** group (**Sunrise**, **Ocean**, and **Berry**, the
same three themes the [kids app](#kids-mode) offers, so a kid moving up to
the grown-up app can keep the look they grew
attached to), plus crowd-favourite editor/slide palettes
(GitHub, Dracula, Nord, Solarized, Gruvbox, Catppuccin, Tokyo Night, Monokai, One
Dark, Ayu, Rosé Pine, Everforest). Open the **Theme…** popover from the ☰ menu (a
small bar button on the trace walk): a grid grouped Light / Dark that **previews
on a sample card as you hover** and re-themes the whole app when you click one,
remembering your choice in the browser (kept in `localStorage`, not the config).
The palette lives in a shared `theme.css` the
server hosts, so every screen (review, browse, and trace walks) themes together.

## Kids mode

alix can also serve a second, touch-first frontend aimed at kids (roughly
age 10). Set `audience = "kids"` in `[serve]` (see
[Configuration](16-configuration.md)) and point it at a folder an adult has
already set up:

```sh
alix --config kids.toml ~/decks-family --lan --port 7781
```

A **box** is a workspace: the home screen shows the boxes as a grid, tap one
to see its decks with a ⭐ mastery indicator per deck, tap a deck, then pick
that deck's depth: **👆 Tap the answer** (Recognize) or **🗣️ Say it
yourself** (Recall); a caught-up choice disables itself instead of starting
an empty session. On a select-all card, taps mark options instead of
answering, and one **Done** button submits the whole set. Review works the same
way underneath as the regular app (reveal, then the mascot says a short
"why" instead of a bare note, then self-rate) with a **💬 Ask Alix** button
that opens a kid-safe tutor overlay scoped to the current card.

v1 is consumption only: it covers reviewing pre-made boxes at Recognize and
Recall depth, plus the tutor. Augmenting a deck, the AI exam, and traces stay
adult-only for now. An adult prepares a box in the regular web app, then
hands the kid a `kids.toml` and the box to open. It's the same engine and the
same `/api/*` contract underneath, just a different page: self-hosted Baloo 2
type, warmer colours, and no keyboard required.

## Building a client?

The JSON API the web app itself speaks is a documented, client-agnostic
contract: `docs/API.md` in the repository (endpoints, DTO field tables, the
flows, and the stability rules) with every response shape pinned by snapshot
tests. Native or alternative clients build against that file.

## Local by design

The server is deliberately local-only: no accounts, no database. By default it
binds to `127.0.0.1` (this machine only). `--lan` binds all interfaces so another
device on your network can reach it: at startup it prints the pairing URL with
the machine's real IP, plus a scannable QR code, right in the terminal. Serving
with `--lan` auto-generates a **pairing token** and requires it on
`/api/*`, so the network endpoint isn't wide open; pin your own with `--token` or
`[serve] token`. Open the printed `…/?token=…` URL (or scan the QR) and the page
attaches the token for you. AI requests still run the model CLI on the host, so
only use `--lan` on a network you trust. The default port lives in the `[serve]`
config section; `--port` overrides it.

`alix <dir>` serves that folder as a **self-contained scoped root**: its own
catalog, shareable augmentation and assets, plus private per-deck progress and
recent history colocated by default. A workspace `store` setting can move the
private user files without moving shareable material. Several instances run
happily side by side, one per family member, say:
`alix ~/decks-maria --lan --port 7781`.

If a launch misbehaves, `alix doctor` checks the setup (config, progress
store, decks directory, backend CLI) and prints a one-line remedy per problem.
The Doctor sheet also names this instance's local log file.

## Preparing a bug report

Every running server keeps a small local diagnostic history without requiring
a flag. From the adult web app, open **About** and choose **Prepare a bug
report**. From a terminal, run `alix bug-report`; `--out <dir>` chooses where
the archive lands. Both paths use the same local ZIP format. Nothing is
uploaded or sent, so open it and review its plain-text files before attaching
it yourself.

The web archive contains the current instance log and its one rollover; the
CLI collects every instance log so it does not have to guess which server had
the bug. The archive also contains version and platform details, a copy of the
active config with every token and AI prompt override removed, and per-deck
counts keyed only by a SHA-256 hash of the stable deck ID. Home-directory and
user names are redacted. By default, no deck text is included. The CLI's explicit
`--include-deck <path>` option adds that one deck verbatim, including card text
and authored notes, and names it in `report.md`. Personal sidecars, AI prompts,
and AI responses are always excluded. The diagnostic log records minted card
IDs plus content-free panic, AI, parser, and HTTP failure classes; verbose
logging can add operational timings. Only known-safe diagnostic fields enter
the archive. Deleting the archive or either log file does not change decks or
progress.
