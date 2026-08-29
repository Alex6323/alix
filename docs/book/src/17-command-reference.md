# 17 · Command reference

A quick index of the `alix` commands. Each links to the chapter that covers it in
depth, where there is one. Run any command with `--help` for its full flags.

## Reviewing

- `alix`: serve the web app: the deck [picker](02-getting-started.md) over
  your decks directory (`~/decks`), printing its URL.
- `alix <dir>`: serve that folder as a **self-contained scoped root**: its own
  catalog and shareable `augment/` and `assets/`, with private per-deck
  `progress/` plus `recent.json` colocated by default. A
  [workspace](08-workspaces.md) dir opens the picker drilled into it; its
  `store` setting may relocate the private user files without moving shareable
  material.

Every review starts from the picker. There's no direct deck launch. Browsing a
deck read-only, sitting the AI exam, and walking a [trace](13-trace-decks.md)
are all reached from the web picker rather than as their own commands (see
[the web app](15-the-web-app.md)).

The single-instance launcher's flags: `--lan` / `--port` / `--token`
([the web app](15-the-web-app.md)), `--session N` (cards per sitting, overriding
the `[review] max_session` config; unrelated to the AI backend's own
`--session-id`), `--config <path>`, and `--log http,select` (enable verbose
file records and mirror the named targets to stderr). The session depth is
picked in the picker's split Depth… menu, an order or region in its focus
drawer ([scheduling](05-scheduling.md)), and the card order is the deck's
`order:` directive.
How each card is checked comes from its `reveal:` combined with the
session's depth ([reveal & session depths](04-review-modes.md)), not a flag.

## Launch profiles

Launch profiles make it easy to run one named alix instance per person in a
household, with its own decks folder, port, and adult or kids frontend. Each
profile is a normal config file under the platform config directory's
`profiles/` folder.

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

## Progress

`alix stats`, `alix list`, and `alix reset` each take a **deck file, a plain
folder, or a [workspace](08-workspaces.md)**: a folder or workspace expands to
its member decks, and each deck resolves to the user-files root the launcher
would serve it with (`--store` > its workspace's store > a served folder or
configured decks root > the global store). Inside that boundary, progress is
loaded from `progress/deck-<token>.json`; folder-wide commands aggregate the
relevant documents in memory without creating an authoritative combined file.

- `alix stats <target>`: progress overview, completion state, and a
  per-depth due count.
- `alix list <target>`: every card with three per-depth cells, shallow to
  deep (Recognize | Recall | Reconstruct), each carrying that depth's state
  and due time; a retired card reads `resting` instead.
- `alix reset <target>`: clear progress (`--card`, `--all`; `-y` to
  skip the prompt). On a workspace it also clears the mastered flags and
  personal-card schedules in the workspace's own store, after one confirmation.
  A target progress document that does not parse is removed with the rest,
  and the prompt says so first; documents outside the target are never
  opened, and an I/O failure stops the reset before anything is erased.
- `alix reset --orphans [target]` clears only **orphaned** progress: store
  keys that match no card or deck in the scanned decks (a stripped
  `<!-- id: … -->` comment, a hand-deleted deck, a double-mint).
  Orphans are never removed automatically (they are evidence), so this is the
  explicit opt-in. It scopes to a named folder/workspace
  store, else the decks-dir root store, and reads every progress document under
  it (the same documents `alix doctor` reports on). A single deck file scopes
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

Both take the same steering options. `--source-url <URL>` records a public
source (added to the deck or workspace `source:`) for later tutor and exam
context after local evidence is frozen. `--goal <TEXT>` scopes what the new
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
  `<deck>.personal.md` is refused outright: it belongs to the deck beside it
  and never gets an `id:` of its own.
- `alix deck augment <deck> --target <...>`: precompute AI augmentations
  (choices, notes, questions, keypoints, format, order). The augmentation
  document stays beside the deck. `--store` affects only the private progress
  needed when the `format` target considers personal cards.
- `alix deck copy <deck> <workspace>`: copy one initialized workspace member,
  its owned frozen assets, and its augmentation into another workspace. Stable
  deck and card IDs are preserved; progress is not copied.
- `alix deck move <deck> <workspace> [--yes]`: move the same public bundle,
  carry progress when the workspaces use different user roots, then remove the
  source. Refuses missing prerequisites and source dependents.
- `alix deck import <file.tsv>`: import an Anki TSV export (no model CLI
  needed; `--workspace <dir>` imports into a workspace).
- `alix deck remove <deck> [--yes]`: remove a deck and everything that is
  its alone: the file, its review history, its frozen assets, its
  augmentations, and any `.bak` backups. Total by design: nothing is backed
  up and it cannot be undone, which the confirmation states along with the
  stakes (cards with progress, reviewed-since date, the exact file list).
  A deck that others `require:` warns and names them; they unlock rather
  than break.
- `alix deck restore <deck>`: swap a deck with its `.bak` backups (file,
  review history, augmentations), undoing the last overwrite (a forced
  import, a trace or workspace regeneration). Nothing is destroyed: the
  swapped-away state becomes the new backup, so running it again swaps
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
- `alix workspace deadline <dir> [<date>|clear]`: show, set, or clear a
  workspace's personal "ready by" date (`--config <path>`); no argument prints
  the current one. Workspace-only, see [Workspaces](08-workspaces.md).
- Tutor: the Ask button (or `?`) in a session, `Ctrl-N` to save a note
  ([the tutor](10-tutor.md)).

The agentic `generate` runs measure the source size before running and prompt
for confirmation when it's large. Pass `--yes` to skip the prompts in
non-interactive scripts. The [AI exam](12-the-ai-exam.md) runs unattended in
the browser instead, so it can't prompt: it truncates an oversized source and
notes it.

## Sharing

- `alix share <path>`: send a deck file, a plain folder, or a workspace to
  someone over [magic-wormhole](https://magic-wormhole.readthedocs.io) (the
  `wormhole` binary must be installed, `alix doctor` checks). A folder is
  staged first so your personal state stays home: `progress/`, the recent list,
  `alix.local.toml`, temporary files, and conflict or backup files never travel.
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
  containing the bounded diagnostic logs, platform and version details, a
  config without tokens or AI prompt guidance, and hashed deck identities with
  aggregate counts. Every included file is plain
  text. No deck content is included by default. `--include-deck` adds exactly
  the named deck verbatim, including its card text and authored notes, and
  names it in `report.md`. Personal sidecars, prompts, and responses always
  stay out. The command never uploads or sends the archive; review it before
  attaching it yourself. It uses the default profile when one is selected,
  otherwise the default config and decks directory.
- `alix doctor [dir-or-deck]`: environment health checks, a one-line remedy per
  problem: the config parses, the current profile's local log path is named,
  the progress store is readable, the decks dir
  scans, and the backend CLI is on your PATH. Name a **deck file** to lint it
  in depth (syntax, named-field `at:` locators, and frozen cards that have
  drifted from their live source). It withholds stale excerpts and reports
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
  still awaiting a token. For `requires:` it separates a dangling filename edge
  from a dangling deck-id edge, a `card-…` id pasted where a deck belongs, a
  file that only shares a required id's name (the id wins, so add the `.md`
  extension to mean the file), and an un-prefixed token it suggests writing as
  `deck-<token>`. It nudges a `source:` that lists more than a few entries
  toward their common directory, and flags a `source:` pointing into `assets/`
  (a deck keeps its real source, never its frozen excerpt fragments). It also
  names deck-like Markdown ignored until explicitly
  initialized, invalid or orphaned per-deck progress or augmentation documents,
  and synchronization conflict copies. Workspace checks also reject live source
  evidence, missing or cross-deck assets, local images outside the owning deck
  directory, and SHA-256 filenames that do not match their bytes. `--backends` additionally
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
- `alix doctor [dir-or-deck] --normalize`: rewrite each checked deck into its
  canonical bytes, dropping a leading byte-order mark, turning CRLF endings
  into LF, and removing trailing spaces and tabs. A hard line break (two or
  more trailing spaces) is kept as exactly two, and a code fence keeps its own
  trailing blanks. alix normalizes every deck it writes anyway, so this is for
  a deck an editor changed after it was initialized. A rewrite that would stop
  the deck parsing is refused.
- `alix doctor [dir-or-deck] --repair-source-locators`: after you review the
  reported citations, stamp fingerprints on currently addressed excerpts and
  rebase any whose lines moved while their content stayed identical, frozen
  excerpts included. A rebase corrects the `at:` line numbers only; the frozen
  evidence and its fingerprint are never rewritten. Changed or multiply
  matching excerpts remain untouched and make the command fail, because whether
  such a card still teaches the truth is a reader's call. Deck and card IDs are
  preserved.
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
  (overwrite leftovers) with their total size, naming both remedies:
  `alix deck restore <deck>` swaps one back,
  `alix doctor <dir> --remove-backup-files` lists and deletes them all after
  one confirmation (`--yes` skips it). Backups warn, they never fail the
  run.
- `--config <path>`: use a different config file.
