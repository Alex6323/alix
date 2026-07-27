# 0023: Semantic workspace update

- Status: Accepted
- Recorded: 2026-07-27
- Retrospective: No
- Refines:
  [ADR 0021](0021-deck-owned-frozen-assets.md) and
  [ADR 0022](0022-workspace-and-user-file-ownership.md)

## Context

ADR 0021 makes frozen deck-owned evidence authoritative and reserves workspace
update as the deliberate boundary for reconciling a deck with its live origin.
Refreshing only an excerpt could make a stale card appear current, while
regenerating a deck can silently transfer stable IDs and learning history to
different propositions.

The user must review the exact proposed cards and evidence before publication.
The proposal may span several decks, and apply must not depend on another
non-deterministic model call.

## Decision

`alix workspace update <workspace>` creates a dot-prefixed sibling staging
workspace and does not modify the authoritative workspace. It reconciles every
eligible frozen member against its recorded local origin, freezes the proposed
evidence, validates the complete staged workspace, records exact baseline
digests, and reports retained, retired, and new cards for review.

`alix workspace update <workspace> --apply` publishes that exact proposal
without calling the model again. It refuses if an authoritative deck changed
since staging. `--discard` removes the proposal without changing the workspace.

A card ID names one learning proposition. An existing ID may survive only when
the identity-bearing parsed learning content is unchanged. Notes, presentation,
and evidence locators may change without changing identity. A changed or
obsolete proposition removes its complete old card block and retires that ID.
A replacement or new proposition carries no authored ID in the model proposal
and receives a fresh ID during staging. Unknown, duplicated, reassigned, or
content-changing retained IDs make the complete proposal fail.

Apply validates every deck, dependency, citation, image, and content-addressed
asset before publication. It writes new immutable assets before deck
references, atomically replaces deck files, restores earlier deck bytes if a
later replacement fails, and retains old assets. Matching shared augmentation
entries for retired IDs are removed after public deck publication.

Private progress is not part of the shared update transaction. Progress under a
retired ID remains explicit orphan evidence and can be removed with
`alix reset --orphans`. A retired ID never becomes active again.

The first implementation accepts local file and directory origins. Remote
capture remains unsupported until its trust and export policy is designed.

## Consequences

- The learner reviews the exact proposal that apply publishes.
- Card history cannot move to a rewritten proposition by position or model
  judgment.
- Even a small question or answer edit receives a new ID under the conservative
  first implementation.
- Note, formatting, and source-location maintenance can retain identity.
- Updating several decks needs staging space for complete owned asset
  directories.
- A stale proposal cannot merge itself over concurrent deck edits.
- Old assets and retired private progress require explicit cleanup.
- URL-backed workspaces cannot use this first update implementation.

## Alternatives considered

### Rewrite cards in place and preserve IDs

This treats IDs as file positions and can attach mature scheduling state to a
different question. Textual similarity is not a sufficient identity contract.

### Ask the update model whether identity survived

The model performing the rewrite is not an independent validator. A
machine-checked conservative signature removes that judgment from the model.

### Apply the proposal immediately

This removes the user review boundary and turns a model call into an
authoritative workspace mutation.

### Generate again during apply

The second output may differ from the reviewed proposal. Apply must consume
immutable staged bytes.

### Refresh only excerpts that still fingerprint-match

Even unchanged evidence can sit below an obsolete or misleading card. Update is
a semantic card review, not an asset synchronization command.

### Delete progress for retired IDs

Progress is private evidence and may be synchronized independently on several
devices. A public workspace mutation must not silently destroy it.

## Compatibility

This adds a command and an ephemeral staging format before 1.0. Production
recognizes only the current staging manifest version. There is no reader for
older proposal formats.

Deck and retained card IDs remain stable. Replaced and obsolete card IDs are
retired rather than reassigned. New cards receive fresh IDs before review.

## Security

The update backend receives read-only access to each recorded local origin and
the current deck. It receives no write or shell access.

Staging may contain source excerpts and images that are not yet published.
It is dot-prefixed to keep it out of normal workspace discovery, but filesystem
permissions remain the confidentiality boundary.

Exact baseline and staged digests detect concurrent or accidental byte changes.
They do not authenticate the author. Apply revalidates paths and content
addresses before copying any object.

## Verification

- Identity tests reject a retained ID after any identity-bearing learning
  content changes and accept note-only or citation-only maintenance.
- Staging tests prove new cards are stamped and obsolete cards disappear with
  their IDs.
- CLI tests prove preview leaves the workspace untouched, apply makes no model
  call, discard removes only staging, and stale baselines fail closed.
- Transaction tests prove assets precede references and later deck-write
  failure restores earlier deck bytes.
- Augmentation tests prove retired active IDs are removed while private
  progress remains untouched.
- Doctor validates both authoritative and staged workspaces.
- The implementation seams are `src/workspace_update.rs`, `src/assets.rs`,
  `src/augment.rs`, and `src/cli/deck.rs`.

## Reversal

Relax identity preservation only if Alix gains an independently reviewable
semantic identity proof that is safer than minting a new ID. Replace persistent
staging only if another mechanism guarantees that the exact reviewed bytes are
the bytes published without a second model call.

