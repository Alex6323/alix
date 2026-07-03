# 17 · Command reference

A quick index of the `alix` commands. Each links to the chapter that covers it in
depth, where there is one. Run any command with `--help` for its full flags.

## Reviewing

- `alix` — open the deck [picker](02-getting-started.md) (recent + `~/decks`).
- `alix <deck>` / `alix review <deck>` — review one deck's due cards.
- `alix browse <deck>` — read through one deck's cards, no grading or scheduling.
- `alix workspace <dir>` — open a [workspace](08-workspaces.md), routing each
  member you pick to a review or a walk (a workspace is reviewed member-by-member,
  never merged).

Common flags: `--mode <m>` ([modes](04-review-modes.md)), `--scheduler <s>` /
`--topology <name>` / `--region <name>` ([scheduling](05-scheduling.md)),
`--cram`, `--new N`, `--limit N`, `--max-typos N`, and `--serve` / `--port` /
`--lan` ([the web app](15-the-web-app.md)).

## Progress

- `alix stats <deck>...` — progress overview and completion state.
- `alix list <deck>...` — every card with its stage and due time.
- `alix reset <deck>...` — clear progress (`--card`, `--cards`, `--all`; `-y` to
  skip the prompt).
- `alix deck check <deck>...` — lint a deck (syntax, duplicate cards, trace `% at:`
  locators, and frozen cards that have drifted from their `% origin:` source).
- `alix deps <deck>` (alias `require`) — edit `% requires:` with a checkbox picker
  ([dependencies](09-dependencies.md)).

## The AI features

- `alix backend check [--all]` — health probe: sends a short request to the
  configured backend (or all four with `--all`) and reports whether each is
  installed, signed in, and responding.
- `alix deck generate <url-or-path>` — [generate a facts deck](11-generating-decks.md).
- `alix deck augment <deck> --target <...>` — precompute AI augmentations
  (choices, notes, questions, keypoints, format, topology).
- `alix import <file.tsv>` — import an Anki TSV export (no model CLI needed).

The agentic commands (`deck generate`, `exam`, `trace --build`, `explore`)
measure the source size before running and prompt for confirmation when it's
large. Pass `--yes` to skip the prompt in non-interactive scripts.
- `alix exam <deck>` — the [AI exam](12-the-ai-exam.md) (`--questions`,
  `--strictness`).
- `alix trace <deck>` — walk a [trace](13-trace-decks.md) (`--build`, `--suggest`,
  `--grade`, `--map`, `--serve`).
- `alix explore <source>` — an [ordered learning plan](14-explore.md) (`--goal`,
  `--into`, `--build`, `--walk`).
- Ask-Claude — `?` in a session, `Ctrl-N` to save a note ([the tutor](10-ask-claude.md)).

## Config

- `alix config` — show the active key bindings; `alix config --init` writes the
  file.
- `--config <path>` — use a different config file.
