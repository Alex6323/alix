# 8 · Workspaces

As your decks multiply, you'll want to treat a *cluster* of them as a unit: all
your Spanish decks, or every deck about one codebase. A **workspace** is that
unit: a folder of decks reviewed together, sharing settings and a name, with its
own progress.

## Do you need one?

**For a single deck you study yourself, no.** A plain `.md` file in your decks
folder is a complete deck: it reviews, schedules, grades, and takes
[the exam](12-the-ai-exam.md) the same way a workspace member does. A
workspace adds settings and files *around* a group of decks; around one deck
there is nothing for it to do.

Reach for a workspace when one of these becomes true:

- **Several decks belong together and should share settings.** The
  `[defaults]` table is written once and every member inherits it, instead of
  the same `direction:` or `reveal:` line at the top of six files.
- **One deck should come before another.** `requires` only means something
  between members: it decides which decks are unlocked yet, and draws the
  [dependency tree](09-dependencies.md) in the picker.
- **The material has a source you want to keep.** Only a workspace member can
  **freeze** its evidence: initializing one copies each cited source excerpt
  and each local image into `assets/deck-<token>/`, and renders each
  [mermaid fence](06-cloze-direction-images.md) into a frozen diagram
  there, so the deck still shows
  them when the original file has moved on, and still shows them on a machine
  that never had the source at all. A deck outside a workspace keeps its images
  beside it and its citations live, so if the cited file moves or changes, the
  card shows a warning in place of the quoted lines.
- **You want to hand the whole cluster to someone else.** Sharing a workspace
  carries its members with their frozen assets and generated augmentation, and
  strips your progress. A loose deck has nothing frozen to carry, so what
  arrives is the file itself and whatever the recipient can still resolve.
- **There is a date you are working toward.** A `deadline` and the pacing ramp
  it drives are read only inside a real workspace; see [Personal
  pacing](#personal-pacing-alixlocaltoml).

What it costs: a folder with an `alix.toml` instead of a single file, and
members living in a `decks/` subfolder rather than loose.

**Changing your mind later is a manual move, not a command.** `alix deck
copy`/`move` transfer *between* workspaces: both the source and the
destination have to be workspaces already (see [Moving decks between
workspaces](#moving-decks-between-workspaces)), so neither promoting a loose
deck nor demoting a member is covered. Promoting one means creating the
workspace and moving the file into its `decks/` yourself. The ids inside the
file are untouched by that, but where your progress is stored depends on the
workspace's own settings, so run [`alix doctor`](17-command-reference.md) on
the result and check that the deck still reports the history you expect.

## Making a workspace

A workspace has an **`alix.toml`** at its root and its initialized `.md` decks
as direct children of `decks/`. The manifest is a scoped version of the global
config file. It sets a title and a `[defaults]` table of directives that every
member deck inherits:

```toml
# ~/decks/spanish/alix.toml
title = "Spanish"

[defaults]
direction = "both"
reveal = "line"
```

Besides `title`, `description`, `icon`, and a shared `source`, the manifest
may set a top-level `source_access`, which overrides the global
`[ask] source_access` for this workspace's decks in either direction (see
[the tutor](10-tutor.md)). The manifest travels with the folder when
shared, so review a received workspace's `alix.toml` before an AI call.

Starting from nothing instead? `alix workspace init <dir>` (`--title` to name
it) scaffolds an empty workspace: an `alix.toml`, an `alix.local.toml`, and an
empty `decks/` plus `assets/`. Both TOML files come fully commented, each key
explained inline, so they document themselves. The fixed layout is:

```text
spanish/
├── alix.toml
├── alix.local.toml
├── decks/
│   └── verbs.md
├── assets/
│   ├── icon.svg
│   └── deck-<token>/
│       └── sha256-<digest>.<ext>
├── progress/
└── augment/
```

Grow the workspace with
[`alix generate … --workspace <dir>`](11-generating-decks.md) or
`alix deck import … --workspace <dir>`, also available from the web UI's ☰
menu's **Add deck…** sheet. Dependencies (`requires:`) are still edited by
hand in the deck files.

Put hand-authored decks under `decks/`, then run `alix deck init <file>` once
for each one. Markdown without a valid opening-frontmatter `id: deck-<token>` is
ignored by discovery. Root-level Markdown is never a workspace member, so
README-style prose and notes can live beside `alix.toml` without becoming picker
entries or being stamped.

Initialization also makes the member portable. Cited excerpts are copied from
explicit source files and source directories alike (never a whole file or
repository), and local card images are copied into `assets/deck-<token>/`. Every managed filename is the
SHA-256 address of its exact bytes. The deck is not initialized successfully if
required evidence or an image cannot be copied.

## Updating from the live source

Frozen evidence is deliberately stable. It does not follow later source edits
in the background. Reconcile every frozen source-backed member explicitly:

```sh
alix workspace update ~/decks/spanish
```

The command gives its AI backend read-only access to each recorded local
`source`, then writes one exact proposal into a dot-prefixed sibling workspace.
The original workspace remains untouched. Inspect the proposed decks and
evidence there, then publish those exact bytes without another model call:

```sh
alix workspace update ~/decks/spanish --apply
```

Use `--discard` instead to remove the proposal. Apply refuses if an original
deck changed after staging.

A card ID belongs to one learning proposition. An unchanged question and
answer may keep its ID while its note or source locator improves. If the
question, answer, cloze, or learning image changes, the old card and ID retire
together and the replacement receives a fresh ID during staging. Obsolete
cards are removed rather than rewritten in place under their old learning
history.

The first update implementation accepts local file and directory sources. A
remote URL source remains review and tutor context, but cannot yet be captured
as a new portable snapshot.

## Moving decks between workspaces

A workspace deck owns more than its Markdown file. Transfer it with Alix so its
frozen evidence and augmentation follow the stable deck ID:

```sh
alix deck copy ~/decks/spanish/decks/verbs.md ~/decks/exam
alix deck move ~/decks/spanish/decks/verbs.md ~/decks/exam
```

Both commands preserve the filename, deck ID, and card IDs. Copy installs the
same public bundle that wormhole sharing sends: the deck,
`assets/deck-<token>/`, and `augment/deck-<token>.json`. It never copies progress.
Move requires confirmation, installs that public bundle first, carries
`progress/deck-<token>.json` when the workspaces use different user roots, then
removes the source. Workspaces configured to use one shared user root already
address the same progress document by deck ID, so no progress file moves.

The destination must be another Alix workspace. Transfer refuses overwrites,
stable-ID collisions, missing required decks, and moves that would break a
source deck's dependents. An inherited or relative `source` is written
explicitly into the transferred deck so the destination cannot reinterpret its
live provenance through unrelated workspace defaults.

Now open the cluster and drill its members one at a time:

```sh
alix ~/decks/spanish/
```

## Shared directives

The `[defaults]` keys are the deck-directive names `reveal`, `input`,
`order`, `direction`, and `sampling` from
[the deck format](03-the-deck-format.md),
plus `strictness`: the learner-side [exam](12-the-ai-exam.md) rigor, which
a deck itself cannot declare. They fill in only what a deck *doesn't* set
for itself, so the precedence is one level deeper than before:

> card `<!-- -->` > deck frontmatter > **workspace `[defaults]`** > built-in default

Set `direction = "both"` once for the whole folder, and a single irregular deck
can still override it with its own `direction: forward` in its frontmatter. It's
the same directive system from chapter 3, just sourced from one more place.

## Personal pacing: `alix.local.toml`

The `alix.toml` is shared: it travels with the workspace when you hand it to
someone. Your **personal** review pacing doesn't belong there. Drop an
`alix.local.toml` beside it to override the global `[review]` config (FSRS
`retention`, `retire_after`, `introduction_cooldown`, and the pacing keys
`max_session` / `new_cards_percent`) for this workspace's decks only:

```toml
# ~/decks/spanish/alix.local.toml
[review]
retention = 0.95         # see these cards more often
retire_after = "never"   # never let them retire
max_session = 20         # bigger sittings for this deck
new_cards_percent = 40   # lean harder on introducing new cards
deadline = "2026-09-01"  # a personal "ready by" date, the day itself inclusive
deadline_ramp = "14d"    # how early the pre-deadline retention ramp starts
```

It uses the same `[review]` keys as the [config file](16-configuration.md), and
it's kept separate from `alix.toml` on purpose, so it stays yours and never
travels when you share the workspace. A missing or malformed one is simply
ignored.

`deadline` and `deadline_ramp` only take effect **inside a real workspace**
(a directory with an `alix.toml`). Set them on a plain decks folder, or on a
loose deck's `alix.local.toml`, and they parse but do nothing: no scheduling
ramp, no picker readout, no doctor warning. See
[Configuration](16-configuration.md) for the full reference and
[Scheduling](05-scheduling.md) for what the ramp does to review.

The session depth (Recognize/Recall/Reconstruct) isn't a workspace setting.
It's picked per session, the same as for a loose deck (see
[Reveal & session depths](04-review-modes.md)).

## Its own files

A workspace keeps shareable material at the workspace root and private
learning state in its selected user-files root:

```text
augment/deck-<token>.json    # shareable generated choices, notes, and topologies
assets/deck-<token>/         # shareable frozen excerpts and local images
progress/deck-<token>.json   # private schedules, history, exam state
```

Renaming a deck file leaves these paths unchanged because the name comes from
its deck id (`deck-<token>`), not its display name. By default the private files are colocated
with the workspace, so folder synchronization carries progress too. A
`store = "..."` line in `alix.toml` moves only private files such as
`progress/` and `recent.json`; augmentation and assets stay beside the decks
they describe.

That makes a workspace a **self-contained, portable unit** for moving, backup,
and folder synchronization: authored decks in `decks/`, frozen excerpts and
images in deck-owned `assets/deck-<token>/` directories, workspace icons directly
in `assets/`, and shareable augmentation all live under one boundary. Sharing
strips progress and local configuration while carrying the matching
augmentation and assets. Decks outside any workspace keep shareable material
beside the deck and private files in the selected user-files root. The CLI
commands (`alix stats`/`list`/`reset`) take a deck file, a plain folder, **or a
workspace**: a folder or workspace expands to its member decks, each resolved
against the same user-files root the launcher would use (`--store <path>` still
overrides private files only).

## In the picker

Folders show up in the picker in two flavors: a folder with `alix.toml` and
initialized `decks/*.md` members appears under **Workspaces**; one without a
manifest is a plain **Folder** whose initialized decks are direct `*.md`
children. Opening either drills in to its decks, drawn as a **dependency tree**:
each deck nests under the prerequisite that gates it, foundations at the roots
(the [next chapter](09-dependencies.md)). A trace member carries a `trace`
badge (facts decks are unbadged), and the drill-in is a single-launch list:
`Enter` on a facts deck
reviews it, `Enter` on a trace **walks** it. Typing a filter flattens the tree
to a plain search.

In the **web** picker, a workspace can show a small **emblem** in place of the
chevron, so a long list of similar-named workspaces is quicker to scan. Drop an
image in the workspace's `assets/` and point `icon = "assets/<file>"` at it in the
`alix.toml` (or just name it `assets/icon.{svg,png,jpg}` and skip the key); an SVG
is tinted to the active theme, a raster shows as-is. When you build a workspace
with `alix generate <source> --workspace <dir>`, the model draws an abstract SVG emblem from
the topic automatically, unless you pass `--icon <file>`.

`alix <dir>` serves a workspace directly: the picker opens drilled into that
view, scoped to the folder and its own store, routing each
member to the right experience (a facts deck to a review, a trace to a walk) and
returning you to the picker when you finish one. (A session is one deck file, so
a whole workspace is never reviewed at once; open it and pick a member.)

A folder without a manifest serves the same way with `alix <folder>`; it
just applies no shared directives.

## Sharing a workspace

A workspace is a self-contained folder, so sharing one is sending the folder
with its `decks/` structure intact.
`alix share <dir>` does that over magic-wormhole with the personal files
(`progress/`, backups, recent list, `alix.local.toml`) left home; the
other side runs `alix receive <code>` and gets it beside their own decks, ready
to serve with `alix <dir>`. Precomputed augmentation documents matching the
shared decks travel: the AI content comes along, unrelated augmentation and
progress do not. A single-deck share carries the `.md` member, its complete
`assets/deck-<token>/` directory, and its matching augmentation. Also available
from the web UI's ☰ menu
(**Share…** / **Add deck…** → Receive), with a `.zip` download/upload fallback
when neither side has `wormhole` installed.

## Titles

A single deck's display name is its `#` heading (the top-level Markdown title); a
workspace's name comes from a `title` in its `alix.toml`. Either replaces the file
name in the picker, the session header, `alix list`, and `alix stats`. It's
display-only: you still refer to decks by file path on the command line, and a
title never affects a card's identity.
