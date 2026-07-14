# 8 · Workspaces

As your decks multiply, you'll want to treat a *cluster* of them as a unit — all
your Spanish decks, or every deck about one codebase. A **workspace** is that
unit: a folder of decks reviewed together, sharing settings and a name, with its
own progress.

## Making a workspace

Any folder of `.txt` decks becomes a workspace the moment you drop a **`alix.toml`**
in it — a scoped version of the global config file. It sets a title and a
`[defaults]` table of directives that every deck in the folder inherits:

```toml
# ~/decks/spanish/alix.toml
title = "Spanish"

[defaults]
direction = "both"
reveal = "line"
```

Starting from nothing instead? `alix workspace init <dir>` (`--title` to name
it) scaffolds an empty workspace — an `alix.toml`, an `alix.local.toml`, and an
`assets/` folder, no decks yet. Both TOML files come fully commented, each key
explained inline, so they document themselves. Grow the workspace with
[`alix generate … --workspace <dir>`](11-generating-decks.md) or
`alix deck import … --workspace <dir>` — also available from the web UI's ☰
menu's **Add deck…** sheet. Dependencies (`% requires:`) are still edited by
hand in the deck files.

Now open the cluster and drill its members one at a time:

```sh
alix ~/decks/spanish/
```

## Shared directives

The `[defaults]` keys are exactly the deck-directive names from
[the deck format](03-the-deck-format.md) — `reveal`, `direction`, `order`, and
the rest. They fill in only what a deck *doesn't* set for itself, so the
precedence is one level deeper than before:

> card `%` > deck `%` > **workspace `[defaults]`** > built-in default

Set `direction = "both"` once for the whole folder, and a single irregular deck
can still override it with its own `% direction: forward`. It's the same directive
system from chapter 3, just sourced from one more place.

## Personal pacing — `alix.local.toml`

The `alix.toml` is shared: it travels with the workspace when you hand it to
someone. Your **personal** review pacing doesn't belong there. Drop an
`alix.local.toml` beside it to override the global `[review]` config — FSRS
`retention`, `retire_after`, `acquire_cooldown` — for this workspace's decks
only:

```toml
# ~/decks/spanish/alix.local.toml
[review]
retention = 0.95         # see these cards more often
retire_after = "never"   # never let them retire
```

It uses the same `[review]` keys as the [config file](16-configuration.md), and
it's kept separate from `alix.toml` on purpose — so it stays yours and never
travels when you share the workspace. A missing or malformed one is simply
ignored.

The session depth (Recognize/Recall/Reconstruct) isn't a workspace setting —
it's picked per session, the same as for a loose deck (see
[Reveal & session depths](04-review-modes.md)).

## Its own progress

A workspace keeps its progress **inside the folder**, in a `progress.json` next to
the decks (override the location with a `store = "..."` line in the `alix.toml`),
separate from the store your loose decks use (your decks directory's own `progress.json`). That makes a workspace a
**self-contained, portable unit**: its decks, its `assets/` (frozen trace
excerpts — covered with traces later), and its history all live in one folder you
can move, copy, or share, with its progress isolated from everything else. Decks
outside any workspace use your decks directory's store; the CLI commands
(`alix stats`/`list`/`reset`) take a deck file, a plain folder, **or a
workspace** — a folder or workspace expands to its member decks, each resolved
against the same store the launcher would serve it with (`--store <path>` still
overrides).

## In the picker

Folders show up in the picker in two flavors: a folder *with* a
`alix.toml` appears under **Workspaces**, one *without* as a plain **Folder**.
Opening either drills in to its decks, drawn as a **dependency tree** — each deck
nested under the prerequisite that gates it, foundations at the roots (the
[next chapter](09-dependencies.md)). Each row is badged `· deck ·` or `· trace ·`,
and the drill-in is a single-launch list: `Enter` on a facts deck reviews it,
`Enter` on a trace **walks** it. Typing a filter flattens the tree to a plain
search.

In the **web** picker, a workspace can show a small **emblem** in place of the
chevron, so a long list of similar-named workspaces is quicker to scan. Drop an
image in the workspace's `assets/` and point `icon = "assets/<file>"` at it in the
`alix.toml` (or just name it `assets/icon.{svg,png,jpg}` and skip the key); an SVG
is tinted to the active theme, a raster shows as-is. When you build a workspace
with `alix generate <source> --workspace <dir>`, the model draws an abstract SVG emblem from
the topic automatically, unless you pass `--icon <file>`.

`alix <dir>` serves a workspace directly: the picker opens drilled into that
view, scoped to the folder and its own store, routing each
member to the right experience — a facts deck to a review, a trace to a walk — and
returning you to the picker when you finish one. (A session is one deck file, so
a whole workspace is never reviewed at once; open it and pick a member.)

A folder without a manifest serves the same way with `alix <folder>`; it
just applies no shared directives.

## Sharing a workspace

A workspace is a self-contained folder, so sharing one is sending the folder.
`alix share <dir>` does that over magic-wormhole with the personal files
(progress, recent list, `alix.local.toml`) left home; the other side runs
`alix receive <code>` and gets it beside their own decks, ready to serve with
`alix <dir>`. Precomputed augmentations (`augment.json`) travel — the AI
content comes along, the progress doesn't. Also available from the web UI's ☰
menu (**Share…** / **Add deck…** → Receive), with a `.zip` download/upload
fallback when neither side has `wormhole` installed.

## Titles

A `title` in the `alix.toml` — or a `% title:` directive on a single deck — gives
a display name, shown in the picker, the session header, `alix list`, and `alix
stats`, instead of the file name. It's display-only: you still refer to decks by
file path on the command line, and a title never affects a card's identity.
