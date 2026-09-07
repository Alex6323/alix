# 0044: Private state lives in `.alix/`, and a `.local` twin is the person's

- Status: Accepted
- Evidence: pub struct UserFiles in src/state.rs
- Recorded: 2026-09-07
- Retrospective: No
- Refines:
  [ADR 0019](0019-workspace-artifact-layout.md),
  [ADR 0022](0022-workspace-and-user-file-ownership.md),
  [ADR 0031](0031-a-personal-file-copies-nothing.md), and
  [ADR 0043](0043-private-state-is-colocated-and-belongs-to-the-person.md)

## Context

ADR 0043 settled that private state is colocated with content and that
any split between the two is a property of a feature (share, a folder
shared with other people, a paired device), performed by a tool at the
boundary, never by the layout. That makes the private set the thing
every such tool must agree on. Before this record it was five patterns
at the root of a folder: `progress/`, `recent.json`, `alix.local.toml`,
`<deck>.personal.md`, and `*.json.tmp`, and the maintainer's model of
the layout, one folder for everything that is a person's, had no shape
on disk that a reader could recognize.

The recognizable shape exists in established practice: a tool's own
directory beside the content (`.git/`, `.obsidian/`) for what the tool
manages, and a `.local` twin of a shared file for what the person edits
and does not share (`.env.local`, `settings.local.json`; `*.local.*` is a
standard ignore pattern). The CLI `--store` override and its fallbacks
(the configured decks folder, then the platform data directory) were the
last places where a deck and its progress could live apart, and the last
places where the CLI and the server resolved a loose deck's store
differently.

## Decision

Every store root, a workspace directory or a plain folder of decks,
holds its machine-managed private state in a `.alix/` directory:
`.alix/progress/<deck-id>.json`, `.alix/recent.json`, and the files the
paired-device sync adds (`.alix/sync.toml` for the root id,
`.alix/pull.json` for a pulled entry's manifest, ADR 0042). Hand-edited
private files are `.local` twins of the shared file they extend:
`alix.local.toml` beside `alix.toml`, `<deck>.local.md` beside
`<deck>.md` (the personal file of ADR 0030 and 0031, renamed; its rules
are unchanged). The private set is therefore two patterns, `.alix/` and
`*.local.*`, and one library definition of it feeds share, the
paired-device pull, doctor, and the ignore block the manual recommends;
a test pins the manual's block to that definition.

A deck's store is its folder's store everywhere: the workspace directory
for a member, the containing folder for a loose deck, for the CLI and
the server alike. The `--store` override is removed, with the decks-dir
and platform-data-dir fallbacks. A command that must write into a folder
it cannot write to fails loudly.

The layout is not recognized in its previous shape. Existing folders are
converted by a throwaway script outside the repository (move three
things, rename the sidecars); an unconverted folder shows its decks as
not started.

## Consequences

- A folder shows content only: `alix.toml`, `decks/`, `assets/`,
  `augment/`, plus the person's `.local` twins; everything the machine
  writes is inside `.alix/`.
- Keeping private files out of a folder shared with other people is two
  ignore lines, the same two patterns share strips and the pull adds
  back for the same person.
- A `.alix/` directory marks a store root, which is the nested-root
  signal the paired-device sync needs.
- The tests that drove the binary with `--store` seed and inspect the
  folder's `.alix/progress/` instead; the harness's temporary home still
  isolates the platform directories.
- ADR 0019's tree, ADR 0022's list of user-owned files, and ADR 0043's
  ignore list are read with this layout from this record on.
