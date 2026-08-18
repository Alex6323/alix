# 0024: Deck transfer reuses public bundles

- Status: Accepted
- Evidence: copy_uses_the_public_bundle_without_copying_progress in src/deck_transfer.rs
- Recorded: 2026-07-27
- Retrospective: No
- Refines:
  [ADR 0021](0021-deck-owned-frozen-assets.md) and
  [ADR 0022](0022-workspace-and-user-file-ownership.md)

## Context

A workspace member is a graph of files owned by one stable deck ID. Local copy,
local move, wormhole share, and zip export must agree about which files form the
shareable deck. Independent implementations would eventually diverge and could
either omit required evidence or leak private progress.

A local move has one additional responsibility: the same learner expects
private progress to follow when the source and destination use different user
roots. That private operation must not change the public sharing boundary.

## Decision

Alix has one single-deck public bundle builder and one public bundle installer.
Wormhole sharing, zip sharing, local copy, and local move call those same
functions.

The public bundle contains the deck Markdown, its deck-owned content-addressed
assets, and its matching augmentation document. It excludes progress, recent
history, and local configuration. Receiving and local installation apply the
same validation and assets-before-deck publication order.

`alix deck copy` installs that public bundle and never writes progress.

`alix deck move` installs the public bundle, carries the matching private
progress document when user roots differ, then removes the source deck and its
owned sidecars. The destination is complete before the source deck is removed.
Failure to remove the source deck rolls the destination back. Failure to remove
a sidecar after the source deck has disappeared leaves the complete destination
authoritative and reports the source orphan.

Copy and move preserve the deck filename, stable deck ID, and every card ID.
They refuse overwrite, stable-ID collisions, missing target prerequisites, and
a move that would break a source dependent.

The staged local bundle materializes the source deck's effective origin.
Relative local origins become absolute paths resolved against the source
workspace, so moving the deck cannot reinterpret provenance through the
destination workspace's defaults.

## Consequences

- Public selection and installation cannot drift between network and local
  transfers.
- Moving one deck is proportional to the files owned by that deck ID.
- Copy creates another public placement of the same identity but no second
  private history.
- Workspaces intentionally sharing a user root also share progress for the same
  stable deck ID without copying it.
- Move is fail-safe rather than globally atomic across two roots: after
  destination publication, cleanup can leave source orphans but not lose the
  only complete deck.
- Dependency closures still require deliberate multi-deck operations.

## Alternatives considered

### Implement filesystem copy directly in the move command

This duplicates the public ownership list and would drift from wormhole
sharing as the bundle evolves.

### Put progress in the public deck bundle

That would make local move easy at the cost of leaking private learning history
through wormhole and zip shares.

### Give copy a new deck ID

The content would become a different learning identity and every card would
need new IDs. Copy is placement, not semantic regeneration.

### Delete the source before publishing the destination

An interrupted cross-filesystem operation could lose the only complete deck.
Destination-first publication makes duplication the recoverable failure mode.

### Copy dependencies automatically

This turns a bounded deck operation into an implicit graph migration. The first
operation rejects an incomplete destination instead.

## Compatibility

The commands are additive before 1.0. The deck bundle remains the current
single supported format and gains no alternate local representation.

Stable IDs and document versions do not change. Copy creates no progress
document. Move relocates the current progress bytes without transforming them.

## Security

Local copy uses the same sanitizer and public selection as network sharing, so
private files cannot enter its staged bundle.

Paths from the bundle marker remain restricted to one Markdown filename.
Destination deck-ID collisions and owned-directory collisions fail before
publication. Filesystem permissions remain the boundary for both workspaces and
their user roots.

## Verification

- Share tests prove the public bundle contains assets and augmentation but no
  progress.
- Transfer tests prove copy and move consume that bundle and preserve IDs.
- Move tests cover progress relocation, dependency refusal, destination
  collision, origin materialization, rollback, and orphan reporting.
- CLI tests cover confirmation and user-facing summaries.
- The implementation seams are `src/share.rs`, `src/deck_transfer.rs`,
  `src/cli/deck.rs`, and `src/cli/main.rs`.

## Reversal

Replace the bundle boundary only if workspace ownership changes. Any
replacement must keep one canonical public-file selection for network and local
transfer, keep private history out of shares, and preserve a destination-first
failure mode.

