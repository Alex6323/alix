# 0027: Async application runtime and capability-owned state

- Status: Proposed
- Recorded: 2026-07-29
- Retrospective: No
- Refines:
  [ADR 0005](0005-progress-store-durability.md),
  [ADR 0006](0006-lean-rust-core.md),
  [ADR 0007](0007-client-neutral-review-contract.md), and
  [ADR 0010](0010-lan-pairing-and-phone-owned-state.md)

## Context

The desktop application server currently runs a fixed `tiny_http` worker pool.
Every stateful handler takes one mutex over the progress store, deck cache,
recent history, active study sessions, long-running jobs, and remote jobs. A
slow catalog traversal therefore owns unrelated study and progress state until
it finishes. More workers do not remove that serialization.

The global lock also supplies accidental correctness. Store replacement,
session transitions, job close-versus-complete races, receive collision checks,
and shutdown flushing rely on handlers not interleaving. Replacing the mutex
with an async mutex or a read-write lock would preserve those hidden
dependencies instead of naming their owners.

The application is also gaining concurrent I/O responsibilities: browser
requests, paired mobile requests, AI subprocesses, sharing, receiving, future
sync integration, timers, cancellation, and progressive picker delivery. A
manual request-worker loop would require Alix to grow its own orchestration and
shutdown framework around those responsibilities.

The learning core must remain synchronous and mobile-appropriate. Async HTTP
types, server tasks, and desktop transport dependencies do not belong in the
parser, scheduler, session, store format, or embedded mobile review core.

## Decision

### Tokio owns full-application orchestration

The full desktop/server capability uses one Tokio runtime for HTTP acceptance,
request tasks, timers, channels, cancellation, bounded blocking delegation,
and structured shutdown.

Tokio is restricted to the existing `full` feature. The lean build must not
compile Tokio, Axum, Hyper, Tower, or transport middleware. Core domain
functions remain synchronous and return ordinary typed values.

Tokio features are selected narrowly. The project does not enable Tokio's
`full` feature as a shortcut.

### Axum and Tower replace the manual HTTP server

Axum owns routing, extraction, response conversion, and typed application
state. Tower middleware supplies request tracing, endpoint-appropriate
deadlines, and other transport-wide behavior that has one clear policy.

The completed production tree has one HTTP implementation. `tiny_http`, the
fixed worker count, the `recv` loop, and the unblock shutdown relay are
removed. The old and new servers do not coexist behind a production feature
flag.

The first server remains HTTP/1.1. Loopback remains the default bind. LAN
pairing, bearer authorization, request limits, static assets, opaque image
routes, and every existing endpoint keep their established product meaning.

### Application state is split into typed capabilities

The router receives a cheap-to-clone application handle composed from these
capabilities:

| Capability | Owns |
| --- | --- |
| Configuration | immutable bindings, audience, pairing data, review settings, backend settings, and static assets |
| Catalog | deck-root inputs, catalog caches, launcher-image index, generation state, in-flight builds, and the last complete snapshot |
| Study | active review, browse, walk, exam, and tutor-origin identity |
| Progress | the active progress store, save status, ordered mutations, store replacement, and published read views |
| Jobs | augment, generate, tutor, share, receive, and remote-job lifecycle |

Handlers extract only the handles they need. There is no generic handle that
exposes every capability and no `Arc<Mutex<ServeState>>` equivalent.

Study and Progress are separate typed capabilities but share one physical
owner thread. Existing core session transitions synchronously update both a
`Session` and a `Store`; keeping them on one owner preserves that atomic
operation without holding a lock across a save or teaching the core about
async. The two handles expose explicit commands rather than closures or raw
mutable access.

The Study/Progress owner:

- serializes accepted study and progress commands;
- saves progress before acknowledging a mutation;
- flushes before replacing the active store;
- publishes read-only progress and study snapshots;
- rejects commands for an obsolete study generation; and
- handles its shutdown command, final flush, and thread join.

Async handlers send typed commands and await Tokio one-shot replies. The owner
may block on its own filesystem work because it is not a runtime worker.

Jobs remain separately owned. A job completion that changes study, progress,
or catalog state submits an explicit command to that owner. It does not retain
raw references into another capability.

### Replaceable state carries generations

Catalog builds and study contexts carry monotonically increasing generations.
A completion may publish or mutate only when its generation is still current.

Changing the catalog root, recent inputs, visible progress, generated content,
received content, or a user-requested refresh invalidates the catalog
generation. Selecting, deselecting, replacing, or closing a study context
advances its generation.

The server must reject an effect that originated from an obsolete context.
Where a server-side captured generation cannot distinguish an old client
action from a current one, the wire contract carries an opaque context token.
Any such DTO or request change is documented and contract-tested before the
affected route is migrated.

### Catalogs publish complete snapshots and progressive observations

`GET /api/decks` remains the complete snapshot contract. After invalidation it
waits for the current generation rather than returning a successful partial or
stale catalog without an approved staleness field.

The Catalog capability plans stable top-level slots before expensive member
parsing. The plan assigns each row a section-relative position and opaque key.
Bounded blocking row builders may finish out of order, but completed rows fill
their planned positions.

The adult WebUI may observe the same build through
`GET /api/decks/stream`. The finite response is authenticated streaming fetch
with newline-delimited JSON. It emits a plan, complete rows, failures, and one
final complete `DeckListDto`. The final catalog is semantically identical to
the ordinary snapshot for that generation.

Snapshot and streaming clients subscribe to one generation build. Joining
does not start another traversal. Disconnecting one stream removes that
subscriber but does not corrupt or cancel work still needed by other
subscribers. Late completions from an invalidated generation are discarded.

The adult picker keeps its previous complete snapshot during refresh,
reconciles rows by opaque key, and does not show an empty-library state while a
partial generation is in flight. The kids client may continue using the
complete snapshot endpoint.

### Blocking work stays outside async critical sections

Filesystem traversal, deck parsing, progress and augmentation I/O, archives,
received-workspace landing, synchronous subprocess waits, and materially
blocking computation run on a dedicated owner thread, through bounded
`spawn_blocking`, or through an existing tracked library worker.

No mutex or read-write-lock guard crosses:

- `.await`;
- filesystem or archive I/O;
- child-process waiting;
- `spawn_blocking`; or
- a potentially blocking channel receive.

Short in-memory critical sections may use a synchronous mutex. An async mutex
requires a resource that deliberately remains owned across an await; it is not
the default state container.

### Shutdown is an owned workflow

Ctrl-C and termination trigger one shutdown sequence:

1. stop accepting requests;
2. reject new mutating commands;
3. cancel cancellable jobs;
4. let bounded accepted work settle;
5. flush accepted progress;
6. stop and join capability owners; and
7. return with no detached file mutator.

Dropping a `spawn_blocking` future is not treated as cancellation of its
closure. Every file-mutating task has a tracked owner and shutdown policy.

### Concurrency is observable but not noisy

Debug request tracing records the matched route, request identifier, total
duration, owner wait, blocking-work duration, cancellation, and
stale-generation rejection. Tokens and sensitive request bodies are excluded.

Normal application output and the picker remain calm. Performance regressions
are verified with operation counts and deterministic gates, not wall-clock CI
thresholds or a persistent user-facing progress meter.

## Consequences

- A cold catalog scan no longer owns the active study session or progress
  store.
- Browser, mobile, and future transport work can share one application
  lifecycle and cancellation model.
- Progress durability remains serialized and save-before-reply.
- Core review behavior stays synchronous and reusable by CLI and mobile.
- Catalog requests coalesce, publish complete snapshots, and can reveal
  complete rows progressively.
- State ownership becomes more explicit but introduces capability handles,
  command enums, generations, and shutdown coordination.
- Blocking work still consumes threads; async does not make filesystem scans
  intrinsically faster.
- A physical Study/Progress owner intentionally trades per-deck write
  parallelism for clear ordering and compatibility with the synchronous core.
- The full desktop dependency tree grows, while the lean mobile dependency
  tree must remain unchanged.

## Alternatives considered

### Keep `tiny_http` and decompose state

This remains the fallback if the runtime and dependency experiment does not
justify Axum. Capability ownership, ordered progress, generations, and
complete catalog snapshots would still be required.

It is not preferred because Alix would retain custom request lifecycle,
streaming, timeout, cancellation, and shutdown orchestration while adding more
concurrent network work.

### Replace the global mutex with `tokio::sync::Mutex`

Waiters would yield, but a catalog request could still own unrelated state
through blocking work. This changes waiting mechanics without changing
ownership.

### Use a read-write lock

Several apparent reads mutate caches, poll jobs, stamp presentation, or touch
the filesystem. A broader read lock would hide those effects rather than
assign them to an owner.

### Give Study and Progress separate physical owners

Core session transitions currently mutate both `Session` and `Store`
synchronously. Two physical owners would require a distributed transaction or
a large core rewrite merely to preserve one local atomic operation. Separate
typed facades over one owner keep the conceptual boundary without inventing
cross-owner commit semantics.

### Build directly on Hyper

Alix needs routing, extraction, response conversion, middleware composition,
and service testing. Rebuilding those facilities would add local
infrastructure without improving domain ownership.

### Use Actix Web

Actix Web is credible, but it adds a framework-specific actor vocabulary while
the design needs Tokio tasks, typed Axum state, ordinary channels, and a small
number of explicit owner threads.

### Use Smol or async-std for the application runtime

These runtimes remain suitable for isolated tools. They are not selected for
the application boundary because the chosen HTTP and middleware stack is
Tokio-native, and runtime adaptation would not reduce the state-decomposition
work.

### Use WebSockets or server-sent events for the catalog

WebSockets add a persistent bidirectional protocol to a finite unidirectional
response. Browser `EventSource` cannot use the existing authorization-header
path. Streaming fetch preserves authorization and cancellation with less
protocol surface.

### Return partial `DeckListDto` values

A successful partial snapshot cannot distinguish "not ready" from "does not
exist" and recreates the empty-picker failure at the API boundary.

## Compatibility

No persisted deck, augmentation, progress, or workspace format changes.

Existing endpoint paths and complete DTO meanings remain stable through the
transport replacement. `GET /api/decks/stream` is additive. Any opaque study
context token required for stale-request rejection is an explicit pre-1.0
client-contract change and must update `docs/API.md`, contract fixtures, web
assets, paired-client callers, and the changelog together.

The `full` Cargo feature gains the async server stack. The lean build remains
the supported mobile boundary and must contain none of it.

The production source contains no compatibility server, transport shim, or
dual runtime. Intermediate branch commits may stage extracted services, but
the mergeable tree reads as one final implementation.

## Security

Authorization runs before protected handlers start catalog, job, or archive
work. Pairing tokens remain absent from event payloads, URLs after bootstrap,
and debug logs.

Request bodies retain endpoint-specific limits. Streaming does not weaken ZIP
path, size, collision, or atomic-publication defenses. A dropped request
cannot leave a half-written progress document, partially landed workspace, or
unowned file mutator.

Opaque catalog keys and study generations are concurrency identities, not
authorization credentials. They do not grant access and must not be treated
as secrets.

The supported network threat model remains ADR 0010's explicitly paired
trusted LAN. This decision does not add TLS, accounts, or hostile-network
exposure.

## Verification

- A browser regression parks catalog work deterministically and proves that
  the real adult picker renders a ready row before the final catalog exists.
- Catalog tests prove stable planned order, one build for concurrent
  subscribers, bounded row concurrency, final snapshot equality, generation
  rejection, disconnect isolation, and failed-refresh retention.
- Study/Progress owner tests prove ordered simultaneous grades,
  save-before-reply, flush-before-replace, stale-context rejection, save-error
  visibility, and shutdown flushing.
- Job and receive tests prove close-versus-complete ordering and make collision
  checking plus destination landing one atomic operation.
- Raw HTTP and contract tests preserve paths, methods, statuses, headers, body
  limits, authorization, and existing JSON snapshots.
- `cargo check --no-default-features` and the lean CI job prove the runtime
  stack stays outside mobile.
- `make deps-check` and dependency inventory compare the reviewed full-feature
  tree before and after replacement.
- Debug traces prove owner wait and blocking work are observable without
  logging tokens.
- The final gates include `make check`, `make ci`, Playwright, and the sourced
  implementation review deck.

## Reversal

Replace Tokio or Axum if measured startup, binary-size, resource, portability,
or maintenance costs exceed their application value. A synchronous replacement
may reuse the capability handles and owner threads.

The state-ownership decision is independently load-bearing. Any replacement
must still prevent catalog work from owning study or progress, preserve one
ordered progress writer, reject stale generations, publish complete catalog
snapshots, maintain structured shutdown, and keep the async transport stack
out of the lean core.
