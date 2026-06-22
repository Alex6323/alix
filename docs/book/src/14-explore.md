# 14 · Explore — goals & curricula

`flash trace --suggest` lists central *traces*. **`flash explore`** goes a layer
up: give it a goal and it prints an ordered **learning plan** — the facts decks
*and* traces worth authoring to reach that goal, dependency-ordered.

```sh
flash explore .                                      # a plan to understand the whole source
flash explore . --goal "how review scheduling works" # a narrow goal → a focused subset
```

Each item is tagged `[trace]` or `[deck]`, chosen by the **shape of the
knowledge**: a *path* you predict hop by hop becomes a trace; a *table of facts* —
a config's knobs, a store's on-disk format — becomes a facts deck. Each carries its
`% requires:` prerequisites (the list is a valid dependency order, foundations
first) and a `% source:` scope. The `--goal` scopes coverage: a broad goal spans
every subsystem; a narrow one collapses to its slice and traces it in more detail.
By default it's read-only — it prints the plan and you author the items yourself
(`flash trace --build` a trace, `flash deck` a facts deck).

## Materializing a workspace — `--into`

```sh
flash explore . --goal "how review scheduling works" --into ~/decks/scheduling/
```

writes a ready-made [workspace](08-workspaces.md): a `flash.toml` (carrying the
goal) and one **stub** per item — a `% trace:` deck per trace, a `% title:` facts
deck per deck — wired together with `% requires:` so they unlock in dependency
order, each `% source:` pointing back at the real source. (It refuses a non-empty
folder unless `--force`.) You then fill the stubs at your own pace.

## Filling it in one shot — `--into --build`

```sh
flash explore . --goal "…" --into ~/decks/scheduling/ --build
```

goes all the way: flash explores the source **once**, then reuses that single
session to fill every item — predict-verify checkpoints for the traces, fact cards
for the decks — so the workspace comes out review-ready in one command. Writing the
whole set from one understanding keeps the items **coherent** (each builds on its
prerequisites instead of repeating them). As a final step it
[freezes the cited excerpts](13-trace-decks.md) of every cited deck — traces and
fact decks with [`% at:` citations](06-cloze-direction-images.md#source-citations)
alike — into the workspace's `assets/`, so it's self-contained and its locators
never drift.

This is the tool's high-water mark: name what you want to understand, and flash
assembles a dependency-ordered curriculum of facts and traces — gated by
[mastery](12-the-ai-exam.md) — that you climb.

## The explore walk — `--walk`

Before you even know what to trace, `flash explore --walk <source>` builds a short
**tour of the source's shape** and walks it like a trace: you predict what kind of
program it is (from the manifest), its domain nouns (from the module list), how
it's driven (the entry point), its spine (the central file), and finally the first
paths worth tracing — each hop revealing the real lines. It's written to a file
(`-o`, default `explore.txt`), so `flash trace explore.txt` re-walks it.
