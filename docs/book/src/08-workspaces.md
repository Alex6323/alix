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
mode = "typing"
```

Now open the cluster and drill its members one at a time:

```sh
alix workspace ~/decks/spanish/
```

## Shared directives

The `[defaults]` keys are exactly the deck-directive names from
[the deck format](03-the-deck-format.md) — `mode`, `direction`, `scheduler`, and
the rest. They fill in only what a deck *doesn't* set for itself, so the
precedence is one level deeper than before:

> CLI flag > card `%` > deck `%` > **workspace `[defaults]`** > built-in default

Set `direction = "both"` once for the whole folder, and a single irregular deck
can still override it with its own `% direction: forward`. It's the same directive
system from chapter 3, just sourced from one more place.

## Its own progress

A workspace keeps its progress **inside the folder**, in a `progress.json` next to
the decks (override the location with a `store = "..."` line in the `alix.toml`),
separate from the global store that loose decks share. That makes a workspace a
**self-contained, portable unit**: its decks, its `assets/` (frozen trace
excerpts — covered with traces later), and its history all live in one folder you
can move, copy, or share, with its progress isolated from everything else. Decks
outside any workspace keep using the global store; `--store <path>` overrides
either.

## In the picker

Folders show up in the picker (terminal and web) in two flavors: a folder *with* a
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
with `alix explore --into <dir> --build`, Claude draws an abstract SVG emblem from
the topic automatically, unless you pass `--icon <file>`.

`alix workspace <dir>` jumps straight into that drill-in view, routing each
member to the right experience — a facts deck to a review, a trace to a walk — and
returning you to the picker when you finish one. (`alix review <dir>` no longer
works on a folder — a session is one deck file, so a whole workspace is never
reviewed at once; open it and pick a member instead.)

A folder without a manifest is still reviewable with `alix review <folder>`; it
just applies no shared directives.

## Titles

A `title` in the `alix.toml` — or a `% title:` directive on a single deck — gives
a display name, shown in the picker, the session header, `alix list`, and `alix
stats`, instead of the file name. It's display-only: you still refer to decks by
file path on the command line, and a title never affects a card's identity.
