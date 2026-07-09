# 15 · The web app

alix is a web app: review, browse, and the exam all run here. `alix` opens a
small local web server and shows you its URL, writing to the **same progress
store** that `alix stats`/`alix list` read — what you grade here is exactly
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
Focus a deck and **Learn** it with **Enter** — a facts deck opens a
review, a [trace](13-trace-decks.md) opens a walk — one deck per session. **Browse**
on **`b`** opens a read-only, in-page read-through instead — step the cards with
Prev/Next, Esc to leave. Selecting a deck that has a
[topology](05-scheduling.md) opens an inline **focus drawer** beneath it: choose
which topology orders the session, and pick a region to drill — click it or step
through with **← / →** — its strength heatmap and the number of cards **due** in
it shown as you go ("Whole deck" is the default). On a workspace row instead,
← / → enter and leave it. After a session, "Choose other decks" (on the summary)
or **Esc** returns here, so you can switch decks without restarting. Every
review starts from this screen — there's no direct deck launch.

## Library actions

The picker's **☰ menu** carries five actions that used to be terminal-only —
everything below is an `/api/*` endpoint, so it's also on the wire for other
clients (see `docs/API.md`):

- **Add deck…** — one sheet, three ways in, all landing in a chosen
  destination (the library root or a workspace): **generate** a deck from a
  URL (with optional guidance) the same way `alix generate` does, but URL
  sources only — a local-file source stays CLI-only, since a LAN token holder
  must not be able to point the server's AI at the server's own filesystem;
  **import** an Anki `.tsv` or an alix `.txt` file; or **receive** — paste a
  wormhole code, or upload a `.zip`.
- **Share…** — sends the focused row (deck, folder, or workspace; the served
  root if nothing's focused) device-to-device over a wormhole code, or
  **download as .zip** as the offline fallback. Personal state (progress,
  recent list, local pacing) stays home either way.
- **Reset…** — wipes a row's progress. Gated on typing the row's name back
  exactly, since this can't be undone; needs a focused row.
- **Doctor** — the free environment checks (config, store, decks, backend,
  share) as ✓/!/✗ rows, screenshot-able for handing to whoever set up the
  instance. The costed `--backends` probe stays CLI-only.
- **Pair a device** — a QR of the pairing URL plus the URL itself, to scan
  from a phone or tablet. Needs `--lan`; a localhost-only instance shows a
  hint instead (nothing reachable to scan).

## Augmenting a deck from the picker

Focus a deck and press **`a`** (or its **Augment** button) to open the **Augment
screen** — the browser face of `alix deck augment`. It shows what the deck's
augmentation cache already holds, one row per target
([choices](04-review-modes.md), notes, questions, [key points](04-review-modes.md),
format) with a coverage bar, alongside its topologies. **Generate** fills only
the cards a target is still missing, run as a background model call while the
page polls (a spinner shows it working); **Remove** clears a target, and the
topology row adds or drops named ones. A guidance box feeds the same `--with`
steer as the command line. It writes the same `augment.json` review reads, so
this only saves you the trip to the terminal. The action shows on decks, not
workspaces.

The **format** target is a non-destructive reshaping pass: for each plain card
whose answer is poorly shaped (a list crammed into prose, a run-on sentence that
wants to be lines) it caches a tidier front, split answer lines, an optional
note, and a suggested reveal-method — applied at display time without touching the deck
file or card identity. Both review and browse show the reshape, so the two views
match. It's an AI heuristic, so it can miss or produce an unhelpful reshape;
**Remove** clears it with no lasting effect.

## Every check, at every depth, plus the AI features

Every [check](04-review-modes.md) works in the browser, at whichever session
depth you picked — a flip or cloze reveal, a line reveal (it auto-scrolls to
the newest line), a typing Reconstruct check (each line marked ✓/✗ with the
correct answer shown, then you grade), an explain Reconstruct check, and the
multiple-choice pick — a new card's attempt-first on-ramp, or a genuine
Recognize-session question (tap an option; a correct pick offers the quiet "I
guessed" undo). Controls are big tap targets and
follow *your* configured key bindings (the page reads them from the server). The
**☰ menu** is context-aware: during review it holds **Ask Tutor**; on the
deck picker, the library actions above plus **keyboard shortcuts**, **refresh
decks**, and **about** — with **Theme…** and **Draw answers** (a per-device
toggle, see below) in both. The ⟳ button re-reads your config, so a changed
`decks_dir` takes effect without restarting — scoped `alix <dir>` instances
stay pinned to their folder.

The AI features come along too: the [tutor](10-tutor.md), the
[AI exam](12-the-ai-exam.md), and [trace walks](13-trace-decks.md) all have a web
surface, each running its model call on a background thread while the page polls —
so the single-threaded server never blocks.

## Draw input

A [`% input: draw`](04-review-modes.md) card, or a `flip`/`explain` card with
the ☰ menu's **Draw answers** toggle switched on, swaps the usual typed/reveal
input for a small canvas: **Pen** · **Eraser** · **Undo** · **Clear**, then
**Reveal**. The drawing stays on screen — frozen, not editable — while you
self-grade against the card's normal reveal, then it's discarded; nothing you
draw is saved or sent anywhere beyond rendering it in the browser. It's
honored on `flip`/`explain` cards only, and there's no OCR or vision model
reading it back — grading is on you, same as any other self-graded card.

## Themes

The web UI ships a **gallery** of colour themes — the alix **Dark**/**Light**
originals and a playful **Kid** theme, plus crowd-favourite editor/slide palettes
(GitHub, Dracula, Nord, Solarized, Gruvbox, Catppuccin, Tokyo Night, Monokai, One
Dark, Ayu, Rosé Pine, Everforest). Open the **Theme…** popover from the ☰ menu (a
small bar button on the trace walk): a grid grouped Light / Dark that **previews
on a sample card as you hover** and re-themes the whole app when you click one,
remembering your choice in the browser (kept in `localStorage`, not the config).
The palette lives in a shared `theme.css` the
server hosts, so every screen — review, browse, and trace walks — themes together.

## Building a client?

The JSON API the web app itself speaks is a documented, client-agnostic
contract: `docs/API.md` in the repository — endpoints, DTO field tables, the
flows, and the stability rules — with every response shape pinned by snapshot
tests. Native or alternative clients build against that file.

## Local by design

The server is deliberately local-only — no accounts, no database. By default it
binds to `127.0.0.1` (this machine only). `--lan` binds all interfaces so another
device on your network can reach it: at startup it prints the pairing URL with
the machine's real IP — plus a scannable QR code, right in the terminal. Serving
with `--lan` auto-generates a **pairing token** and requires it on
`/api/*`, so the network endpoint isn't wide open; pin your own with `--token` or
`[serve] token`. Open the printed `…/?token=…` URL (or scan the QR) and the page
attaches the token for you. AI requests still run the model CLI on the host, so
only use `--lan` on a network you trust. The default port lives in the `[serve]`
config section; `--port` overrides it.

`alix <dir>` serves that folder as a **self-contained scoped root**: its own
catalog, with its own `progress.json` and `recent.json` kept inside the folder.
Several instances run happily side by side — one per family member, say:
`alix ~/decks-maria --lan --port 7781`.

If a launch misbehaves, `alix doctor` checks the setup — config, progress
store, decks directory, backend CLI — and prints a one-line remedy per problem.
