# 15 · The web app — `--serve`

Everything `alix` does in the terminal it can also do in a browser. Add `--serve`
to `review` or `browse` (or run `alix --serve` with no decks) and it runs the same
session logic over a tiny local web server, writing to the **same progress store**
— so a card you grade or remove in the browser shows up on the command line and
vice versa. It's handy on a tablet or phone, where touch (and images) beat a TUI.

```sh
alix review rust.txt --serve              # open http://127.0.0.1:7777
alix review rust.txt --serve --port 8080
alix review rust.txt --serve --lan        # reachable from other devices on your network
alix browse rust.txt --serve
alix --serve                              # no decks → pick them in the browser
```

## Choosing decks in the browser

Run `--serve` without naming decks and the page opens the **deck-selection
screen** — the same list as the terminal picker. **Up / down** move between
decks; a **search box in the header** filters the list (focus it with **`/`**).
Focus a deck and **Learn** it with **`l`** (or Enter) — a facts deck opens a
review, a [trace](13-trace-decks.md) opens a walk — one deck per session (a locked
deck won't start; `browse` ignores locking, and is on **`b`**). Selecting a deck that has a
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
the cards a target is still missing, run as a background Claude call while the
page polls (a spinner shows it working); **Remove** clears a target, and the
topology row adds or drops named ones. A guidance box feeds the same `--with`
steer as the command line. It writes the same `augment.json` review reads, so
this only saves you the trip to the terminal. The action shows on decks, not
workspaces.

The **format** target is a non-destructive reshaping pass: for each plain card
whose answer is poorly shaped (a list crammed into prose, a run-on sentence that
wants to be lines) it caches a tidier front, split answer lines, an optional
note, and a suggested mode — applied at display time without touching the deck
file or card identity. Both review and browse show the reshape, so the two views
match. It's an AI heuristic, so it can miss or produce an unhelpful reshape;
**Remove** clears it with no lasting effect.

## Every mode, plus the AI features

All [answer modes](04-review-modes.md) work in the browser — flip, line (it
auto-scrolls to the newest line), typing/fuzzy (each line marked ✓/✗ with the
correct answer shown), and choice (tap an option). Controls are big tap targets and
follow *your* configured key bindings (the page reads them from the server). The
**☰ menu** is context-aware: during review it holds **Ask Claude** and **Remove
card**; on the deck picker, **keyboard shortcuts**, **refresh decks**, and
**about** — with **Theme…** in both.

The AI features come along too: the [ask-Claude tutor](10-ask-claude.md), the
[AI exam](12-the-ai-exam.md), and [trace walks](13-trace-decks.md) all have a web
surface, each running its Claude call on a background thread while the page polls —
so the single-threaded server never blocks.

## Themes

The web UI ships a **gallery** of colour themes — the alix **Dark**/**Light**
originals and a playful **Kid** theme, plus crowd-favourite editor/slide palettes
(GitHub, Dracula, Nord, Solarized, Gruvbox, Catppuccin, Tokyo Night, Monokai, One
Dark, Ayu, Rosé Pine, Everforest). Open the **Theme…** popover from the ☰ menu (a
small bar button on the trace walk): a grid grouped Light / Dark that **previews
the whole UI live as you hover**, and remembers your choice in the browser (kept
in `localStorage`, not the config). The palette lives in a shared `theme.css` the
server hosts, so every screen — review, browse, and trace walks — themes together.

## Local by design

The server is deliberately local-only — no accounts, no database. By default it
binds to `127.0.0.1` (this machine only). `--lan` binds all interfaces so another
device on your network can reach it at `http://<your-ip>:<port>`, but there's **no
authentication**, and AI requests run `claude` on the host — so only use `--lan` on
a network you trust. The default port lives in the `[serve]` config section;
`--port` overrides it.
