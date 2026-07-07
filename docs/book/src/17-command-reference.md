# 17 · Command reference

A quick index of the `alix` commands. Each links to the chapter that covers it in
depth, where there is one. Run any command with `--help` for its full flags.

## Reviewing

- `alix` — open the web deck [picker](02-getting-started.md) (recent +
  `~/decks`), printing its URL.
- `alix <deck>` / `alix review <deck>` — review one deck's due cards, in the
  browser.
- `alix workspace <dir>` — open a [workspace](08-workspaces.md), routing each
  member you pick to a review or a walk (a workspace is reviewed member-by-member,
  never merged).

Browsing a deck read-only, and sitting the AI exam, are both reached from the
web picker rather than as their own commands (see
[the web app](15-the-web-app.md)).

Common flags: `--topology <name>` / `--region <name>`
([scheduling](05-scheduling.md)), `--cram`, `--new N`, `--limit N`,
`--level <recognize|recall|reconstruct>` (default: the deck's own last-used
level), and `--serve` / `--port` / `--lan` ([the web app](15-the-web-app.md)).
How each card is checked comes from its `% reveal:` combined with the
session's level ([reveal & session levels](04-review-modes.md)), not a flag.

## Progress

- `alix stats <deck>...` — progress overview, completion state, and a
  per-level due count.
- `alix list <deck>...` — every card with its Recall/Reconstruct schedule
  state, a ✓ once it's recognized, and its due time.
- `alix reset <deck>...` — clear progress (`--card`, `--all`; `-y` to
  skip the prompt). Non-interactive: name a deck or pass `--card`/`--all`.
- `alix deck check <deck>...` — lint a deck (syntax, duplicate cards, trace `% at:`
  locators, and frozen cards that have drifted from their `% origin:` source).

Deck [dependencies](09-dependencies.md) (`% requires:`) are edited by hand in
the deck file — there's no separate command for it.

## The AI features

- `alix backend check [--all]` — health probe: sends a short request to the
  configured backend (or all four with `--all`) and reports whether each is
  installed, signed in, and responding.
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

## Config

- `alix config` — show the active key bindings; `alix config --init` writes the
  file.
- `--config <path>` — use a different config file.
