# 0025: Backups are out of scope; the folder is the backup unit

- Status: Accepted
- Recorded: 2026-07-27
- Retrospective: No
- Refines: [ADR 0001](0001-local-first-files.md) and
  [ADR 0005](0005-progress-store-durability.md)

## Context

The engineering review (2026-07-23, finding 3) recommends an `alix backup`
command with a portable archive, `alix restore --dry-run`, a retention policy,
and doctor backup-availability checks, as part of making persisted data
production-grade.

That recommendation is written for an application that owns an opaque datastore
(a database file, a server store), where the application is the only thing that
can produce a coherent backup. Alix is the opposite by design (ADR 0001): all
state is plain files in one folder, each a small versioned JSON or Markdown
document the user can read, copy, and diff directly.

## Decision

Alix does not provide a backup or restore command, and does not manage backup
generations, archives, or retention. The backup unit is the decks folder
itself, and the user's own folder tooling (a cloud drive, `git`, `rsync`, Time
Machine, `cp -r`) is the backup mechanism.

Alix's responsibility is narrower and is kept: **Alix's own writes must not
destroy the user's files.** Every state, deck, and manifest write goes through
one crash-safe primitive (`src/fsio.rs::replace_file`): write a sibling
temporary file, flush its data, atomically rename it over the target, and (on
unix) flush the target's directory entry; freshly created ancestor directories
are flushed the same way. Two guarantees follow, and they are not the same:

- **Atomicity** (a crash or interrupted save leaves the previous file intact,
  never a half-written one): tested. A kill-point fault-injection suite injects
  a failure after each write step and asserts the target reads as either the
  old or the new content.
- **Power-loss durability** (the committed bytes survive a hard power loss):
  argued from the ordering (flush the data before the rename, and rely on the
  filesystem's atomic rename), not unit-tested — a unit test cannot drop the
  page cache. The directory-entry flush is a no-op on Windows (std cannot sync
  a directory there), so on Windows this rests entirely on NTFS's own
  journaling. Cross-platform durability testing is Phase-1 open work, not
  claimed here.

The guarantee is also **per document**: an operation that writes several
documents (a folder-wide `reset`, a deck replacement) orders its writes so a
partial completion corrupts no single file, but is not one atomic transaction
(ADR 0001) — see roadmap item `{#aggregate-save-atomicity}`.

Detection of damage that has already happened (an unreadable document, orphaned
progress, a synchronization conflict copy, a stray reserved file) stays with
`alix doctor`, which reports and advises but does not repair.

## Consequences

- No frozen archive format, retention policy, backup naming scheme, or restore
  collision contract is introduced. None of that permanent surface has to be
  designed, versioned, or carried.
- A user who wants their study history protected must keep the decks folder in a
  backup tool, exactly as they would any folder of files. The manual and the
  `doctor` remedy text say so.
- The one loss an external folder backup cannot cover is Alix's own destructive
  command (for example `alix reset`) landing between the user's backups. Undo
  for destructive commands is a separate, local concern, not backup, and is not
  built here; it is tracked as roadmap item `{#reset-undo}`.
- This declines backup only for the pre-1.0, plain-files, ongoing-use case. A
  post-1.0 breaking format change still owes the backup and rollback that
  ADR 0001 and ADR 0005 require of it; that obligation is unchanged.
- Disposition against the review's acceptance criteria: the `alix backup` /
  `restore`-in-CI criterion is **not applicable** under this decision, not
  satisfied by a subsystem. The kill-at-every-write-point criterion is met **on
  unix** by the fault suite; the cross-platform (macOS/Windows) fault tests and
  any format-migration criteria remain Phase-1 open and are not claimed here.

## Alternatives considered

### Build `alix backup` / `alix restore` with a portable archive

Rejected. Because every Alix artifact is a plain file in one folder, a complete
backup is already a copy of that folder; an archive command would freeze a
bespoke container format, path grammar, manifest schema, timestamp encoding,
retention rule, and checksum choice forever to reproduce, less well, what `tar`,
`git`, and cloud drives already do losslessly. It adds concepts a user must
learn and a permanent format to maintain, and it duplicates better external
tools. This is the kind of scope the project's fit gate and the review's own
"do not build subsystems that duplicate better external tools" both exclude.

### Rolling per-document backups before overwrite

Rejected for the same reason at smaller scale, plus it doubles write I/O on the
per-mutation flush path (ADR 0005) and still freezes a directory name and
retention default. It protects only against Alix corrupting a document, which
the crash-safe write path and fault suite already prevent.

### Undo for destructive commands

Deferred, not rejected. `alix reset` and similar are the only losses a folder
backup cannot always cover. A local, bounded undo for those specific commands is
a coherent future feature, distinct from general backup. It is not built here
and has no committed design.

## Security

A backup that contained `progress/` would be private user state (ADR 0022). By
declining to produce backups, Alix creates no new artifact that could carry
private history into a share. The user's own backups inherit their own tool's
security.

- `src/fsio.rs` unit tests: atomic replacement leaves the target readable as
  either the old or the new content after a fault at any write step
  (mutation-checked: a direct-to-target write fails it); a read-only directory
  fails the save and keeps the original; durable directory creation builds the
  missing chain.
- `src/store.rs` and `src/augment.rs`: a truncated document is rejected as a
  format error, never a panic, and a fresh save recovers.
- `tests/cli.rs`: the server drains and exits cleanly on SIGTERM (ADR 0005).
- `alix doctor` reports unreadable documents, orphans, and conflict copies and
  advises restoring from the user's own folder backup.
- **Not tested** (correct-by-construction only): that a fsync'd write survives a
  real power loss (a unit test cannot drop the page cache), and any durability
  on Windows (the directory flush is a unix-only no-op there). These are
  Phase-1 cross-platform work, not covered by this suite.

## Reversal

Revisit only if Alix ever stores state somewhere a folder copy cannot capture
coherently (an embedded database, a lock-held binary store, an online
coordinator). At that point the datastore, not the folder, becomes the backup
unit, and a backup/restore contract with a versioned archive format would be
required. While state remains plain per-deck files, the folder is the backup
unit and this decision stands.
