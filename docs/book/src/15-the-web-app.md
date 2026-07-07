# 15 · The web app

alix is a web app: review, browse, and the exam all run here. `alix` opens a
small local web server and shows you its URL, writing to the **same progress
store** regardless of how you launch it — so what you grade here is exactly
what `alix stats`/`alix list` show. It's especially handy on a tablet or
phone, where touch (and images) work naturally.

```sh
alix review rust.txt                        # open http://127.0.0.1:7777
alix review rust.txt --serve --port 8080    # a different port
alix review rust.txt --serve --lan          # reachable from other devices on your network
alix                                         # no decks → pick them in the browser
```

## Choosing decks in the browser

Run `alix` without naming decks and the page opens the **deck-selection
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
or **Esc** returns here, so you can switch decks without restarting. Naming a
deck on the command line skips this screen.

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
deck picker, **keyboard shortcuts**, **refresh decks**, and **about** — with
**Theme…** and **Draw answers** (a per-device toggle, see below) in both.

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

## Local by design

The server is deliberately local-only — no accounts, no database. By default it
binds to `127.0.0.1` (this machine only). `--lan` binds all interfaces so another
device on your network can reach it at `http://<your-ip>:<port>`. Serving with
`--lan` auto-generates a **pairing token** (printed at startup) and requires it on
`/api/*`, so the network endpoint isn't wide open; pin your own with `--token` or
`[serve] token`. Open the printed `…/?token=…` URL in a browser and the page
attaches the token for you. AI requests still run the model CLI on the host, so
only use `--lan` on a network you trust. The default port lives in the `[serve]`
config section; `--port` overrides it.
