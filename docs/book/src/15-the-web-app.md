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
screen** — the same list as the terminal picker. Tap a deck to start it (in review
a locked deck won't start; `browse` ignores locking), or tick several checkboxes
and **Confirm** to start them as a merged session. After a session, "Choose other
decks" (on the summary or the ⋮ menu) returns here, so you can switch decks without
restarting. Naming decks on the command line skips this screen.

## Every mode, plus the AI features

All [answer modes](04-review-modes.md) work in the browser — flip, line (it
auto-scrolls to the newest line), typing/fuzzy (each line marked ✓/✗ with the
correct answer shown), and choice (tap an option). Controls are big tap targets and
follow *your* configured key bindings (the page reads them from the server). The ⋮
menu holds **Remove** and **Choose decks**.

The AI features come along too: the [ask-Claude tutor](10-ask-claude.md), the
[AI exam](12-the-ai-exam.md), and [trace walks](13-trace-decks.md) all have a web
surface, each running its Claude call on a background thread while the page polls —
so the single-threaded server never blocks.

## Themes

The web UI ships a **gallery** of colour themes — the alix **Dark**/**Light**
originals and a playful **Kid** theme, plus crowd-favourite editor/slide palettes
(GitHub, Dracula, Nord, Solarized, Gruvbox, Catppuccin, Tokyo Night, Monokai, One
Dark, Ayu, Rosé Pine, Everforest). Open the **Theme…** popover from the ⋮ menu (a
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
