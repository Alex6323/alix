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
  **freeze** its evidence: a card can cite lines of a file through an
  `<!-- at: -->` locator that names a `cite:` root (a folder or URL the deck
  or workspace declares; see [Source
  citations](06-cloze-direction-images.md#source-citations)), and
  initializing a member copies each cited excerpt
  and each local image into `assets/deck-<token>/`, and renders each
  [mermaid fence](06-cloze-direction-images.md) into a frozen diagram
  there, so the deck still shows
  them when the original file has moved on, and still shows them on a machine
  that never had the source at all. A deck outside a workspace keeps its images
  beside it and its citations live, so if the cited file moves or changes, the
  card shows a warning in place of the quoted lines.
- **You want to hand the whole cluster to someone else.** Sharing a workspace
  carries its members with their frozen assets and generated augmentation
  (AI-made extras alix keeps per card, such as multiple-choice options and
  notes; see [Augmenting a deck from the
  picker](15-the-web-app.md#augmenting-a-deck-from-the-picker)), and strips
  your progress. A loose deck has nothing frozen to carry, so what
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
file are untouched by that, but progress now resolves at the new workspace
root, so run [`alix doctor`](17-command-reference.md) on the result and check
that the deck still reports the history you expect.

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

Besides `title`, `description`, `icon`, a shared `link` (one URL or a
list, read by the tutor after each member's own `link:` entries), and a `[cite]`
table of [citation roots](06-cloze-direction-images.md#source-citations)
every member sees, the manifest may set a top-level `source_access`, which lets the tutor read
the live files under a card's local `cite:` root, and overrides the global
`[ask] source_access` for this workspace's decks in either direction (see
[the tutor](10-tutor.md#grounding-a-frozen-card-cite)). The manifest travels with the folder when
shared, so review a received workspace's `alix.toml` before an AI call.
An `alix.toml` that does not parse (bad TOML, or a key alix does not know) is
an error, never a fallback to defaults: the picker lists the workspace as an
error row with the parse message and no members, and `alix doctor` names the
problem.

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
│       └── asset-<52 base32>.<ext>
├── sources/
│   └── deck-<token>/
│       ├── manifest.toml
│       ├── source-<digest>.<ext>
│       └── manifest.local.toml
├── .alix/
│   ├── progress/deck-<token>.json
│   ├── exam/deck-<token>.json
│   ├── exam/deck-<token>.prepared.json
│   └── recent.json
└── augment/
```

`sources/` holds each deck's **exam sources**: frozen copies of local files
the [AI exam](12-the-ai-exam.md#sources) examines a deck against. They are
not citations: `alix deck source add` creates them, and
`alix deck generate` or `alix workspace generate` from a local file or
folder creates them for the decks it writes. `alix workspace init` creates
no `sources/` folder.

Grow the workspace with
[`alix deck generate … --into <dir>`](11-generating-decks.md) or
`alix deck import … --workspace <dir>`, also available from the web UI's ☰
menu's **Add deck…** sheet. Dependencies (`requires:`) are still edited by
hand in the deck files.

Put hand-authored decks under `decks/`, then run `alix deck init <file>` once
for each one. Markdown without a valid opening-frontmatter `id: deck-<token>` is
ignored by discovery. Root-level Markdown is never a workspace member, so
README-style prose and notes can live beside `alix.toml` without becoming picker
entries or being stamped.

Initialization also makes the member portable. Cited excerpts are copied
from the live files under the deck's `cite:` roots: only what each
citation names, its lines, or the whole file for a citation that names no
lines, never a whole root. Local card images are copied into
`assets/deck-<token>/`. Every managed filename is the
SHA-256 address of its exact bytes. The deck is not initialized successfully if
required evidence or an image cannot be copied. After a successful freeze,
initialization deletes every file in `assets/deck-<token>/` that the deck no
longer references, such as the excerpt of a citation whose range changed.

## Updating from the live source

Frozen evidence is deliberately stable. It does not follow later source edits
in the background. Reconcile every frozen source-backed member explicitly:

```sh
alix workspace update ~/decks/spanish
```

The command gives its AI backend read-only access to each member's first
local `cite:` root, then writes one exact proposal into a staging copy of
the workspace beside it, named after the workspace with a leading dot
(`~/decks/.spanish.updating` here), and prints that path. The original
workspace remains untouched. Inspect the proposed decks and evidence there,
then publish those exact bytes without another model call:

```sh
alix workspace update ~/decks/spanish --apply
```

Use `--discard` instead to remove the proposal. Apply refuses if an original
deck changed after staging.

Every member is checked before the first model call, so these cost nothing:
a member without any `cite:` root is skipped, a member whose only roots are
URLs stops the command, a member whose cards cite nothing is left as is and
named in the report, and a member whose citations are not yet frozen stops
the command. After that, a
member whose proposal comes back unusable is reported as that member and left
exactly as the proposal found it, while the other members still stage. The
command exits non-zero and names each one, the reasons are recorded in the
proposal so `--apply` reprints them, and applying touches only the members that
did update. If every member fails there is no proposal to apply, and the
staging directory is left for inspection.

A [card ID](03-the-deck-format.md#why-editing-a-deck-is-safe) belongs to
one thing to learn. An unchanged question and answer may keep its ID while
its note or source locator improves. If the question, answer, cloze, an
image or the authored choices change, the proposal must drop the old card
with its ID, and the replacement receives a fresh ID during staging; a
proposal that keeps an ID on changed content is refused for that member.
Obsolete cards are removed rather than rewritten in place under their old
learning history.

A [topic ID](03-the-deck-format.md#topic-ids) (the permanent id of a `#`
topic, written as a line in the deck file) stays with its topic. A proposal may remove an obsolete topic
whole, its ID with it, but a proposal that copies a topic ID onto a second
topic or invents one is refused. A new `#` topic in the proposal receives a
fresh ID during staging. A deck that already repeats a topic ID is refused
before the update asks the backend, with the duplicate named; fix it first.

Update reads each member's first local `cite:` root (the workspace `[cite]`
table first, then the deck's own `cite:`). A remote root remains review and
tutor context, but cannot yet be captured as a new portable snapshot.

## Moving decks between workspaces

A workspace deck owns more than its Markdown file. Transfer it with Alix so its
frozen evidence and augmentation follow the stable deck ID:

```sh
alix deck copy ~/decks/spanish/decks/verbs.md ~/decks/exam
alix deck move ~/decks/spanish/decks/verbs.md ~/decks/exam
```

Both commands preserve the filename, deck ID, and card IDs. Copy installs the
same public bundle that sharing sends with `--with-sources`: the deck,
`assets/deck-<token>/`, `sources/deck-<token>/` (verified on landing), and
`augment/deck-<token>.json`. The folder's `manifest.local.toml` comes along
too: the roots it names are this machine's folders, which a local copy or
move does not change. It never copies progress. Move requires confirmation, installs that public
bundle first, carries `.alix/progress/deck-<token>.json` to the destination
workspace, then removes the source.

The destination must be another Alix workspace. Transfer refuses overwrites,
stable-ID collisions, a topic id a destination member already carries,
missing required decks, and moves that would break a source deck's
dependents.

Transfer never rewrites a deck's locators or `cite:`, and changes
neither workspace's `alix.toml`. It is allowed only when every key the deck's locators
use resolves in the destination, through the deck's own `cite:` or the
destination's `[cite]` table, to the same root as in the source. Local roots
are compared as absolute paths with symlinks resolved; URL roots must match
exactly. A relative root resolves against its workspace folder, so a key
written as `code = "crates"` in both manifests names two different folders,
and the transfer refuses. The refusal names each key and both roots. Keys that
no locator uses are not checked.

Now open the cluster and drill its members one at a time:

```sh
alix ~/decks/spanish/
```

## Shared directives

The `[defaults]` keys are the deck-directive names `reveal`, `input`,
`review`, `direction`, and `sampling` from
[the deck format](03-the-deck-format.md),
plus `strictness`: the learner-side [exam](12-the-ai-exam.md) rigor, which
a deck itself cannot declare. They fill in only what a deck *doesn't* set
for itself, so a member deck has one more level of precedence than a loose
deck:

> card `<!-- -->` > deck frontmatter > **workspace `[defaults]`** > built-in default

Set `direction = "both"` once for the whole folder, and a single irregular deck
can still override it with its own `direction: forward` in its frontmatter. It's
the same directive system from chapter 3, just sourced from one more place.

## Personal pacing: `alix.local.toml`

The `alix.toml` is shared: it travels with the workspace when you hand it to
someone. Your **personal** review pacing doesn't belong there. Drop an
`alix.local.toml` beside it to override the global `[review]` config for
this workspace's decks only: `retention` (the recall probability
[FSRS](05-scheduling.md#fsrs), alix's scheduler, aims for), `retire_after`
(the interval at which a card [retires](05-scheduling.md#retiring-cards)),
`introduction_cooldown` (the gap between seeing a new card and its first
quiz), and the pacing keys `max_session` / `new_cards_percent`:

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
ignored. Its `.local.` name follows the same private naming rule as a deck's
`.local.md` personal file.

`deadline` and `deadline_ramp` only take effect **inside a real workspace**
(a directory with an `alix.toml`). Set them on a plain decks folder, or on a
loose deck's `alix.local.toml`, and they parse but do nothing: no scheduling
ramp, no picker readout, no doctor warning. See
[Configuration](16-configuration.md) for the full reference and
[Scheduling](05-scheduling.md) for what the ramp does to review.

The session depth (Recognize/Recall/Reconstruct) isn't a workspace setting.
It's picked per session, the same as for a loose deck (see
[Reveal & session depths](04-review-modes.md)).

## Warming the whole workspace

`alix workspace augment <dir> --target <...>` precomputes AI augmentations for
every member deck at once:

```sh
alix workspace augment ~/decks/rust --target choices
alix workspace augment ~/decks/rust --target icon
```

The card targets (`choices`, `notes`, `questions`, `keypoints`, `format`) run
as **one batched call over every member's cards**, not one call per deck, and
each card's result is filed back under the deck that owns it in `augment/`.
`icon` draws the workspace emblem and is the target only a workspace has; it
redraws unconditionally, replacing the old one.

There is no `--target order` here. A review order is built per deck, so it
stays on [`alix deck augment`](17-command-reference.md).

## Its own files

A workspace keeps shareable material and private learning state at the
workspace root:

```text
augment/deck-<token>.json    # shareable generated choices, notes, and review orders
assets/deck-<token>/         # shareable frozen excerpts and local images
sources/deck-<token>/        # frozen exam sources: manifest.toml and the copies
sources/deck-<token>/manifest.local.toml # private: where each key's folder is on this machine
.alix/progress/deck-<token>.json # private schedules, history, exam state
.alix/exam/deck-<token>.json # private exam idea lists and what was asked
.alix/exam/deck-<token>.prepared.json # a written exam waiting to be taken
.alix/recent.json            # private recent-deck history
```

Renaming a deck file leaves these paths unchanged because the name comes from
its deck id (`deck-<token>`), not its display name. Private files are colocated
with the workspace, so folder synchronization carries progress too.

That makes a workspace a **self-contained, portable unit** for moving, backup,
and folder synchronization: authored decks in `decks/`, frozen excerpts and
images in deck-owned `assets/deck-<token>/` directories, workspace icons directly
in `assets/`, and shareable augmentation all live under one boundary. Sharing
strips progress and local configuration while carrying the matching
augmentation and assets. Decks outside any workspace keep shareable material
beside the deck and private files in that deck's containing folder. The CLI
commands (`alix stats`/`list`/`reset`) take a deck file, a plain folder, **or a
workspace**: a folder or workspace expands to its member decks, and every
command resolves the same colocated `.alix/` root the launcher uses.

## In the picker

Folders show up in the picker in two flavors: a folder with `alix.toml` and
initialized `decks/*.md` members appears under **Workspaces**; one without a
manifest is a plain **Folder** whose initialized decks are direct `*.md`
children. Opening either drills in to its decks, drawn as a **dependency tree**:
each deck nests under the prerequisite that gates it, foundations at the roots
(the [next chapter](09-dependencies.md)). A [trace
deck](13-trace-decks.md), which walks a path through a source checkpoint by
checkpoint, carries a `trace` badge; an ordinary deck of cards, a facts
deck, is unbadged. The drill-in is a single-launch list:
`Enter` on a facts deck
reviews it, `Enter` on a trace **traces** it. Typing a filter flattens the tree
to a plain search.

In the **web** picker, a workspace can show a small **emblem** in place of the
chevron, so a long list of similar-named workspaces is quicker to scan. Drop an
image in the workspace's `assets/` and point `icon = "assets/<file>"` at it in the
`alix.toml` (or just name it `assets/icon.{svg,png,jpg}` and skip the key); an SVG
is tinted to the active theme, a raster shows as-is. When you build a workspace
with `alix workspace generate <dir>`, the model draws an abstract SVG emblem from
the topic automatically, unless you pass `--icon <file>`.

`alix <dir>` serves a workspace directly: the picker opens drilled into that
view, scoped to the folder and its own store, routing each
member to the right experience (a facts deck to a review, a trace deck to a trace) and
returning you to the picker when you finish one. (A session is one deck file, so
a whole workspace is never reviewed at once; open it and pick a member.)

A folder without a manifest serves the same way with `alix <folder>`; it
just applies no shared directives.

## Sharing a workspace

Sharing a workspace sends its alix files, not the whole folder: the
`alix.toml` manifest, `decks/`, the `assets/` its decks own, and their
augmentation. Everything else at the workspace root, such as the source trees
your `cite:` roots point at, notes, or a README, stays home: the frozen
excerpts in `assets/` already carry what the cards show. The frozen exam
sources in `sources/deck-<token>/` are copies of your own files, so they stay
home too unless you pass `--with-sources`; the receiver cannot take the exam
without them, and the command says how many source files it left out. The
machine-local `sources/deck-<token>/manifest.local.toml` never travels either
way. The [mobile app](18-the-mobile-app.md#syncing-with-the-desktop)'s
sync with a [paired](19-pairing.md) desktop sends the same set with the
sources always included: the phone reads its copy to tell which topics
have sources and whether their results are current, while the desktop
writes and grades the phone's exam from its own copy.
`alix share <dir>` does that over magic-wormhole with the personal files
(`.alix/`, `*.local.*`, backups, and conflict copies) left home; the
other side runs `alix receive <code>` and gets it beside their own decks, ready
to serve with `alix <dir>`. Precomputed augmentation documents matching the
shared decks travel: the AI content comes along, unrelated augmentation and
progress do not. A single-deck share carries the `.md` member, its complete
`assets/deck-<token>/` directory, and its matching augmentation, plus its
`sources/deck-<token>/` copies (without `manifest.local.toml`) with
`--with-sources`. Also available
from the web UI's ☰ menu
(**Share…** / **Add deck…** → Receive), with a `.zip` download/upload fallback
when neither side has `wormhole` installed.

## Titles

A single deck's display name comes from its frontmatter `title:` (failing that,
its condensed `trace:`, the path description a trace deck declares, then
its filename stem); a workspace's name comes from a
`title` in its `alix.toml`. Either replaces the file
name in the picker, the session header, `alix list`, and `alix stats`. It's
display-only: you still refer to decks by file path on the command line, and a
title never affects a card's identity.
