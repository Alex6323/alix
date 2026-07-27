# 0005: Progress-store durability and writer model

- Status: Accepted
- Recorded: 2026-07-24
- Retrospective: Yes
- Refined by:
  [ADR 0017](0017-per-deck-state-documents.md), which makes one versioned
  document per deck the persistence and conflict boundary, and
  [ADR 0022](0022-workspace-and-user-file-ownership.md), which names progress
  as private user-owned state.

## Decision history

The local placement and ownership direction is recorded in ADR 0001. Commit
`80f2bd7` made bare-deck progress folder-local on 2026-07-12, `b590425`
documented the self-contained synchronization model, and `d390d14` routed
mutating commands through the same store resolution. Commit `cde778c` added a
last-writer marker and synchronization-conflict detection on 2026-07-15.
On 2026-07-26 the shared write path gained data and directory-entry syncing
around the atomic rename, closing the power-loss window that could leave a
replaced document empty; stale sessions now also surface their failed saves
through the review API instead of only the server log.
On 2026-07-27 session mutations stopped deferring: every store mutation
flushes its per-deck document before the response returns, and the server
drains and flushes on Ctrl-C/SIGTERM, replacing the 2026-07-19
session-batched flush whose accepted loss window this closes. An append-only
review journal was considered for the same goal and rejected: document sizes
keep the fsync as the dominant cost either way, and a durability log's
resulting-state deltas would not serve a future merge log's per-device
semantic events, so the journal remains a roadmap option on its original
triggers.

## Context

Review history is valuable personal state written frequently and often kept in
folders synchronized by external tools. A crash must not leave half a JSON
document, an older binary must not reinterpret a newer store, and two devices
must not silently overwrite each other under the pretense of safe concurrent
operation.

Atomic file replacement solves torn writes, but it does not merge divergent
histories. Alix needs an honest writer model before it needs a complex
distributed data structure.

## Decision

Progress is stored in version-1 JSON documents, one per initialized deck.
Normal saves serialize a complete replacement document to a sibling temporary
file and rename it over the destination. Recent activity uses the same
replacement pattern.

Readers reject unsupported document versions and documents whose declared
owner does not match the stable ID in the filename. Before 1.0, a breaking
persisted-state change is a clean format break performed outside production
Alix; production contains no runtime converter or compatibility branch for a
superseded pre-1.0 layout.

The supported model is one active writer at a time. The store records the last
device and write time. Alix warns about a recent foreign writer and detects
Syncthing-style conflict copies. These guards reveal likely divergence; they
do not merge it.

Atomic replacement guarantees that readers see the old complete file or the
new complete file. It does not make simultaneous writers safe.

## Consequences

- Crashes during a normal write do not expose a partially serialized store.
- One deck's whole progress document is rewritten for a save.
- External folder synchronization works for sequential roaming.
- Learners must avoid simultaneous review on several devices.
- Conflict copies and recent foreign writes remain actionable evidence instead
  of being silently ignored.
- A binary refuses an unsupported progress-document version rather than
  rewriting it.
- A future multi-writer design must be explicit and migratable.

## Alternatives considered

### Application database

A database could offer transactions and indexing, but it would weaken the
portable plain-file model in ADR 0001 and would not by itself solve
multi-device merge semantics.

### Event log

An append-only history could support replay and some merges, but it increases
compaction, corruption-recovery, and compatibility complexity before
simultaneous writers are a supported requirement.

### CRDT progress state

A CRDT requires precise merge semantics for grades, schedules, exams, virtual
cards, and destructive operations. Adopting one without a real concurrent
workflow would freeze speculative complexity into the persisted format.

### Last writer wins without warnings

This is simple but presents silent data loss as successful synchronization.

### Ship runtime converters before 1.0

This would preserve preceding development layouts, but each rapid pre-1.0
shape change would add compatibility code and tests to production. Clean
external conversion keeps the shipped implementation equal to the current
design.

## Compatibility

The version, owner ID, revision, serialized card and deck state, and writer
marker in each `progress/<alix-id>.json` document are persisted surfaces.
Unsupported versions are rejected. A future post-1.0 breaking change requires
an explicit compatibility and conversion policy before it ships.

A *hard* break (a bumped or unrecognized document version) is loud: the reader
rejects the file and leaves it on disk untouched. A *soft* break is not. The
pre-1.0 policy permits reshaping a document with `#[serde(default)]` and no
version bump, and no field uses `deny_unknown_fields`. So renaming, removing, or
repurposing a serde field without bumping the version deserializes silently: the
old field is ignored, the new one defaults, and the next save rewrites the file
in the new shape, dropping that dimension of history. Nothing detects it, and a
folder backup only helps a user who happened to snapshot before running the new
binary. This silent-loss channel is accepted for fields that carry no
load-bearing history; a soft break that would touch schedules, review history,
or exam state must instead be gated by a version bump (making it a loud hard
break) or a one-time external conversion, never shipped as a silent
`#[serde(default)]`.

## Security

The writer marker is a conflict signal, not authentication. A local or synced
file can be modified by anything with filesystem access. Store parsing must
remain bounded and must not execute content.

## Verification

- `src/store.rs` owns format loading, complete-file saves, writer detection,
  conflict-copy discovery, and their tests.
- `src/recent.rs` uses the same temporary-file replacement pattern.
- `src/workspace.rs` and `src/cli/common.rs` resolve the correct folder-local
  user root for progress operations.
- Store tests reject unsupported versions, mismatched owners, and stale
  revisions.

## Reversal

Replace this model when supported simultaneous multi-device writing makes
conflict detection insufficient, or measured scale makes complete-file writes
unacceptable. After 1.0, a replacement requires a versioned conversion,
backup and rollback, crash and fault-injection tests, and documented merge
semantics for every kind of progress.
