# 0001: Local-first plain files

- Status: Accepted
- Recorded: 2026-07-24
- Retrospective: Yes
- Refined by: [ADR 0017](0017-per-deck-state-documents.md), which narrows
  workspace-wide state files to stable-ID per-deck documents.

## Decision history

This record reconstructs a direction introduced incrementally before the ADR
convention existed:

- `9b2755c` (2026-06-16) established local deck and progress files in the
  initial application.
- `ec01769` (2026-06-21) gave each workspace its own progress store.
- `80f2bd7` (2026-07-12) made the decks folder self-contained for bare
  `alix` use.
- `cde778c` (2026-07-15) added writer and synchronization-conflict guards.

The placement and ownership decision predates those later durability guards.
ADR 0005 records the detailed store-write and concurrency model.

## Context

Alix is a learning tool for material people own, edit, and move themselves.
Requiring an account, hosted service, or opaque database would make access to a
deck depend on Alix and would weaken the promise that a deck remains useful
outside the application.

Deck content and personal review state have different sharing and compatibility
needs. Content should be readable, editable, versionable, and intentionally
shareable. Review history and recent activity are personal, machine-written
state that should follow the learner's chosen storage without leaking when a
deck is shared.

## Decision

Markdown deck files are the canonical source for authored learning content.
Users can create and edit them with ordinary text tools, and Alix reads them
directly rather than importing them into an application-owned database.

Personal state is stored in explicit files. A folder or workspace normally
keeps review history in `progress.json` and recent activity in `recent.json`
beside its decks. Configuration may override the progress-store path.

Alix itself provides no account or cloud storage. Users may copy or synchronize
their folders with tools they choose. Sharing a folder strips personal state,
including `progress.json`, `recent.json`, and `alix.local.toml`.

## Consequences

- Decks remain readable and editable without Alix.
- Existing file tools provide backup, version control, search, and sync.
- Desktop, mobile, CLI, and web surfaces must preserve the same file semantics.
- File-format compatibility and atomic writes are product requirements.
- Alix cannot assume transactional multi-file updates or simultaneous writers.
- Concurrent offline review needs conflict detection or a future merge design;
  the current guidance is to review on one device at a time.

## Alternatives considered

### Application database as the canonical store

A database would simplify transactions, indexing, and some migrations, but
would make content opaque to normal editors and turn import/export into a
compatibility boundary. That conflicts with the product's Markdown-native
identity.

### Hosted account and synchronization service

A hosted service could coordinate concurrent writers, but it would add an
account, operational dependency, privacy boundary, and recurring service cost.
Users can already choose their own folder synchronization tool.

### Store review state inside each deck

This would make a single file self-contained, but routine reviews would rewrite
authored content and create noisy diffs. It would also make sharing a deck leak
personal history unless every export scrubbed it.

## Compatibility

Markdown syntax, card identity directives, and the versioned `progress.json`
shape are persisted surfaces. The current pre-1.0 store deliberately provides
best-effort loading rather than a forward-version fence; ADR 0005 records that
temporary limitation. A stable compatibility promise must preserve existing
authored content and review history or provide an explicit migration with
backup and rollback behavior.

Personal-state file names are also part of the sharing boundary. A new personal
file must be excluded both when staging a share and when receiving one.

## Security

Local-first does not mean every local file is safe to expose. The default server
bind remains local, LAN access requires an explicit opt-in, and model tools must
not receive arbitrary host paths from a client.

Sharing follows data minimization: authored decks and workspace material may
travel, while personal progress, recent activity, local configuration, hidden
files, and backups stay home.

## Verification

- `src/cli/common.rs` resolves deck, folder, and workspace targets without
  importing their content into another store.
- `src/workspace.rs` defines the default workspace store as `progress.json` and
  tests folder-local resolution.
- `src/store.rs` versions the progress format and writes through a temporary
  file followed by rename.
- `src/recent.rs` applies the same temporary-file write pattern to
  `recent.json`.
- `src/share.rs` excludes personal files while staging and receiving shared
  folders, with tests for both directions.

## Reversal

Replace this decision only if plain files can no longer meet measured
correctness, scale, or multi-writer requirements. A replacement must keep
Markdown as a supported interchange format, migrate existing decks and progress
without silent loss, provide backup and rollback, and document who operates any
new synchronization or account boundary.
