# 17 · Command reference

A quick index of the `alix` commands. Each links to the chapter that covers it in
depth, where there is one. Run any command with `--help` for its full flags.

## Reviewing

- `alix` — serve the web app: the deck [picker](02-getting-started.md) over
  your decks directory (`~/decks`), printing its URL.
- `alix <dir>` — serve that folder as a **self-contained scoped root**: its own
  catalog, with its own `progress.json` and `recent.json` inside the folder, so
  several instances can run side by side. A [workspace](08-workspaces.md) dir
  opens the picker drilled into it, with its own store.

Every review starts from the picker — there's no direct deck launch. Browsing a
deck read-only, and sitting the AI exam, are both reached from the
web picker rather than as their own commands (see
[the web app](15-the-web-app.md)).

The launcher's flags — its only ones: `--lan` / `--port` / `--token`
([the web app](15-the-web-app.md)), `--new N` / `--limit N` (session pacing,
overriding the `[review]` config), and `--config <path>`. The session depth is
picked in the picker's split Learn ▾ menu, a topology or region in its focus
drawer ([scheduling](05-scheduling.md)), and the card order is the deck's
`% order:` directive.
How each card is checked comes from its `% reveal:` combined with the
session's depth ([reveal & session depths](04-review-modes.md)), not a flag.

## Progress

- `alix stats <deck>...` — progress overview, completion state, and a
  per-depth due count.
- `alix list <deck>...` — every card with its Recall/Reconstruct schedule
  state, a ✓ once it's recognized, and its due time.
- `alix reset <deck>...` — clear progress (`--card`, `--all`; `-y` to
  skip the prompt). Non-interactive: name a deck or pass `--card`/`--all`.
- `alix deck check <deck>...` — lint a deck (syntax, duplicate cards, trace `% at:`
  locators, and frozen cards that have drifted from their `% origin:` source).

Deck [dependencies](09-dependencies.md) (`% requires:`) are edited by hand in
the deck file — there's no separate command for it.

## The AI features

- `alix deck generate <url-or-path>` — [generate a facts deck](11-generating-decks.md).
- `alix deck augment <deck> --target <...>` — precompute AI augmentations
  (choices, notes, questions, keypoints, format, topology).
- `alix import <file.tsv>` — import an Anki TSV export (no model CLI needed).

The agentic commands (`deck generate`, `trace --build`, `explore`)
measure the source size before running and prompt for confirmation when it's
large. Pass `--yes` to skip the prompt in non-interactive scripts. The
[AI exam](12-the-ai-exam.md) runs unattended in the browser instead, so it
can't prompt — it truncates an oversized source and notes it.
- `alix trace <deck>` — walk a [trace](13-trace-decks.md) in the terminal
  (`--build`, `--suggest`, `--grade`, `--map`); the same trace also walks from
  the web picker.
- `alix explore <source>` — an [ordered learning plan](14-explore.md) (`--goal`,
  `--into`, `--build`, `--walk`).
- Tutor — the Ask button (or `?`) in a session, `Ctrl-N` to save a note
  ([the tutor](10-tutor.md)).

## Config & health

- `alix config` — show the active key bindings; `alix config --init` writes the
  file.
- `alix doctor [dir]` — environment health checks, a one-line remedy per
  problem: the config parses, the progress store is readable, the decks dir
  scans (broken decks point at `alix deck check`), and the backend CLI is on
  your PATH. `--backends` additionally probes the configured AI backend end to
  end (one real, tiny request); `--all-backends` probes all four. Report-only —
  it fixes nothing itself.
- `--config <path>` — use a different config file.
