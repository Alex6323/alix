# 0019: Workspace artifact layout

- Status: Accepted
- Recorded: 2026-07-26
- Retrospective: No
- Refines: [ADR 0001](0001-local-first-files.md),
  [ADR 0015](0015-frozen-source-snapshots.md), and
  [ADR 0017](0017-per-deck-state-documents.md)
- Refined by:
  [ADR 0022](0022-workspace-and-user-file-ownership.md)

## Context

An Alix workspace owns several artifact classes: shared configuration, personal
configuration, authored Markdown decks, frozen assets, private progress, and
shareable augmentation. The initial workspace layout stored deck files beside
all of the other root artifacts.

That flat arrangement makes `alix.toml` visually compete with an arbitrary
number of decks and overloads a member deck's parent directory as the manifest
root, state root, asset root, source-reference root, and prerequisite directory.
The resulting assumption is repeated across desktop, web, mobile, sharing,
generation, doctor, and source grounding.

The workspace is already the portable content boundary and the default
location for private user files. Its internal artifact classes need one fixed
structure that every client can derive without recursive search.

## Decision

A workspace uses this layout:

```text
<workspace>/
├── alix.toml
├── alix.local.toml
├── decks/
│   └── <deck>.md
├── assets/
├── progress/
└── augment/
```

As refined by ADR 0022, this tree shows the default colocated user files.
`store` may relocate progress and recent history, while the manifest, decks,
assets, augmentation, and workspace-scoped local manifest remain anchored to
the workspace.

`alix.toml`, when present at a directory root, establishes the workspace
structure. Initialized member decks are direct children of `decks/`. Root-level
Markdown is not workspace membership and may be ordinary documentation.
Discovery is not recursive.

A plain directory without `alix.toml` remains a loose-deck container and
discovers initialized direct `*.md` children. The `decks/` directory is required
only inside a workspace.

Every path role has one anchor:

- shared and local manifests, assets, augmentation, icons, and relative
  `store` overrides use the workspace root;
- progress uses the selected user root, which defaults to the workspace;
- relative `source`, `origin`, and image references authored in a workspace
  member also use the workspace root;
- `requires` resolves among sibling members in `decks/`;
- loose-deck references continue to use the loose deck's parent.

Creation and ingestion paths receive a workspace root and derive its member
directory. Sharing preserves the same tree, includes matching augmentation,
and excludes private progress and local configuration.

Production code implements only this structure. Pre-1.0 workspace files are
moved externally and keep their existing `alix-id` and card IDs.

## Consequences

- Workspace roots remain readable as manifests plus named artifact
  directories even when they contain many decks.
- Member discovery, state routing, source grounding, and asset lookup no longer
  infer unrelated boundaries from `deck.parent()`.
- `source: assets`, workspace images, and relative origins remain concise after
  deck files move one level deeper.
- A root-level Markdown file in a workspace can safely be documentation but
  cannot be reviewed as a member until placed under `decks/`.
- Moving a deck between a plain folder and a workspace may change the anchor
  for its relative content paths, which is visible and deliberate.
- Tools that create or receive members must know whether their destination is a
  plain decks directory or a workspace root.
- Nested workspaces and recursive member trees remain unsupported.

## Alternatives considered

### Keep all artifacts flat

This preserves fewer path components but leaves unrelated artifact classes
mixed and preserves the repeated parent-directory assumption.

### Discover members from both root and `decks/`

Two locations create ambiguous duplicate handling and path anchoring. They
would also preserve a permanent branch solely for abandoned pre-1.0 data.

### Configure the member directory in `alix.toml`

Configurable structure makes equivalent workspaces non-uniform and increases
the validation, sharing, and client contract without a demonstrated need.

### Resolve member paths from `decks/`

That makes workspace-owned assets and frozen sources require `../assets`
references. Anchoring portable member content at the workspace root is clearer
and preserves existing deck text through the external move.

### Put progress and augmentation under `decks/`

Those files are state, not authored Markdown. Mixing them into `decks/` would
weaken the directory's meaning and provide no synchronization benefit.

## Compatibility

This is an intentional pre-1.0 layout break. Production code recognizes only
workspace members under `decks/` and has no flat-layout reader or converter.
Existing workspaces are backed up and moved externally before release.

Deck and card identities do not change, so their per-deck progress and
augmentation documents remain addressed by the same IDs even when progress
uses a separate user root.

## Security

The change does not expand trust. Markdown, manifests, assets, and sources
remain untrusted filesystem input.

The fixed member boundary narrows automatic deck discovery: root documentation
inside a workspace cannot become reviewable or writable merely because it
resembles a deck. Explicit deck initialization remains the write-authority
boundary from ADR 0018.

## Verification

- Workspace tests pin the strict member directory, plain-folder behavior,
  member-to-root recognition, and root-level Markdown exclusion.
- Source and deck tests pin workspace-root anchoring for sources, origins,
  images, snapshots, and manifest defaults.
- Listing, picker, doctor, dependency, state, sharing, generation, CLI, server,
  and mobile tests use the same layout.
- Tracked examples, bundled samples, E2E fixtures, and maintainer workspaces
  contain no root-level member decks.
- `make pre-1-0-check` guards the production implementation against
  backwards-compatibility machinery.

## Reversal

Replace the fixed layout only if measured user workflows require nested member
trees or another portable artifact boundary. A replacement must keep member
discovery deterministic, preserve stable deck identity and per-deck state,
maintain explicit source and asset anchors, and define one structure shared by
desktop, web, and mobile.
