# 17 · Command reference

A quick index of the `alix` commands. Each links to the chapter that covers it in
depth, where there is one. Run any command with `--help` for its full flags.

## Reviewing

- `alix`: serve the web app: the deck [picker](02-getting-started.md) over
  your decks directory (`~/decks`), printing its URL.
- `alix <dir>`: serve that folder as a **self-contained scoped root**: its own
  catalog and shareable `augment/` and `assets/`, with private per-deck
  progress plus recent history under its colocated `.alix/` directory. A
  [workspace](08-workspaces.md) dir opens the picker drilled into it.

Every review starts from the picker. There's no direct deck launch. Browsing a
deck read-only, sitting the AI exam, and tracing a [trace deck](13-trace-decks.md)
are all reached from the web picker rather than as their own commands (see
[the web app](15-the-web-app.md)).

The single-instance launcher's flags: `--lan` / `--port` / `--token`
([the web app](15-the-web-app.md)), `--session N` (cards per sitting, overriding
the `[review] max_session` config; unrelated to the AI backend's own
`--session-id`), `--config <path>`, and `--log http,select` (enable verbose
file records and mirror the named targets to stderr). The session depth is
picked in the picker's split Depth… menu, an order or region in its focus
drawer ([scheduling](05-scheduling.md)), and the card order is the deck's
`review:` directive.
How each card is checked comes from its `reveal:` combined with the
session's depth ([reveal & session depths](04-review-modes.md)), not a flag.

## Launch profiles

Launch profiles make it easy to run one named alix instance per person in a
household, with its own decks folder, port, and adult or kids frontend. Each
profile is a normal config file under the platform config directory's
`profiles/` folder.

A profile name cannot be empty or contain `/`, `\`, `.`, or `:`.

```sh
alix profile add timmy --decks ~/decks-timmy --port 7002 --kids
alix profile list
alix profile timmy
alix profile default timmy
alix --launch-all
alix profile remove timmy
```

`alix profile <name>` launches that profile on the LAN and reuses the stable
token generated when it was added, so a phone can bookmark the printed URL.
`alix profile default` shows the current default, names one when given a
profile, and clears it with `--clear`; bare `alix` launches that default.
`alix --launch-all` starts every profile in the foreground on its configured
port. Ctrl-C or closing the terminal stops them together.

Each profile's decks folder must be separate: it cannot equal, contain, or sit
inside another profile's folder. `alix profile add` refuses an overlap, and
bare `alix doctor` reports every existing pair without rewriting either
profile. Paths are resolved before comparison, so symlink aliases count as the
same folder.

Each profile keeps its own [tutorial](15-the-web-app.md#the-tutorial)
progress, and `alix profile remove` also deletes that profile's tutorial
folder.

## Progress

`alix stats`, `alix list`, and `alix reset` each take a **deck file, a plain
folder, or a [workspace](08-workspaces.md)**: a folder or workspace expands to
its member decks, and each deck resolves exactly as the launcher does: a
workspace member uses the workspace root and a loose deck uses its containing
folder. Inside that boundary, progress is loaded from
`.alix/progress/deck-<token>.json`; folder-wide commands aggregate the
relevant documents in memory without creating an authoritative combined file.

- `alix stats <target>`: progress overview, completion state, and a
  per-depth due count.
- `alix list <target>`: every card with three per-depth cells, shallow to
  deep (Recognize | Recall | Reconstruct), each carrying that depth's state
  and due time; a retired card reads `resting` instead.
- `alix reset <target>`: clear progress (`--card`, `--all`; `-y` to
  skip the prompt). On a workspace it also clears each deck's exam state and
  personal-card schedules in the workspace's own store, after one confirmation.
- `alix reset <target> --exam` (or `--exam --all`): clear only exam state, per
  deck: each card's exam result, the deck's passed and mastered instants, and
  its last full count. Drilling progress stays. Before asking, it names the
  decks whose exams this will lock: dependents in the same folder or workspace
  that have never sat their own exam. Not combinable with `--card`.
  A target progress document that does not parse is removed with the rest,
  and the prompt says so first; documents outside the target are never
  opened, and an I/O failure stops the reset before anything is erased.
- `alix reset --orphans [target]` clears only **orphaned** progress: store
  keys that match no card or deck in the scanned decks (a stripped
  `<!-- id: … -->` comment, a hand-deleted deck, a double-mint).
  Orphans are never removed automatically (they are evidence), so this is the
  explicit opt-in. It scopes to a named folder/workspace
  store, else the configured decks folder, and reads every progress document
  under its `.alix/` directory (the same documents `alix doctor` reports on).
  A single deck file scopes
  to that deck's own document instead. A folder whose last deck was deleted is
  still a valid target. Every deck-like file in a folder is scanned for live
  ids, including one still awaiting its `id:` line, and any of them failing to
  parse aborts the sweep, since its cards cannot be told apart from orphans.
  Run `alix doctor` first to see what it would clear.

Deck [dependencies](09-dependencies.md) (`requires:`) are edited by hand in
the deck file. There's no separate command for it.

## The AI features

AI authoring lives under the noun it produces, so the command names what you
get: **`alix deck generate`** always writes one deck, **`alix workspace
generate`** always builds a workspace.

Both take the same steering options. `--link <URL>` (repeatable, http or
https only) records a tutor reference: the deck's `link:` list for `deck
generate`, the workspace `alix.toml` `link` key for `workspace generate`. `--goal <TEXT>` scopes what the new
deck or workspace teaches. `--language <LANGUAGE>` controls learner-facing
output, and `--audience <TEXT>` controls assumed knowledge and difficulty.
`--card-style mixed|plain|cloze|authored-choices` selects the facts-card shape;
workspace trace items retain their checkpoint shape. `--force` overwrites what
is already there. Each subcommand takes `--into <dir>`, with the meaning its
own result needs: for `deck` an existing workspace to write into, for
`workspace` the folder to build.

- **`alix deck generate <source>`** → one deck from a web page URL, a local
  file, or a directory taken whole, with no planning pass
  ([facts decks](11-generating-decks.md); `-o/--output`, `--cards`,
  `--review`, `--print`; `--into <workspace>` writes it into that workspace's
  `decks/` instead of the decks dir, and the workspace must already exist).
  - with **`--trace`** → that deck is a [trace](13-trace-decks.md) authored
    over the source (`-o/--output` defaults to `explore.md`).
    `--trace --plan` prints a ranked menu of suggested traces instead.
  - given an existing **`trace:` stub deck** → builds its checkpoints in place.
- **`alix workspace generate <dir>`** → the directory is explored for an
  [ordered learning plan](14-explore.md), which is confirmed and then built as
  a workspace, whatever its size (`--title`/`--icon` name and brand it;
  `--into <dir>` is the folder to build, created if absent, defaulting to one
  named after the source under the decks dir). `--plan` prints the plan and
  stops. A source that is not a directory is
  refused, naming `deck generate` instead.

The rest of the AI-and-deck surface:

- `alix deck init <file>`: explicitly initialize a hand-authored Markdown deck
  with stable deck and card IDs. Uninitialized `.md` files are ignored by
  discovery and never stamped merely because they contain `##` headings, and a
  `<deck>.local.md` is refused outright: it belongs to the deck beside it
  and never gets an `id:` of its own.
- `alix deck augment <deck> --target <...>`: precompute AI augmentations
  (choices, notes, questions, keypoints, format, order). The augmentation
  document stays beside the deck; private progress is read from the deck's
  colocated `.alix/` directory when `format` considers personal cards.
- `alix deck source <subcommand>`: manage a workspace deck's frozen exam
  material, kept in `sources/<deck-id>/` at the workspace root
  ([sources](12-the-ai-exam.md#sources)). `<deck>` is the deck file or a
  unique prefix of its `deck-` id, searched in the current workspace and the
  configured decks folder (an id held by two decks is refused, naming both);
  `<source>` is a unique prefix of a
  `source-` id; a topic is a unique prefix of a `topic-` id or the deck id,
  written `[<ranges>|]<topic>` where it binds.
  - `add <deck> <file>... [--root <key>=<dir>] [--key <key>] --topic
    <topic>...`: freeze local, non-empty UTF-8 text files and bind them.
    `--root` saves `<dir>` as the folder of `<key>` (refused when `<key>` is
    already saved to another folder); `--key` picks the root when several
    hold a file. `--topic` may be left out only on a deck without `#`
    topics, which binds the deck id. Warns, and never refuses, when a bound
    topic's exam prompt exceeds a backend's limit
    ([how big a topic may be](12-the-ai-exam.md#how-big-a-topic-may-be)).
  - `remove <deck> <source>...`: drop sources and delete their copies.
  - `update <deck> <source> <file> [--key <key>] [--keep-results]`: refreeze
    a source from a file, keeping its bindings and their ranges; prints each
    kept range for review. `--keep-results` declares that the new bytes keep
    the meaning: a result of a topic the source grounds moves to the new
    material unless it already was stale, and the standing stays solid if it
    was solid before ([sources](12-the-ai-exam.md#sources)). Warns like
    `add` on a topic over a backend's limit.
  - `bind <deck> <source> <topic>...`: add a topic's entry, or replace it
    when the topic is already bound. Warns like `add` on a topic over a
    backend's limit.
  - `unbind <deck> <source> <topic>...`: drop topics, named without ranges;
    the last topic of a source cannot be unbound (remove the source
    instead).
  - `root set <deck> <key> <dir>` and `root unset <deck> <key>`: save or
    forget where a key's live files are on this machine, printing
    `old -> new`.
  - `list <deck>`: each topic with its source count and bytes, then one
    line per backend with the topic's exam prompt measured against that
    backend's limit (`copilot  133649 of 128000 estimated tokens  exceeds
    the limit`; [how big a topic may be](12-the-ai-exam.md#how-big-a-topic-may-be)),
    and each
    source with its id, bytes, display path, drift (`same`, `changed`,
    `missing`, `unreadable`, `linked` for a live path that is a symbolic
    link or leaves its root, which is never followed, or `unchecked` for a
    key without a usable root, followed by the reason: no root on this
    machine, or a saved root that is not an absolute existing folder), and
    bindings.
- `alix deck copy <deck> <workspace>`: copy one initialized workspace member,
  its owned frozen assets, its frozen exam sources with their
  `manifest.local.toml` (the roots it names are this machine's), and its
  augmentation into another workspace. Stable
  deck and card IDs are preserved; progress is not copied.
- `alix deck move <deck> <workspace> [--yes]`: move the same public bundle,
  carry progress when the workspaces use different user roots, then remove the
  source. Refuses missing prerequisites and source dependents. Copy and move
  both refuse when a `cite:` key the deck's locators use would resolve to a
  different root in the destination, or when a destination member already carries one of the deck's topic ids.
  Neither rewrites the deck.
- `alix deck import <file.tsv>`: import an Anki TSV export (no model CLI
  needed; `--workspace <dir>` imports into a workspace).
- `alix deck remove <deck> [--yes]`: remove a deck and everything that is
  its alone: the file, its review history, its frozen assets, its frozen exam
  sources, its augmentations, and any `.bak` backups. Total by design: nothing is backed
  up and it cannot be undone, which the confirmation states along with the
  stakes (cards with progress, reviewed-since date, the exact file list).
  A deck that others `require:` warns and names them; they unlock rather
  than break.
- `alix deck restore <deck>`: swap every available `.bak` side for that deck
  (deck file, `.alix/progress` history, augmentations). This undoes a forced
  import, a trace or workspace regeneration, and also the progress backup an
  accepted paired-phone push leaves when the deck text itself has no backup.
  The command reports which of the three sides swapped. Nothing is destroyed:
  the swapped-away state becomes the new backup, so running it again swaps
  back. There is nothing to restore after `deck remove`, which deletes the
  backups too.
- `alix workspace init <dir>`: scaffold an empty
  [workspace](08-workspaces.md): an `alix.toml` (`--title` names it), an
  `alix.local.toml` (personal pacing: deadline, retention), and an empty
  `decks/` plus `assets/`. Grow it with `alix deck generate … --into <dir>` or
  `alix deck import … --workspace <dir>`.
- `alix workspace update <dir>`: reconcile frozen source-backed members with
  their recorded local sources. The first run stages an exact sibling
  workspace for review; `--apply` publishes it without another model call and
  `--discard` removes it. Changed or obsolete learning propositions retire
  their old card IDs; replacements receive fresh IDs.
- `alix workspace augment <dir> --target <...>`: precompute AI augmentations
  for every member deck at once (choices, notes, questions, keypoints, format),
  plus `icon`, the workspace's own emblem. The card targets run as **one
  batched call over every member's cards**, not one call per deck, and each
  card's result is filed back under its own deck in the workspace's
  augmentation folder. `--target order` is deliberately absent: a review order
  is built per deck, so it lives on `alix deck augment`. `icon` redraws
  unconditionally, replacing the old emblem.
- `alix workspace deadline <dir> [<date>|clear]`: show, set, or clear a
  workspace's personal "ready by" date (`--config <path>`); no argument prints
  the current one. Workspace-only, see [Workspaces](08-workspaces.md).
- Tutor: the Ask button (or `a`) in a session, **Make this a note** to save a
  note ([the tutor](10-tutor.md)).

The agentic `generate` runs measure the source size before running and prompt
for confirmation when it's large. Pass `--yes` to skip the prompts in
non-interactive scripts. The [AI exam](12-the-ai-exam.md) runs unattended in
the browser instead, so it can't prompt: it refuses to start on a topic whose
prompt exceeds the backend's limit, naming the topic, the backend, the size
and the limit ([how big a topic may be](12-the-ai-exam.md#how-big-a-topic-may-be)).

## Sharing

- `alix share <path>`: send a deck file, a plain folder, or a workspace to
  someone over [magic-wormhole](https://magic-wormhole.readthedocs.io) (the
  `wormhole` binary must be installed, `alix doctor` checks). A folder is
  staged first so your personal state stays home: `.alix/`, `*.local.*`,
  temporary files, and conflict or backup files never travel. From a
  workspace only its alix files go: the manifest, `decks/`, owned `assets/`,
  and augmentation; source trees and other files at its root stay home.
  Frozen exam sources (`sources/deck-<token>/`) stay home too unless you pass
  `--with-sources`, since they are copies of your own files; the receiver
  cannot take the exam without them, and the command says how many source
  files it left out. The machine-local `sources/deck-<token>/manifest.local.toml`
  never travels either way.
  Matching `augment/deck-<token>.json` documents do travel, including when sharing
  one deck. A single frozen deck also carries its complete
  `assets/deck-<token>/` directory. A symbolic link inside what you share is
  refused by name rather than followed, since the copy carries files and the
  folder you picked is the boundary of what leaves the machine: replace the link
  with what it points to, or remove it. Tell the receiver the code wormhole
  prints. No wormhole around?
  `--zip [--output <path>]` writes the same staged copy as a `.zip` to mail or
  hand over instead.
- `alix receive <code-or-zip>`: fetch what someone shared, by wormhole code
  or by a `.zip` path (the `--zip` fallback's output, same landing either
  way). A deck lands in your
  decks directory (`--workspace <dir>` puts it inside a workspace; `--force`
  overwrites a same-named deck); a folder lands under its own name beside
  your other decks and is never overwritten. Personal files that leaked from
  the sender's side are stripped on arrival, and an archive carrying a symbolic
  link is refused before anything lands, so the sender cannot decide what
  appears in your decks folder: ask them for one that carries the file itself.

## Config & health

- `alix config`: show the active key bindings; `alix config --init` writes the
  file.
- `alix bug-report [--out <dir>] [--include-deck <path>]`: write a local ZIP
  containing the bounded diagnostic logs, platform, version, and build
  commit details, a config without tokens or AI prompt guidance, and hashed
  deck identities with aggregate counts. Every included file is plain
  text. No deck content is included by default. `--include-deck` adds exactly
  the named deck verbatim, including its card text and authored notes, and
  names it in `report.md`. Personal sidecars, prompts, and responses always
  stay out. The command never uploads or sends the archive; review it before
  attaching it yourself. It uses the default profile when one is selected,
  otherwise the default config and decks directory.
- `alix doctor [dir-or-deck]`: with no argument or a directory, environment
  health checks, a one-line remedy per problem: the config parses, the
  current profile's local log path is named, the progress store is readable,
  the decks dir scans, and the backend CLI, the `share` helper (`wormhole`),
  and the `diagrams` renderer (`sekien`) are on your PATH. Name a **deck
  file** instead to lint it in depth (syntax and named-field `at:` locators);
  that form runs the
  deck lint only, none of the environment rows. It withholds stale excerpts and reports
  a unique exact relocation, changed content, ambiguity, or a missing
  fingerprint. alix does not recognize or rewrite old deck formats; a deck
  written in one fails as ordinary invalid input (an unknown key, an id or
  locator that fails the current grammar). Over a
  **folder or workspace** it also
  reports identity problems across the decks as a set: duplicate deck or card
  tokens (naming which copy keeps the earned progress), store keys matching no
  live card or deck (orphans, clear them with `alix reset --orphans`), a
  non-canonical token, a frontmatter that can't be stamped, an id marker away
  from its card's closing line (the position stamping mints at), and cards
  and topics still awaiting a token. A topic ID held by two topics, in one
  deck or in two, is an error naming the copy and the line to delete before
  running `alix deck init`. For `requires:` it separates a dangling filename edge
  from a dangling deck-id edge, a `card-…` id pasted where a deck belongs, and
  a file that only shares a required id's name (the id wins, so add the `.md`
  extension to mean the file). It flags a `cite:` root pointing into `assets/`
  (a deck keeps its real source, never its frozen excerpt fragments). It
  warns when a sourced topic's exam prompt exceeds a backend's limit, with
  the line the exam would refuse with
  ([how big a topic may be](12-the-ai-exam.md#how-big-a-topic-may-be)). It also
  names deck-like Markdown ignored until explicitly
  initialized, invalid or orphaned per-deck progress or augmentation documents,
  and synchronization conflict copies. Workspace checks also reject live source
  evidence, missing or cross-deck assets, local images outside the owning deck
  directory, and SHA-256 filenames that do not match their bytes. They warn
  about a frozen object the deck no longer references and about any non-deck
  entry in `decks/` (hidden files, personal `.local.md` files, `README`/`LICENSE`,
  backups and conflict copies excepted). A live `at:` citation that does not
  resolve is an error. A locator whose key is missing or names no `cite:` root,
  that breaks the locator grammar, or that does not fit its root (a file under
  a URL root, a bare key under a directory root, a URL passage without an
  `asset:`) is an error in every mode, repair flags included, frozen or live,
  trace decks included. In `alix.toml` it warns about each unknown key or
  invalid value in `[defaults]`, which alix ignores, and reports `cite`
  inside `[defaults]` as an error naming where the key belongs. An
  `alix.toml` that does not parse is an error. `--backends` additionally
  probes the configured AI backend end to end (one real, tiny request);
  `--all-backends` probes all four. `--grading` spot-checks the configured
  model's exam grading against the hand-labeled calibration probes (a few
  real, costed calls, batched by strictness): answers that must not pass
  (wrong, empty, off-topic, incomplete at strict, flawed math derivations)
  and answers that should (correct ones, including full proofs). A failed must-not-pass probe
  is the serious direction (exam grades may be too lenient), while a missed
  should-pass probe only means the grader is harsher than intended. It's a
  spot check, not a certification. Without an explicit repair flag, doctor is
  report-only and fixes nothing.
  With no argument it also checks every launch profile's root identity. Two
  profiles carrying the same `.alix/sync.toml` `root_id` are an error naming
  both profiles. A malformed root id names its profile and file, and a nested
  `.alix/sync.toml` below a profile folder names the profile and relative path.
  A missing file or key is clean and doctor never mints one.
- `alix doctor [dir-or-deck] --normalize`: rewrite each checked deck into its
  canonical bytes, dropping a leading byte-order mark, turning CRLF endings
  into LF, and removing trailing spaces and tabs. A hard line break (two or
  more trailing spaces) is kept as exactly two, and a code fence keeps the
  trailing blanks on every line it owns, opener and closer included. alix normalizes every deck it writes anyway, so this is for
  a deck an editor changed after it was initialized. A rewrite that would stop
  the deck parsing is refused.
- `alix doctor [dir-or-deck] --warn-on-cite-locator-drift`: also warn where
  a frozen excerpt no longer matches its live source: the root or file is
  gone, the text changed, it occurs more than once, or it moved to other
  lines. Frozen evidence going stale is normal (a shared frozen deck works
  without its sources), so without this flag doctor says nothing about it,
  and with it the finding is a warning, never an error. A live citation that
  does not resolve is an error either way.
- `alix doctor [dir-or-deck] --repair-cite-locators`: after you review the
  reported citations, stamp fingerprints on currently addressed excerpts and
  rebase any whose lines moved while their content stayed identical, frozen
  excerpts included (it finds moved frozen excerpts itself, with or without
  `--warn-on-cite-locator-drift`). A whole-file citation is never narrowed
  to a range. A rebase corrects the `at:` line numbers only; the frozen
  evidence and its fingerprint are never rewritten. Changed or multiply
  matching excerpts remain untouched and make the command fail, because whether
  such a card still teaches the truth is a reader's call. Deck and card IDs are
  preserved.
- `alix doctor [dir-or-deck] --warn-on-source-drift`: also warn where a
  deck source's live file changed, is gone, is a symbolic link, or cannot be
  read, and name each key that has sources but no usable root on this
  machine, with the `alix deck source root set` command that saves one
  ([sources](12-the-ai-exam.md#sources)). Damage to a sources folder is an
  error with or without the flag.
- `alix doctor [dir-or-deck] --repair-sources [--keep-results]`: refreeze
  each deck source whose live file changed, keeping its bindings and
  printing each kept range for review. Refused as a whole when two sources
  would hold the same bytes or a range would lie beyond a new copy's last
  line. A live file that is gone is reported and its source kept; a key
  without a usable root is never read, and the warning names why with the
  `alix deck source root set` command that fixes it. Copies no row names
  are deleted. A topic over a backend's limit never stops a repair; the
  doctor findings printed afterwards warn about it as they always do.
  `--keep-results` (only beside `--repair-sources`) keeps each
  refrozen topic's result and the standing, as
  `alix deck source update --keep-results` does.
- `alix doctor [dir-or-deck] --repair-diagrams`: after you review the
  reported [diagram findings](06-cloze-direction-images.md), delete stamps
  attached to no fence and re-freeze every stale or unfrozen fence
  (workspace members only; needs the renderer on PATH). Orphan removal is
  whole-line and atomic; a second run has nothing to do.
- `alix doctor [dir-or-deck] --repair-positions`: after you review the
  reported [span anchor divergences](06-cloze-direction-images.md), rewrite
  each diverged `position:` anchor to where its span binds today (the
  keep-what-you-authored resolution). To keep an anchor's old target
  instead, set `occurrence=` yourself; doctor never retargets a span on its
  own.
- `alix doctor [dir-or-deck] --repair-frontmatter-order`: rewrite each
  checked deck's frontmatter into the
  [canonical key order](03-the-deck-format.md) (authored keys first, machine
  lines like `id` last). Opt-in only: doctor never diagnoses your own order,
  and frontmatter it cannot safely permute (a blank line or comment inside
  the block) is left as-is with a note. Card and deck IDs are preserved.
- `alix doctor [dir-or-deck] --repair-comment-order`: rewrite each checked
  deck's trailing comment machinery into the canonical order (invocation,
  directives, region comments, `at:` locator, `id` last). Opt-in only:
  any order parses, an editorial comment or content bounds what may move,
  and IDs are preserved.
- Folder and workspace runs also count accumulated `.bak` backup files
  (overwrite leftovers, including the progress backup an accepted
  paired-phone push leaves under `.alix/progress`) with their total size,
  naming both remedies:
  `alix deck restore <deck>` swaps one back,
  `alix doctor <dir> --remove-backup-files` lists and deletes them all after
  one confirmation (`--yes` skips it). Backups warn, they never fail the
  run.
- `--config <path>`: use a different config file.
