# 0005: Progress-store durability and writer model

- Status: Accepted
- Recorded: 2026-07-24
- Retrospective: Yes
- Refined by: [ADR 0017](0017-per-deck-state-documents.md), which narrows the
  active-writer and conflict boundary from a workspace store to a deck document.

## Decision history

The local placement and ownership direction is recorded in ADR 0001. Commit
`80f2bd7` made bare-deck progress folder-local on 2026-07-12, `b590425`
documented the self-contained synchronization model, and `d390d14` routed
mutating commands through the same store resolution. Commit `cde778c` added a
last-writer marker and synchronization-conflict detection on 2026-07-15.

Commit `2c088bd` had previously pinned the store version at 1 and removed the
forward-version fence on 2026-07-04. This was an explicit pre-1.0 choice:
persisted state loads best-effort while the shape is still changing.

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

Progress is stored as JSON containing a version field. Normal saves serialize
a complete new state to a sibling temporary file and rename it over the
destination. Recent activity uses the same replacement pattern.

Before 1.0, the version remains pinned at 1 and is informational. Readers use
Serde defaults and lenient handling for selected regenerable entries. They do
not refuse a higher version and there is no general migration or automatic
backup framework. This limitation is explicit: best-effort survival is not yet
a stable compatibility promise.

The supported model is one active writer at a time. The store records the last
device and write time. Alix warns about a recent foreign writer and detects
Syncthing-style conflict copies. These guards reveal likely divergence; they
do not merge it.

Atomic replacement guarantees that readers see the old complete file or the
new complete file. It does not make simultaneous writers safe.

## Consequences

- Crashes during a normal write do not expose a partially serialized store.
- The whole JSON document is rewritten for a save.
- External folder synchronization works for sequential roaming.
- Learners must avoid simultaneous review on several devices.
- Conflict copies and recent foreign writes remain actionable evidence instead
  of being silently ignored.
- A pre-1.0 binary can load and later rewrite a store containing unknown newer
  fields, so forward compatibility is not guaranteed.
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

### Enforce version fences and migrations before 1.0

This would protect forward compatibility, but each rapid pre-1.0 shape change
would require a durable migration contract. The project explicitly deferred
that promise while retaining the version field as the future seam.

## Compatibility

The `progress.json` version, serialized card and deck state, and writer marker
are persisted surfaces. Today, unknown fields are ignored and a higher version
is not rejected. Before Alix claims production-grade compatibility, version
increments must introduce explicit newer-version refusal or a compatible
preservation rule, plus backed-up migrations for breaking changes.

## Security

The writer marker is a conflict signal, not authentication. A local or synced
file can be modified by anything with filesystem access. Store parsing must
remain bounded and must not execute content.

## Verification

- `src/store.rs` owns format loading, complete-file saves, writer detection,
  conflict-copy discovery, and their tests.
- `src/recent.rs` uses the same temporary-file replacement pattern.
- `src/workspace.rs` and `src/cli/common.rs` resolve the correct folder-local
  store for every operation.
- The `loads_any_version` store test preserves the current pre-1.0
  best-effort policy.

## Reversal

Replace this model when supported simultaneous multi-device writing makes
conflict detection insufficient, or measured scale makes complete-file writes
unacceptable. A replacement requires a versioned migration, backup and
rollback, crash and fault-injection tests, and documented merge semantics for
every kind of progress.
