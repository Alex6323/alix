# 0028: Capability-gated builds and embedded Wormhole

- Status: Accepted (design; NOT yet implemented — the text below describes
  the target state in completed tense. Production still shells out to the
  `wormhole` CLI, there is no `wormhole` Cargo feature, and `mobile/alix-sync`
  is empty. Corrected 2026-08-02 after the docs audit read the tense as a
  shipped claim.)
- Recorded: 2026-07-29
- Retrospective: No
- Refines:
  [ADR 0007](0007-client-neutral-review-contract.md),
  [ADR 0010](0010-lan-pairing-and-phone-owned-state.md), and
  [ADR 0024](0024-deck-transfer-reuses-public-bundles.md)

## Context

ADR 0006 defined `--no-default-features` as the mobile build and placed every
sharing transport behind `full`. That boundary served offline review, but it no
longer matches the product.

This record supersedes [ADR 0006](0006-lean-rust-core.md).

The Alix mobile study application needs first-class Wormhole send and receive.
Requiring a separately installed Python `wormhole` executable cannot work on
Android or iOS and makes desktop sharing depend on external installation.
Alix therefore embeds the Rust `magic-wormhole` crate.

Embedding Wormhole does not require the parser, scheduler, session, store, or
document model to become async. The executor that drives the transfer crate is
private to the transfer boundary. It neither selects nor prejudices whether
Alix later adopts an application runtime.

The current `magic-wormhole` implementation uses the `async-io` and Smol
dependency family. That fact informs the local transfer dependency review. It
does not make that family the Alix application runtime, rule out Tokio or Axum,
or require the HTTP server to share an executor.

The useful build boundary is capability-based:

- a domain-only dependency firewall;
- a Wormhole-capable mobile and library build; and
- the complete desktop application.

The separate `apps/alix-sync` application embeds Syncthing for continuous
workspace synchronization. It is a different product and does not depend on
the Alix Rust library.

## Decision

### Preserve a domain-only dependency firewall

`cargo build --no-default-features --lib` remains a supported build, but it is
not described as the mobile product.

The domain-only build contains parser, identity, scheduler, session, store,
presentation, deck, workspace, and other shared file semantics. It excludes
network listeners, AI subprocess orchestration, command-line parsing, archive
transport, and Wormhole networking.

Its purpose is to enforce dependency direction and prove that domain behavior
does not depend on application capabilities.

### Add a dedicated `wormhole` capability

Cargo defines an explicit `wormhole` feature for embedded Magic Wormhole send
and receive.

Conceptually:

```toml
[features]
default = ["full"]
wormhole = ["dep:magic-wormhole", "dep:<reviewed-executor>"]
full = [
  "wormhole",
  "dep:tiny_http",
  "dep:zip",
  "dep:qrcodegen",
  "dep:clap",
  "dep:ctrlc",
]
```

The exact executor crate and features require the normal dependency review
before `Cargo.toml` changes. Selection is local to the transfer implementation
and weighs correctness, dependency overlap, maintenance, platform support, and
measured artifact cost.

The choice creates no application-runtime precedent. Transfer APIs expose no
executor-specific types, so a later implementation may change or unify the
private executor without changing callers.

The `wormhole` feature includes only the transfer engine, cancellation and
event model, and the share/receive bundle boundary needed by desktop and mobile.
It does not include the HTTP server, Axum, Hyper, Tower, AI providers, desktop
CLI parsing, or Syncthing.

### Make `full` compose capabilities

The default `full` desktop feature includes `wormhole` because desktop share
and receive use the embedded implementation.

The desktop web server remains part of `full` and continues using
`tiny_http` until the separate Axum investigation produces evidence and a new
decision.

The feature graph expresses product composition. It is not a tier in which
every lower feature must be appropriate for every consumer.

### Give each mobile application its actual capability set

The current study application is renamed from `apps/mobile` to `apps/alix`.
It depends on the Alix library with default features disabled and the
`wormhole` feature enabled. It receives the shared learning domain plus
Wormhole transfer, without the desktop HTTP and AI stack.

The new `apps/alix-sync` application embeds Syncthing and does not depend on
the Alix Rust library merely to reuse paths, configuration, QR rendering, or
domain types.

Wormhole is an explicit user-initiated transfer in `apps/alix`. Syncthing is
continuous file convergence in `apps/alix-sync`. Their feature and process
boundaries remain separate.

### Hide asynchronous transport behind a transfer boundary

Public parser, scheduler, session, store, deck, and workspace APIs remain
synchronous.

The embedded Wormhole implementation owns its executor and asynchronous
network state behind a transfer boundary that exposes:

- start send or receive;
- transfer events and progress;
- cancellation;
- one terminal outcome; and
- joined shutdown.

Desktop CLI, desktop web jobs, and the mobile bridge consume that boundary
without receiving executor handles or Wormhole protocol types.

The implementation may run one transfer on a dedicated thread that drives the
private executor. It must not establish a global application runtime merely
because the underlying crate is async.

A future application-runtime decision is made independently on Alix's own
server and orchestration requirements. The transfer executor may contribute
dependency-overlap measurements to that investigation, but it is evidence
rather than precedent. Conversely, adopting an application runtime does not
force transfer onto it unless a separate implementation review shows that
doing so preserves this boundary and improves the system.

### Remove subprocess ownership from production transfer

The completed production tree does not shell out to the Python
`magic-wormhole` CLI.

There is one send implementation and one receive implementation. Desktop and
mobile reuse the same Rust transfer boundary. A missing external executable is
no longer a production transfer state.

Existing public-bundle staging and receive landing remain the Alix-owned file
boundary. Wormhole transports the prepared bytes and does not decide which
workspace files are public.

### Pin and review the dependency and license boundary

Alix pins a reviewed `magic-wormhole` release and selects only the required
features. `make deps-check` runs before and after dependency changes.

The dependency review records:

- direct features;
- the async executor and I/O families;
- duplicate dependency families;
- desktop binary-size change;
- Android Rust-library and APK size changes; and
- the license and notice obligations of Magic Wormhole and its dependencies.

Magic Wormhole's EUPL-1.2 license and source-availability obligations are
handled in the release and notice workflow before distribution.

## Consequences

- Desktop sharing no longer requires Python or an external `wormhole`
  executable.
- The Alix study application can send and receive directly on mobile.
- Domain-only builds remain a strong dependency-direction check.
- Mobile carries the Wormhole networking dependency closure by deliberate
  capability choice.
- The desktop `full` build composes Wormhole with server, CLI, AI, and archive
  capabilities.
- Async implementation details remain confined to transfer code.
- The repository maintains two distinct mobile applications with different
  responsibilities.
- Embedded networking increases supply-chain, binary-size, licensing, and
  mobile lifecycle work.

## Alternatives considered

### Put Wormhole directly in every build

This would weaken the domain dependency firewall and make parser or scheduler
consumers compile an unrelated networking stack.

### Keep Wormhole behind `full`

This preserves the old feature graph but prevents the study application from
shipping native mobile transfer without also gaining desktop-only
dependencies.

### Keep shelling out on desktop and embed only on mobile

Two transfer implementations would drift in events, cancellation, errors, and
bundle handling. Desktop would retain an installation dependency that the Rust
crate removes.

### Select an application runtime in this ADR

This ADR decides embedded transfer, not application orchestration. Tokio,
Smol, no application runtime, and any HTTP-stack change remain in scope for the
separate server-runtime investigation. That investigation weighs Alix's own
middleware, timeout, cancellation, maintenance, documentation, portability,
and measured-cost requirements. The private transfer executor is one
dependency-overlap input and carries no architectural vote.

### Reimplement Magic Wormhole

The protocol, cryptography, rendezvous behavior, and transit negotiation are
correctness-critical commodity functionality. Reimplementation would create
more security and interoperability risk than maintaining the dependency.

### Merge Wormhole transfer into `apps/alix-sync`

`apps/alix-sync` owns continuous Syncthing convergence. Requiring it for an
explicit one-time share would couple independent products and make transfer
availability depend on the companion application.

## Compatibility

Deck, workspace, progress, augmentation, and public-bundle formats do not
change.

The feature graph is an in-repository build contract. `apps/alix` changes from
domain-only to domain-plus-Wormhole. The completed tree contains no fallback
subprocess path.

Public transfer commands and API DTOs retain their product meaning. Observable
event or error changes require the normal API, client, documentation, and
changelog updates.

## Security

The embedded crate processes network input in the Alix process. Version
pinning, dependency review, cancellation tests, and release provenance become
part of the application threat surface.

Pairing codes and protocol secrets are excluded from logs. Received bytes still
pass through the existing bounded staging, archive, path, collision, and atomic
landing rules before publication.

The feature boundary does not grant `apps/alix-sync` access to Alix AI or
learning internals, and does not grant `apps/alix` continuous synchronization
authority.

## Verification

- `cargo build --no-default-features --lib` proves the domain-only dependency
  firewall.
- `cargo build --no-default-features --features wormhole --lib` proves the
  mobile transfer composition.
- The `full` build proves the desktop application composes the same transfer
  implementation.
- Dependency inventories prove HTTP and AI families remain absent from the
  Wormhole-only build.
- Desktop and mobile transfer tests exchange the same staged public bundle and
  land it through the same receive workflow.
- Cancellation and shutdown tests prove no transfer thread or file writer is
  detached.
- Mobile evidence records Android library and APK size before and after the
  capability.
- Source and release audits verify license notices and pinned provenance.
- Production searches prove the external `wormhole` subprocess path is gone.

## Reversal

Replace Magic Wormhole when security, maintenance, platform, size, or
interoperability evidence shows that the embedded crate is unsuitable.

A replacement must preserve one desktop/mobile transfer boundary, explicit
user initiation, bounded receive landing, cancellation, and the domain-only
dependency firewall.
