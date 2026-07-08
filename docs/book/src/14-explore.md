# 14 · Generate a workspace — goals & curricula

[`alix generate --trace --plan`](13-trace-decks.md) lists central *traces*.
Pointing **`alix generate`** at a **directory** goes a layer up: give it a goal
and it explores the source first — one AI planning pass — and prints an ordered
**learning plan**: the facts decks *and* traces worth authoring to reach that
goal, dependency-ordered.

```sh
alix generate . --plan                                      # a plan to understand the whole source
alix generate . --plan --goal "how review scheduling works" # a narrow goal → a focused subset
```

Each item is tagged `[trace]` or `[deck]`, chosen by the **shape of the
knowledge**: a *path* you predict hop by hop becomes a trace; a *table of facts* —
a config's knobs, a store's on-disk format — becomes a facts deck. Each carries its
`% requires:` prerequisites (the list is a valid dependency order, foundations
first) and a `% source:` scope. The `--goal` scopes coverage: a broad goal spans
every subsystem; a narrow one collapses to its slice and traces it in more detail.
`--plan` is read-only — it prints the plan and stops, so you can author the items
yourself (`alix generate` [a trace](13-trace-decks.md) or
[a facts deck](11-generating-decks.md) per item).

## Building the workspace

```sh
alix generate . --goal "how review scheduling works" --workspace ~/decks/scheduling/
```

Without `--plan`, the plan's size decides. A one-item plan collapses to a single
facts deck (`--deck` forces that from the start, skipping the plan pass). More
items become a **workspace build**: the plan prints, `alix` confirms
(`Build N items into <dir>? [y/N]` — `-y` skips it), then goes all the way — it
explores the source **once** and reuses that single session to fill every item —
predict-verify checkpoints for the traces, fact cards for the decks — so the
[workspace](08-workspaces.md) comes out review-ready in one command: an
`alix.toml` (carrying the goal; `--title` names it) and one deck per item — a
`% trace:` deck per trace, a `% title:` facts deck per deck — wired together
with `% requires:` so they unlock in dependency order, each `% source:` pointing
back at the real source. Writing the whole set from one understanding keeps the
items **coherent** (each builds on its prerequisites instead of repeating them).
As a final step it [freezes the cited excerpts](13-trace-decks.md) of every
cited deck — traces and fact decks with
[`% at:` citations](06-cloze-direction-images.md#source-citations) alike — into
the workspace's `assets/`, so it's self-contained and its locators never drift.

The destination is `--workspace <dir>`, defaulting to a folder named after the
source under your decks directory. (It refuses a non-empty folder unless you
pass `--force`.)

This is the tool's high-water mark: name what you want to understand, and `alix`
assembles a dependency-ordered curriculum of facts and traces — gated by
[mastery](12-the-ai-exam.md) — that you climb.

## The explore walk — `--trace`

Before you even know what to trace, `alix generate <source> --trace` builds a
short **tour of the source's shape**, written as a trace deck: you predict what
kind of program it is (from the manifest), its domain nouns (from the module
list), how it's driven (the entry point), its spine (the central file), and
finally the first paths worth tracing — each hop revealing the real lines. It's
written to a file (`-o`, default `explore.txt`; `--workspace` places it inside a
workspace), and you walk it from the [web picker](15-the-web-app.md): run `alix`
and pick it.
