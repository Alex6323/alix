# 0022: Workspace and user file ownership

- Status: Accepted
- Recorded: 2026-07-27
- Retrospective: No
- Supersedes:
  [ADR 0017](0017-per-deck-state-documents.md)
- Refines:
  [ADR 0001](0001-local-first-files.md),
  [ADR 0019](0019-workspace-artifact-layout.md), and
  [ADR 0021](0021-deck-owned-frozen-assets.md)

## Context

Per-deck progress and augmentation documents share the same stable deck ID, but
they do not share ownership. Progress belongs to one user and must never travel
with a public workspace. Augmentation is regenerable learning material that
does travel with its deck.

Treating both as a generic state layout made `--store` and workspace `store`
overrides relocate augmentation together with progress. Sharing then needed a
private-state root to find a public artifact, and moving a deck required
reasoning about a path abstraction whose name did not identify its owner.

The separation axis is ownership and shareability, not file format or whether a
file changes at runtime.

## Decision

Alix has two explicit filesystem owners:

```text
WorkspaceFiles
├── alix.toml
├── decks/
├── assets/
└── augment/

UserFiles
├── progress/
├── recent.json
└── alix.local.toml
```

`WorkspaceFiles` derives shareable paths from the content root. A workspace
member resolves to its manifest root; a loose deck resolves to its parent.
`augment/<deck-id>.json` therefore stays beside the deck content regardless of
any progress-store override.

`UserFiles` derives private paths from the root appropriate to the operation.
Progress and recent history use the user root selected by `--store`, workspace
`store`, the configured decks directory, or the platform default. The
workspace-scoped `alix.local.toml` remains at the workspace root but is still
addressed as a user-owned file and excluded from every share.

The `--store` option names user files only. `alix deck augment --store` may read
virtual cards from that user's progress document, but it writes augmentation
through `WorkspaceFiles`. A workspace `store` override likewise cannot relocate
augmentation or assets.

The Rust API exposes the two owner types rather than a generic `Layout`,
`Domain`, or untyped path join. Progress opening validates the deck ID once and
passes the validated identity to path derivation and store construction.
Augmentation opening accepts a workspace root or loaded deck, never a progress
store path.

Sharing includes the selected decks, their owned assets, matching augmentation,
and shared manifest. It excludes progress, recent history, and local
configuration. Doctor validates workspace and user documents at their separate
roots and reports conflicts without treating augmentation as private state.

## Consequences

- A user can relocate progress without breaking distractors, notes, formats, or
  topologies shared with the deck.
- A deck bundle can find its complete public material without access to private
  user paths.
- Different users can share one workspace while keeping independent progress
  roots.
- Loose decks retain shareable augmentation beside their Markdown file.
- Code that needs both domains must name both owners explicitly.
- `alix.local.toml` remains workspace-scoped even though it is private.
- The word "store" continues to describe the user progress location; it does
  not describe augmentation ownership.

## Alternatives considered

### Keep one generic layout type

A generic type shortens some signatures but hides the decision callers must
make. It allows a user-selected progress path to accidentally become the
address of a shareable artifact.

### Use `Domain::Shared` and `Domain::User`

An enum makes ownership visible at each join but still permits invalid
combinations at runtime. Separate types expose only paths their owner can
possess and make signatures state the required domain.

### Put augmentation under the user root

That keeps all mutable JSON together but makes a public, regenerable deck
artifact private by location. Sharing and moving a deck would again require
knowledge of a user's progress configuration.

### Embed augmentation in Markdown

Generated material would create noisy authored diffs, make regeneration rewrite
cards, and weaken the distinction between authored content and derived
enrichment.

### Move local configuration into a shared user root

One root reused by several workspaces would make `alix.local.toml` collide or
require another workspace identity layer. Keeping it beside its workspace makes
its scope obvious while sharing exclusions preserve privacy.

## Compatibility

This is an intentional pre-1.0 path break. Production code reads augmentation
only from the content root and progress only from the selected user root. It
contains no dual reader or runtime relocation path.

Maintainer and example workspaces are converted outside production code before
the change is committed. Stable deck and card IDs remain unchanged, so document
filenames and learning identities do not change.

## Security

Progress, recent history, and local configuration may disclose personal study
behavior or machine-local settings. All sharing and archive paths exclude them
recursively.

Augmentation may disclose model-generated explanations derived from a deck and
is intentionally shared. Assets may contain copied source or media and remain
subject to ADR 0021's explicit export boundary.

Neither owner type is an access-control mechanism. Filesystem permissions and
the sharing sanitizer remain the enforcement boundaries.

## Verification

- `WorkspaceFiles` and `UserFiles` unit tests pin exact path derivation.
- CLI tests prove `--store` still supplies virtual cards while augmentation is
  written only under the content root.
- Workspace tests prove a `store` override does not relocate augmentation.
- Sharing tests include matching augmentation and assets while excluding every
  user file.
- Doctor tests discover progress and augmentation at their distinct roots and
  surface conflicts and orphaned documents.
- Desktop, web, CLI, and mobile callers compile without the removed generic
  layout or progress-root augmentation constructors.
- The implementation seams are `src/workspace.rs`, `src/state.rs`,
  `src/augment.rs`, `src/share.rs`, and the mobile review bridge.

## Reversal

Replace the owner types only if Alix adopts a different durable sharing
boundary. A replacement must keep private study history out of public bundles,
preserve per-deck stable identity, make user-root overrides unable to relocate
workspace content accidentally, and define one path contract shared by every
client.
