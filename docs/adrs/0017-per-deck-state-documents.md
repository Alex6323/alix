# 0017: Per-deck state documents

- Status: Accepted
- Recorded: 2026-07-25
- Retrospective: No
- Refines: [ADR 0001](0001-local-first-files.md) and
  [ADR 0005](0005-progress-store-durability.md)

## Context

ADR 0001 separates canonical Markdown content from personal review state and
ADR 0005 adopts one active writer for that state. The initial implementation
grouped all schedules, histories, deck progress, records, virtual cards, and
writer metadata for a workspace into one JSON file. Generated augmentation for
all decks was grouped into a second JSON file.

Studying the same workspace on several devices is a normal workflow. With
workspace-wide files, reviewing one deck on a phone and another on a desktop
rewrites the same progress file even though the learning histories are
independent. Generating augmentation for unrelated decks has the same
unnecessary conflict boundary.

Alix already gives every initialized deck a stable minted identity, stored as
`alix-id` in Markdown frontmatter. That identity survives filename changes.
Persistence can therefore follow the domain object that changes independently
instead of the mutable path chosen by the learner.

## Decision

Progress and augmentation are separate, versioned JSON documents per deck:

```text
<state-root>/
├── progress/<deck-id>.json
└── augment/<deck-id>.json
```

`--store` and workspace `store` settings name the state-root directory. By
default, a loose deck uses its decks folder and a workspace member uses its
workspace folder.

The filename is the stable deck ID alone. It never includes the deck filename,
title, or path, so an ordinary rename does not rename or orphan state.

The three persisted roles remain distinct:

- `<deck>.md` is canonical authored content. It may contain low-churn identity
  and provenance directives, but never volatile review history or generated
  augmentation.
- `progress/<deck-id>.json` is private, indispensable learner state. It owns
  schedules and history, deck progress, records, virtual cards, and writer
  metadata for one deck.
- `augment/<deck-id>.json` is shareable, regenerable enrichment. It owns that
  deck's generated distractors, notes, variants, key points, format
  suggestions, fingerprints, and topologies.

Every state document is version 1 and carries its owning deck ID and revision.
Writes use a sibling temporary file and atomic rename. A save compares the
loaded revision with the on-disk revision and rejects a stale writer instead of
silently overwriting it.

The active-writer boundary is one progress document. Different devices may
write different decks inside one synchronized workspace. The same deck still
requires sequential handoff until Alix defines explicit same-deck merge
semantics. Revisions improve local and post-sync detection; they are not a
distributed lock while devices are disconnected.

Workspace operations discover and aggregate the relevant deck documents in
memory. No central mutable index is authoritative. A disposable index remains
permissible if measured performance later requires one.

Synchronization conflict copies are preserved and surfaced. Modification
times, device labels, and deterministic filename ordering may help diagnosis
but never choose the semantically correct learning history.

Because Alix is pre-1.0, production code implements only this architecture. It
contains no decoder, compatibility branch, layout marker, sentinel, or runtime
conversion for abandoned persisted layouts. Existing maintainer data is
converted outside the application with a disposable tool, verified against an
independent backup, and the tool is then removed. After 1.0, incompatible
format changes require a separately designed upgrade contract.

## Precedents and lessons

These systems validate the design family; Alix does not adopt them as runtime
dependencies.

### Joplin item synchronization

[Joplin's synchronization architecture](https://joplinapp.org/help/dev/spec/sync)
treats notes, notebooks, tags, and resources as independently identified sync
items behind a filesystem-like adapter. Its
[conflict guidance](https://joplinapp.org/help/apps/conflict/) preserves a
conflicting version for inspection instead of pretending an automatic winner
is a semantic merge.

Alix adopts stable-ID item boundaries and preserved conflicts. It does not copy
Joplin's local database, hosted synchronization service, account model, or
general-purpose sync engine.

### CouchDB documents and revisions

[CouchDB's replication model](https://docs.couchdb.org/en/stable/replication/conflicts.html)
uses independently versioned JSON documents as update and conflict boundaries.
A stale revision is rejected on one node, while divergent offline revisions
remain explicit conflicts the application must resolve.

Alix adopts a deck-sized document boundary, revision-aware writes, and visible
conflicts. It does not embed CouchDB, implement revision trees, or claim that a
revision makes filesystem synchronization transactional.

### Maildir files and atomic delivery

The [Maildir design](https://cr.yp.to/proto/maildir.html) stores each message
under a unique name, writes through a temporary location, and lets unrelated
messages change independently without a mailbox-wide lock.

Alix adopts the isolation and atomic-install principles. It chooses one file
per deck rather than one file per review because the deck is the present
ownership and recovery aggregate.

### Git object identity

[Git's object model](https://git-scm.com/docs/gitdatamodel.html) addresses
objects by stable IDs rather than mutable working-tree names.

Alix likewise addresses state by stable identity instead of filename. Deck IDs
remain minted identities rather than content hashes because authored edits
must preserve the same deck identity.

### Syncthing conflict units

[Syncthing detects conflicts per file](https://docs.syncthing.net/users/syncing)
and preserves a conflicting version because it cannot know which content is
semantically correct.

Alix therefore makes the independently owned deck the file-level conflict unit
and treats conflict copies as actionable histories. It does not use
modification time as a learning-state resolution rule.

## Consequences

- Two devices can study different decks in one synchronized workspace without
  rewriting the same progress document.
- Renaming a deck file does not rename or orphan its state document.
- A malformed or conflicting document has a deck-sized blast radius.
- Loading a workspace requires discovering and aggregating several small files.
- Operations that mutate several decks cannot assume a transactional
  multi-file write and must define ordering or recovery.
- Moving a deck between state roots requires moving or reconnecting both of its
  stable-ID documents; doctor must report orphaned or duplicated state.
- Sharing includes selected augmentation documents and excludes all progress.
- Regenerable augmentation conflicts may be discarded and rebuilt, while
  progress conflicts require preservation and deliberate recovery.
- Simultaneous offline review of the same deck remains unsupported.
- A large workspace creates more files, but ordinary deck counts do not justify
  hash-prefix fan-out or a database index.

## Alternatives considered

### Keep workspace-wide JSON documents

This is simple to load, but it turns unrelated deck activity into competing
writes and makes the active-writer rule unnecessarily workspace-wide.

### Embed personal state as Markdown machine lines

Every review would rewrite authored content, pollute version-control history,
conflict with editors, prevent clean use of read-only decks, and leak personal
history unless every sharing path scrubbed it.

### Prefix state files with deck filenames

Human-readable paths would make inspection easier, but renaming a deck would
stale or rename its state. The minted deck ID is already the authoritative,
location-independent key.

### Use SQLite or another application database

A database would add local transactions and indexes, but one opaque database
file remains one synchronization conflict unit and cannot merge independent
offline replicas. A database remains acceptable as a disposable index or
behind a future online coordinator, not as the synchronized source of truth.

### Store one JSON document per card

This narrows conflicts further but creates many files and complicates deck
mastery, exams, virtual-card lifecycle, bulk reset, and backup. The deck is the
smallest useful aggregate until same-deck concurrency supplies evidence for
immutable per-review events.

### Implement CRDT-like review merging immediately

Set union can merge uniquely identified events, but it does not decide how
concurrent grades affect FSRS, whether both attempts count, or how exams,
virtual cards, resets, and deletion merge. Per-deck documents preserve this
future option without freezing speculative semantics now.

## Security

Progress documents remain private personal state and must be excluded from
sharing at every depth. Augmentation documents remain intentionally shareable
deck material and may reveal generated explanations or variants derived from
the deck.

Deck IDs, revisions, and device labels are integrity and coordination metadata,
not authentication secrets. Anything with filesystem access can modify them.

## Verification

- Store tests prove distinct deck documents, rename stability, stale revision
  rejection, replacement rebinding, and atomic writes.
- Synchronization tests preserve same-deck conflict copies while allowing
  independent deck writes.
- Sharing tests include matching augmentation documents and exclude progress.
- Doctor reports missing deck IDs, duplicate IDs, orphaned state, incompatible
  versions, and synchronization conflicts.
- CLI, HTTP API, mobile bridge, and E2E fixtures resolve the same state-root
  layout.
- `src/state.rs`, `src/store.rs`, `src/augment.rs`, `src/share.rs`, and the
  mobile bridge are the primary implementation seams.

## Reversal

Replace per-deck JSON if measured workspace scale makes discovery unacceptable,
if same-deck concurrent review becomes a supported requirement, or if an
online coordination service enters Alix's product boundary.

After 1.0, a replacement must preserve Markdown as canonical authored content,
upgrade every deck document without silent loss, retain stable identity, keep
private progress out of shares, provide backup and rollback, and define
conflict semantics rather than hiding them behind database or synchronization
behavior.
