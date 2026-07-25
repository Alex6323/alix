# 0017: Per-deck state documents

- Status: Accepted
- Recorded: 2026-07-25
- Retrospective: No
- Refines: [ADR 0001](0001-local-first-files.md) and
  [ADR 0005](0005-progress-store-durability.md)

## Context

ADR 0001 separates canonical Markdown content from personal review state and
ADR 0005 adopts one active writer for that state. The current implementation
places every deck's cards, deck progress, records, virtual cards, and writer
marker in one workspace-wide `progress.json`. It likewise places every deck's
generated augmentation and topology data in one `augment.json`.

Studying the same workspace on several devices is a likely normal workflow.
With workspace-wide files, reviewing one deck on a phone and another deck on a
desktop rewrites the same progress file even though the learning histories are
independent. Generating augmentation for unrelated decks has the same
unnecessary conflict boundary.

Alix already gives every deck a stable minted identity, stored under
`alix-id` in the Markdown frontmatter, that survives filename changes. The
persisted-data boundary can therefore follow the domain object that changes
independently rather than the mutable path chosen by the learner. This is
aggregate-per-file persistence, not scale-oriented database sharding: the
purpose is to isolate ownership, synchronization, corruption, migration, and
recovery by deck.

## Decision

Progress and augmentation are stored as separate, versioned JSON documents per
deck. Their logical layout is:

```text
progress/<deck-id>.json
augment/<deck-id>.json
```

The filename is the stable deck ID alone. It never includes the deck filename,
title, or path, so ordinary renames cannot stale the association.

The three persisted roles remain distinct:

- `<deck>.md` is canonical authored content. It may contain low-churn machine
  directives for identity and provenance, but never volatile review history or
  generated augmentation state.
- `progress/<deck-id>.json` is private, indispensable learner state. It owns
  the deck's card schedules and history, deck progress, records, virtual cards,
  writer metadata, and any future review-event representation.
- `augment/<deck-id>.json` is shareable, derived, regenerable enrichment. It
  owns that deck's generated distractors, notes, variants, key points, format
  suggestions, fingerprints, and topologies.

The active-writer boundary is one progress document. Different devices may
write progress for different decks in the same synchronized workspace. The
same deck continues to require sequential handoff until Alix defines and
implements explicit same-deck merge semantics.

Each state document carries its format version, owning deck ID, and a revision
or generation. Local saves use a sibling temporary file and atomic rename. A
writer must reject a stale base revision when it can observe one rather than
silently overwriting it. This optimistic check improves local and post-sync
detection; it is not presented as a distributed lock while devices are
disconnected.

The state layout itself is versioned. A client that encounters a newer layout
or an in-progress migration must fail closed instead of creating the legacy
workspace-wide files beside it. Layout migration uses an exclusive guard,
backup, validation, and rollback. Ordinary review does not acquire a
workspace-wide synchronization lock.

Conflicting copies are preserved and surfaced. Modification times, device
labels, and deterministic filename ordering may help diagnosis but never choose
the semantically correct learning history automatically.

Workspace views load and aggregate the relevant deck documents. No central
mutable index becomes authoritative; a disposable local index remains
permissible if measured performance later requires one.

Simultaneous offline writing to the same deck remains an extension point. The
preferred candidate is immutable, uniquely identified review events written in
per-device streams and deterministically replayed into derived scheduling
state. That design may use CRDT-like set union, but it requires explicit
semantics for concurrent reviews, FSRS ordering, exams, virtual cards, resets,
and deletion before it can replace the single-writer deck document.

## Precedents and lessons

These systems validate the design family; Alix does not adopt them as runtime
dependencies.

### Joplin item synchronization

[Joplin's synchronization architecture](https://joplinapp.org/help/dev/spec/sync)
treats notes, notebooks, tags, and resources as independently identified sync
items behind a filesystem-like adapter. It also versions the sync target,
records a minimum compatible application version, and gives clients stable
identities. Its
[conflict guidance](https://joplinapp.org/help/apps/conflict/) preserves the
local version for inspection instead of pretending that an automatic winner is
a semantic merge.

Alix adopts stable-ID item boundaries, explicit layout compatibility,
fail-closed migration behavior, and preserved conflicts. It does not copy
Joplin's local database, hosted synchronization service, account model, or
general-purpose sync engine. External folder synchronization remains an
operator choice.

Joplin's
[synchronization locks](https://joplinapp.org/help/dev/spec/sync_lock/)
also separate ordinary concurrent synchronization from exclusive sync-target
migration. Alix adopts that distinction: migrations need exclusive fencing,
while normal writes are isolated by deck and do not claim a workspace-wide
lock.

### CouchDB documents and revisions

[CouchDB's replication model](https://docs.couchdb.org/en/stable/replication/conflicts.html)
uses independently versioned JSON documents as update and conflict boundaries.
A stale revision is rejected on one node, while divergent offline revisions
remain explicit conflicts that the application must resolve.

Alix adopts a deck-sized document boundary, revision-aware writes, and visible
conflicts. It does not embed CouchDB, implement its revision trees, or claim
that a generation field makes arbitrary filesystem replication transactional.

### Maildir files and atomic delivery

The [Maildir design](https://cr.yp.to/proto/maildir.html) stores each message
under a unique name, writes through a temporary location, and lets unrelated
messages change independently without a mailbox-wide lock.

Alix adopts the same isolation principle and retains temporary-file plus rename
writes. It chooses one file per deck rather than one file per review because a
deck is the present ownership and recovery aggregate. A future immutable event
representation may revisit the finer boundary.

### Git object identity

[Git's object model](https://git-scm.com/docs/gitdatamodel.html) addresses
objects by stable IDs rather than mutable working-tree names and separates
immutable objects from mutable references.

Alix adopts identity-derived state paths and keeps the door open to separating
immutable review events from derived mutable schedules. Deck IDs remain minted
identities rather than content hashes because authored edits must preserve the
same deck identity.

### Syncthing conflict units

[Syncthing detects conflicts per file](https://docs.syncthing.net/users/syncing)
and preserves the losing version as a propagated conflict copy because it
cannot know which content is semantically correct.

Alix therefore makes the independently owned deck the file-level conflict unit
and treats Syncthing conflict copies as actionable histories. It does not use
Syncthing's modification-time winner as a learning-state resolution rule.

## Consequences

- Two devices can study different decks in one synchronized workspace without
  rewriting the same progress document.
- Renaming a deck file does not rename or orphan its state document.
- A malformed or conflicting document has a deck-sized blast radius.
- Loading a workspace requires discovering and aggregating several small files.
- Operations that truly mutate several decks cannot assume a transactional
  multi-file write and must define ordering, journaling, or recovery.
- Moving one deck between workspaces must move or reconnect both of its
  stable-ID documents; doctor must report orphaned or duplicated state.
- Sharing includes the selected decks' augmentation documents and excludes
  every progress document.
- Regenerable augmentation conflicts may be discarded and rebuilt, while
  progress conflicts require preservation and deliberate recovery.
- Simultaneous offline review of the same deck remains unsupported until its
  merge semantics are designed and migrated explicitly.
- A large workspace creates more files, but ordinary deck counts do not justify
  hash-prefix directory fan-out or a database index.

## Alternatives considered

### Keep one progress and augmentation file per workspace

This is simple to load and currently implemented, but it turns unrelated deck
activity into competing writes and makes the active-writer rule unnecessarily
workspace-wide.

### Embed personal state as Markdown machine lines

This would make a deck file superficially self-contained, but every review
would rewrite authored content, pollute version-control history, conflict with
editors, prevent clean use of read-only decks, and leak personal history unless
every sharing path scrubbed it. Stable identity and provenance directives are
low-churn shared metadata; progress is high-churn private state.

### Prefix state files with deck filenames

Human-readable paths would make inspection easier, but renaming a deck would
stale or rename its state. The minted deck ID is already the authoritative,
location-independent key.

### Use SQLite or another application database

A database would add transactions and indexes on one device, but an opaque
database file remains one synchronization conflict unit and cannot merge
independent offline replicas. It would also weaken direct inspection and
recovery. A database remains acceptable as a disposable index or as the store
behind a future online coordinator, not as the synchronized source of truth.

### Store one JSON document per card

This narrows conflicts further but creates many files and complicates deck
mastery, exams, virtual-card lifecycle, bulk reset, backup, and migration. The
deck is the smaller useful aggregate until same-deck concurrency supplies
evidence for immutable per-review events.

### Implement CRDT-like review merging immediately

Set union can merge uniquely identified events, but it does not decide how
concurrent grades affect FSRS, whether both attempts count, or how non-review
operations merge. The per-deck layout preserves this path without freezing
speculative semantics into the first migration.

## Compatibility

The current workspace-wide `progress.json` and `augment.json` are persisted
pre-1.0 formats. Migration must:

1. back up both source files;
2. map card, deck, topology, record, and virtual-card entries to stable deck
   IDs;
3. quarantine ambiguous or orphaned entries instead of dropping them;
4. write and validate every new document before retiring either source file;
5. preserve a rollback path; and
6. prevent older clients from recreating the legacy layout after migration.

Card and deck IDs do not change. Sharing, receiving, doctor, reset, promotion,
augmentation, listing, web, CLI, and mobile bridge paths must resolve the same
per-deck documents.

## Security

Progress documents remain private personal state and must be excluded from
sharing at every depth. Augmentation documents remain intentionally shareable
deck material and may reveal generated explanations or variants derived from
the deck.

Deck IDs, revisions, generations, device labels, and lock files are integrity
and coordination metadata, not authentication secrets. Anything with
filesystem access can modify them.

## Verification

- Migration tests cover backup, interruption, rollback, ambiguous ownership,
  virtual cards, topologies, and older-client refusal.
- Store tests prove that different decks resolve different documents, deck
  renames preserve resolution, stale generations are rejected, and atomic
  replacement survives failure.
- Synchronization tests preserve same-deck conflict copies while allowing
  independent deck writes.
- Sharing tests include matching augmentation documents and exclude all
  progress documents.
- Doctor reports missing deck IDs, duplicate IDs, orphaned state, incompatible
  versions, incomplete migration, and synchronization conflicts.
- Contract tests keep desktop, web, CLI, and mobile resolution behavior aligned.
- `src/workspace.rs`, `src/store.rs`, `src/augment.rs`, `src/share.rs`, and the
  mobile bridge are the primary implementation seams.

## Reversal

Replace aggregate-per-deck files if measured workspace scale makes discovery
unacceptable, if same-deck concurrent review becomes a supported requirement,
or if an online coordination service becomes part of Alix's product boundary.

A replacement must preserve Markdown as canonical authored content, migrate
every deck document without silent loss, retain stable identity, keep private
progress out of shares, provide backup and rollback, and define conflict
semantics rather than hiding them behind database or synchronization behavior.
