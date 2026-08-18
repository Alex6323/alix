# 0027: Capability-owned application state

- Status: Accepted
- Evidence: pub(super) struct CatalogHandle in src/serve/catalog_owner.rs
- Recorded: 2026-07-29
- Retrospective: No
- Refines:
  [ADR 0005](0005-progress-store-durability.md),
  [ADR 0007](0007-client-neutral-review-contract.md), and
  [ADR 0010](0010-lan-pairing-and-phone-owned-state.md)

## Decision history

This record reuses number 0027 after withdrawing the publicly pushed Proposed
record "Async application runtime and state ownership." That proposal was
never Accepted. Its state-ownership question moved here, while its runtime and
transport assumptions were removed or separated into later decisions.

## Context

The desktop application server uses a fixed `tiny_http` worker pool. Static
routes are handled independently, but every stateful route then takes one
mutex over the progress store, recent history, deck cache, active study
sessions, job registries, and remote jobs.

That lock is broader than the operations it protects. A catalog build holds it
while traversing and parsing files. A progress mutation holds it while saving
the progress document. `GET /api/doctor` holds it while synchronously spawning
backend and Wormhole version checks. These operations prevent unrelated
stateful requests from progressing even though the HTTP server has several
workers.

The lock also provides accidental correctness. Store replacement, tutor
alignment, job close-versus-complete behavior, receive collision checks,
presentation-stamp flushing, and shutdown all rely on handler-wide exclusion.
Replacing the mutex with several informal locks would make those dependencies
harder to see without assigning each mutation to one owner.

Two earlier blank-picker incidents do not prove that async HTTP is required.
The keep-alive reader problem and quadratic catalog traversal were independently
root-caused and fixed. The current tree has two separate live bugs: a root
`read_dir` failure becomes a successful empty catalog, and the adult picker
clears its stage before awaiting a catalog request without a retryable error
state. Those bugs must be fixed directly. They are not evidence for changing
the HTTP stack.

The state problem remains independently load-bearing. Browser and paired
mobile requests can act on the same application state, and long filesystem or
subprocess work should not own unrelated review progress.

## Decision

### Decompose state without changing HTTP transport

Application state is split behind typed capabilities while the production
server remains `tiny_http`.

This decision does not select Tokio, Axum, Hyper, Tower, streaming catalog
delivery, or another HTTP implementation. A later transport investigation may
reuse these ownership boundaries, but it cannot be used to justify delaying
them.

The synchronous parser, scheduler, session, store, and document workflows
remain synchronous.

### Use capability handles instead of a global state handle

The server entry point composes these capabilities:

| Capability | Owns |
| --- | --- |
| Configuration | immutable bind, audience, pairing, review, backend, and asset inputs |
| Catalog | root inputs, deck cache, resolution snapshot, launcher images, complete catalog snapshots, and invalidation |
| Study | active review, browse, walk, exam, tutor transcript, and current study identity |
| Progress | active and non-active progress documents, save state, ordered mutations, replacement, and read projections |
| Jobs | augment, generate, tutor worker, share, receive, and remote-job lifecycle |

Study and Progress are separate typed handles over one physical owner thread.
Current session transitions synchronously mutate both `Session` and `Store`.
One owner preserves that transaction boundary without teaching the domain core
about threads or async.

Handlers receive only the capabilities they need. There is no generic escape
hatch that exposes every sender, lock, or mutable owner field.

### Resolve names independently from complete catalog construction

Catalog owns an immutable resolution snapshot that maps the accepted
client-facing deck and workspace names to validated targets.

Resolution is a cheap discovery product, not a `DeckListDto` side effect. A
select, browse, reset, exam, share, receive, import, augment, drawer, deadline,
or remote request must not wait for every catalog row to calculate status
before its target can be identified.

Catalog refreshes the resolution snapshot before publishing a complete catalog
snapshot. Selecting a deck may change recent ordering and therefore invalidate
catalog presentation, but it does not invalidate the mapping from that deck's
name to its target.

The implementation plan derives the complete name-taking endpoint inventory
from the route match arms in `src/serve/mod.rs`. No endpoint performs an
independent full catalog traversal to resolve one name.

### Preserve external-editor visibility

Explicit invalidation follows every successful Alix write that can change
catalog presentation or resolution. The implementation inventory includes
progress mutations, recent history, deck rewrites, tutor note and card writes,
augmentation changes, import, generation, receive landing, workspace
deadlines, reset, and root changes.

Explicit triggers are not sufficient because authored files may change in an
editor or synchronization tool. Every catalog fetch and name resolution runs a
cheap metadata revalidation over the known discovery inputs. Changed metadata
invalidates the affected resolution or catalog entry before the request
returns.

The existing picker refresh action remains a normal `GET /api/decks` refetch.
This decision does not invent a refresh endpoint that is absent from the
current API.

### Publish complete catalog snapshots

Catalog readers receive one complete `DeckListDto` or an explicit error. A
root-read failure is not a successful empty catalog.

Equivalent concurrent list requests may share one in-flight complete build.
The first implementation does not add progressive rows, skeleton slots,
opaque presentation keys, NDJSON, server-sent events, or WebSockets.

Catalog consumes immutable progress projections. It does not borrow a writable
active store or open unrelated mutable progress documents while building rows.

### Give all progress documents one mutation owner

The Progress capability owns active and non-active progress document I/O.
Reset, deck-drawer operations, catalog progress projections, and active review
mutations do not open competing writable `Store` values in handlers.

The owner:

- serializes accepted mutations;
- saves before acknowledging a mutation;
- preserves the once-only presentation stamp;
- retains the current save error until a later successful save;
- flushes before replacing the active document;
- rejects replacement when the flush still fails;
- publishes immutable read projections; and
- performs the final shutdown flush before its thread joins.

When replacement is rejected, the current study and progress state remains
active. After correcting the filesystem problem, repeating the same selection
or deselection request retries the flush. There is no force-discard path.

### Keep tutor identity with Study and tutor execution with Jobs

The tutor worker belongs to Jobs because it is long-running and cancellable.
The transcript, originating card, and permitted effects belong to Study.

Starting tutor work captures the current study identity and card identity.
Job completion submits a typed effect to Study. Study appends transcript,
notes, or draft results only if that identity is still current. Leaving tutor,
advancing the card, selecting another deck, or closing the study context makes
the older completion ineligible.

Jobs never mutates a session or progress document directly.

### Classify retry behavior before migrating mutations

Removing global request serialization changes the consequences of a dropped
HTTP reply. A client disconnect does not cancel an accepted owner command.

Before a mutating route moves to an owner, its phase brief classifies it as:

- card- or session-relative, requiring current study identity;
- naturally idempotent;
- single-flight under an existing job identity; or
- non-idempotent, requiring an approved operation identity.

A card-relative mutation accepted once must become stale before a retry can
grade or alter a later card. If the current server-side information cannot
prove that property, the implementation stops and presents the exact client
contract proposal to the maintainer before changing DTOs or requests.

### Treat owner failure as application failure

This policy is a deliberate availability change from today's poison-tolerant
global mutex. The maintainer ratified coordinated shutdown as the owner-death
contract when accepting this ADR.

An owner-thread panic or unexpected mailbox closure does not trigger an
automatic owner restart with uncertain in-memory state. The supervisor rejects
new mutations, initiates the normal shutdown workflow, reports the failed
owner, and lets the process be restarted from durable files.

An owner failure must not leave the server running indefinitely with a
permanent partial 503 state. A client-visible service-unavailable response is
permitted only while coordinated shutdown is already in progress.

### Keep blocking work outside unrelated ownership

Filesystem traversal, parsing, progress and augmentation I/O, archive work,
received-content landing, and subprocess waiting execute on the capability
that owns the workflow or an existing tracked library worker.

No capability lock or borrowed mutable state crosses filesystem I/O, child
process waiting, archive work, or a blocking channel receive. Owner threads may
perform their own bounded synchronous work because they do not block unrelated
owners.

### Make shutdown an owned workflow

Shutdown:

1. stops accepting new requests;
2. rejects new mutating commands;
3. cancels cancellable jobs;
4. settles accepted atomic operations;
5. flushes accepted progress;
6. stops each owner; and
7. joins every owner before the server returns.

No file-mutating worker may outlive the application server.

## Consequences

- Catalog, doctor, and job work no longer own active study progress.
- Existing synchronous domain behavior remains reusable by CLI and mobile.
- Name resolution no longer requires a complete catalog row build.
- External file edits remain visible on the next catalog or resolution request.
- Progress durability and study ordering become explicit owner contracts.
- The implementation gains typed commands, replies, read projections, and
  shutdown coordination.
- One Progress owner intentionally prioritizes clear local ordering over
  parallel writes to several progress documents.
- The HTTP server and public API can remain unchanged while ownership changes.

## Alternatives considered

### Keep one application actor

A single actor would hide the mutex but preserve global serialization. It does
not let catalog or doctor work progress independently from Study and Progress.

### Split `ServeState` into several shared locks

This is smaller mechanically, but handlers could acquire several locks in
different orders and recreate cross-domain coupling. Typed commands make the
transaction owner explicit.

### Use a read-write lock

Several read-shaped routes mutate caches, poll jobs, stamp presentation, open
files, or spawn processes. A read-write lock would misclassify effects without
changing ownership.

### Change to Axum while decomposing state

Changing ownership, transport, runtime, middleware, and tests together makes
failures harder to attribute. Axum remains a later investigation after this
decision is implemented and measured.

### Add progressive catalog delivery

No current measurement establishes a user-perceptible need for partial catalog
rows. Complete snapshots preserve the current API with less state and UI
surface. Progressive delivery requires its own later spec if measurements
justify it.

## Compatibility

No persisted deck, progress, augmentation, or workspace format changes.

Existing endpoint paths and DTOs remain stable unless the maintainer separately
approves a concrete study-identity or operation-identity proposal. Such a
proposal must update `docs/API.md`, contract fixtures, web and mobile clients,
and the changelog together.

The mergeable production tree contains only the new ownership model. It does
not retain a global-state fallback or dual command path.

## Security

Authorization still runs before protected work. Capability handles do not
weaken the existing bind, pairing, body-limit, archive, path, or static-asset
rules.

Study and operation identities are concurrency guards, not authorization
credentials. They grant no access and are not logged.

Separating receive landing and non-active progress writes behind named owners
prevents check-then-act races from bypassing collision or durability rules.

## Verification

- A route inventory generated from `src/serve/mod.rs` assigns every stateful
  endpoint to named capabilities.
- Resolution tests prove a select can resolve during a parked complete catalog
  build and that recent-history updates do not invalidate target identity.
- External-edit tests modify, add, and remove files outside Alix, then prove
  the next fetch or resolution sees the change.
- Progress-owner tests cover simultaneous grades, presentation stamping,
  save-before-reply, replacement rejection and retry, non-active resets,
  save-error projection, shutdown flush, and thread join.
- Tutor tests cover completion after card advance, tutor exit, and deck
  replacement.
- Panic and dropped-reply tests prove the approved shutdown and retry
  semantics.
- Catalog tests prove complete snapshots, explicit root-read errors,
  equivalent-request coalescing, and immutable progress projections.
- Existing API contract fixtures remain unchanged unless a separately
  approved identity change lands.

## Reversal

Replace owner threads or capability transport when measurements show a simpler
mechanism with the same ownership guarantees.

Any replacement must still prevent unrelated catalog, doctor, job, and
progress work from sharing one critical section; preserve one ordered progress
writer; keep external edits visible; reject stale study effects; and complete
owned shutdown.
