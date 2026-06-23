# 17 · Command reference

A quick index of the `alix` commands. Each links to the chapter that covers it in
depth, where there is one. Run any command with `--help` for its full flags.

## Reviewing

- `alix` — open the deck [picker](02-getting-started.md) (recent + `~/decks`).
- `alix <deck>...` — review due cards; several decks merge into one session.
- `alix review <deck-or-folder>...` — the same, explicit, and how you review a
  [workspace](08-workspaces.md) folder.
- `alix browse <deck>...` — read through cards with no grading or scheduling.
- `alix workspace <dir>` — open a workspace, routing each member to a review or a
  walk.

Common flags: `--mode <m>` ([modes](04-review-modes.md)), `--scheduler <s>`
([scheduling](05-scheduling.md)), `--cram`, `--new N`, `--limit N`, `--max-typos N`,
and `--serve` / `--port` / `--lan` ([the web app](15-the-web-app.md)).

## Progress

- `alix stats <deck>...` — progress overview and completion state.
- `alix list <deck>...` — every card with its stage and due time.
- `alix reset <deck>...` — clear progress (`--card`, `--cards`, `--all`; `-y` to
  skip the prompt).
- `alix check <deck>...` — lint a deck (syntax, duplicate cards, trace `% at:` locators).
- `alix deps <deck>` (alias `require`) — edit `% requires:` with a checkbox picker
  ([dependencies](09-dependencies.md)).

## The AI features

- `alix deck <url-or-path>` — [generate a facts deck](11-generating-decks.md).
- `alix import <file.tsv>` — import an Anki TSV export (no Claude needed).
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
