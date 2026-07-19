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
Prev/Next, Esc to leave. Selecting a deck that has a
[topology](05-scheduling.md) opens an inline **focus drawer** beneath it: choose
which topology orders the session, and pick a region to drill (click it or step
through with **← / →**) its strength heatmap and the number of cards **due** in
it shown as you go ("Whole deck" is the default). On a workspace row instead,
← / → enter and leave it. After a session, "Choose other decks" (on the summary)
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

## Library actions

The picker's **☰ menu** carries five actions that used to be terminal-only:
everything below is an `/api/*` endpoint, so it's also on the wire for other
clients (see `docs/API.md`):

- **Add deck…**: one sheet, three ways in, all landing in a chosen
  destination (the library root or a workspace): **generate** a deck from a
  URL (with optional guidance) the same way `alix generate` does, but URL
  sources only, a local-file source stays CLI-only, since a LAN token holder
  must not be able to point the server's AI at the server's own filesystem;
  **import** an Anki `.tsv` or an alix `.md` file; or **receive**: paste a
  wormhole code, or upload a `.zip`.
- **Share…**: sends the focused row (deck, folder, or workspace; the served
  root if nothing's focused) device-to-device over a wormhole code, or
  **download as .zip** as the offline fallback. Personal state (progress,
  recent list, local pacing) stays home either way.
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
format, and topology, gets its own card: a short, plain description of what
that augmentation does, a small neutral before/after preview, its coverage
count, and its action. **Generate** fills only the cards a target is still
missing, run as a background model call while the page polls (a spinner shows
it working); **Remove** clears a target, and the topology card adds or drops
named topologies. Each card has its own compact guidance input, feeding the
same `--with` steer as the command line, with a kind-specific example as its
placeholder so you can see what a steer is good for; a batch carries each
ticked card's own guidance. It writes the same `augment.json` review reads, so
this only saves you the trip to the terminal.

The action also works on a **workspace or folder row**: the same screen opens
over all its decks at once, so a Generate fills a target's gaps across every
member, Remove clears it across every member, and an Order generated here is
one workspace-wide pedagogical path (a workspace review session picks it up).
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
Recognize-session question (tap an option; a correct pick offers the quiet "I
guessed" undo). Controls are big tap targets and
follow *your* configured key bindings (the page reads them from the server).
A dim **"N left"** count in the header shows how many cards the session still
holds; it can tick up when a card you missed cools back in for its retry. The
**☰ menu** is context-aware: during review or a trace walk it holds **Ask
Tutor**; on the deck picker, the library actions above plus **keyboard
shortcuts**, **refresh decks**, and **about**, with **Theme…** and **Draw
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
to see its decks and a ⭐ mastery indicator per deck (display-only, the row
itself isn't tappable), then pick a depth for the whole box: **👆 Tap the
answer** (Recognize) or **🗣️ Say it yourself** (Recall); a caught-up choice
disables itself instead of starting an empty session. Review works the same
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
catalog, with its own `progress.json` and `recent.json` kept inside the folder.
Several instances run happily side by side, one per family member, say:
`alix ~/decks-maria --lan --port 7781`.

If a launch misbehaves, `alix doctor` checks the setup (config, progress
store, decks directory, backend CLI) and prints a one-line remedy per problem.
