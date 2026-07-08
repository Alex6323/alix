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
deck read-only, sitting the AI exam, and walking a [trace](13-trace-decks.md)
are all reached from the web picker rather than as their own commands (see
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

`alix stats`, `alix list`, and `alix reset` each take a **deck file, a plain
folder, or a [workspace](08-workspaces.md)** — a folder or workspace expands to
its member decks, and each deck resolves to the store the launcher would serve
it with (`--store` > its workspace's store > a served folder's own
`progress.json` > the global store).

- `alix stats <target>` — progress overview, completion state, and a
  per-depth due count.
- `alix list <target>` — every card with its Recall/Reconstruct schedule
  state, a ✓ once it's recognized, and its due time.
- `alix reset <target>` — clear progress (`--card`, `--all`; `-y` to
  skip the prompt). On a workspace it also clears the mastered flags and
  virtual cards in the workspace's own store, after one confirmation.

Deck [dependencies](09-dependencies.md) (`% requires:`) are edited by hand in
the deck file — there's no separate command for it.

## The AI features

**`alix generate <source>`** is the one AI-authoring verb. What it makes
follows the source:

- a **web page URL or a local file** → one
  [facts deck](11-generating-decks.md) (`-o/--output`, `--cards`, `--review`,
  `--print`, `--force`; `--workspace <dir>` writes it into that workspace).
- a **directory** → explored first for an
  [ordered learning plan](14-explore.md) scoped by `--goal`: a one-item plan
  becomes a single deck, a bigger plan a **workspace build**, confirmed before
  it runs (`--workspace <dir>` sets the destination, `--title`/`--icon` name
  and brand it). `--plan` prints the plan and stops; `--deck` forces a single
  deck from a directory.
- with **`--trace`** → a [trace](13-trace-decks.md) authored over the source,
  written as a trace deck (`-o/--output`, default `explore.txt`;
  `--workspace <dir>` places it). `--trace --plan` prints a ranked menu of
  suggested traces instead.
- an existing **`% trace:` stub deck** → builds its checkpoints in place.

The rest of the AI-and-deck surface:

- `alix deck augment <deck> --target <...>` — precompute AI augmentations
  (choices, notes, questions, keypoints, format, topology).
- `alix deck import <file.tsv>` — import an Anki TSV export (no model CLI
  needed; `--workspace <dir>` imports into a workspace).
- `alix workspace init <dir>` — scaffold an empty
  [workspace](08-workspaces.md): an `alix.toml` (`--title` names it) and an
  `assets/` dir, no decks — grow it with the `--workspace` flags above.
- Tutor — the Ask button (or `?`) in a session, `Ctrl-N` to save a note
  ([the tutor](10-tutor.md)).

The agentic `generate` runs measure the source size before running and prompt
for confirmation when it's large. Pass `--yes` to skip the prompts in
non-interactive scripts. The [AI exam](12-the-ai-exam.md) runs unattended in
the browser instead, so it can't prompt — it truncates an oversized source and
notes it.

## Sharing

- `alix share <path>` — send a deck file, a plain folder, or a workspace to
  someone over [magic-wormhole](https://magic-wormhole.readthedocs.io) (the
  `wormhole` binary must be installed — `alix doctor` checks). A folder is
  staged first so your personal state stays home: `progress.json`, the recent
  list, `alix.local.toml`, and backup files never travel. Tell the receiver
  the code wormhole prints. No wormhole around? `--zip [--output <path>]`
  writes the same staged copy as a `.zip` to mail or hand over instead.
- `alix receive <code>` — fetch what someone shared. A deck lands in your
  decks directory (`--workspace <dir>` puts it inside a workspace; `--force`
  overwrites a same-named deck); a folder lands under its own name beside
  your other decks and is never overwritten. Personal files that leaked from
  the sender's side are stripped on arrival.

## Config & health

- `alix config` — show the active key bindings; `alix config --init` writes the
  file.
- `alix doctor [dir-or-deck]` — environment health checks, a one-line remedy per
  problem: the config parses, the progress store is readable, the decks dir
  scans, and the backend CLI is on your PATH. Name a **deck file** to lint it
  in depth (syntax, duplicate answers, trace `% at:` locators, and frozen cards
  that have drifted from their `% origin:` source). `--backends` additionally
  probes the configured AI backend end to end (one real, tiny request);
  `--all-backends` probes all four. Report-only — it fixes nothing itself.
- `--config <path>` — use a different config file.
