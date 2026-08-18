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

**First product scope is desktop and Android; iOS is a later arc with its own
gate.** Amended 2026-08-18: the feasibility work ran on Linux and closed every
proof except iOS, where compilation and the transfer lifecycle are unproved and
unprovable without macOS. No iOS client exists yet; one is planned once the
deck format is settled and the Android app reaches feature parity, where the
sharing bar is server-backed sharing while paired (restated 2026-08-18 with
the first-scope ruling below; it was first stated as on-device Wormhole) plus
Syncthing syncing. The first task of that iOS arc, before
any capability work builds on it, is the evidence this record cannot supply
today: a release-mode iOS build linking the Wormhole-only library, then a
lifecycle proof that suspension or cancellation leaves no detached transfer
task, no detached writer, and no partial file published. iOS suspension is the
structural risk: unlike Android's foreground service, iOS offers no mechanism
to keep an arbitrary socket transfer alive in the background.

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
- a Wormhole-capable library build, enabled per application when an
  application needs it; and
- the complete desktop application.

**First-scope mobile sharing is server-backed, ruled 2026-08-18.** The phone
shares and receives through the paired desktop server's JSON API, which runs
the one transfer implementation on the desktop; the phone is interface, not
transport. `mobile/alix` therefore stays domain-only and carries no Wormhole
closure, no zip, and no EUPL code. There is no evidence yet that anyone shares
where no desktop is running, so the on-device transfer arc is deferred until
that evidence exists; the capability flag makes enabling it a build change,
not an architectural one. Deferring it also removes the iOS transfer-lifecycle
risk from every planned build, since no phone drives a background socket. The
build cost this ruling adds: share and receive become job-shaped endpoints on
the server (the existing background-ask pattern), reachable by any paired
client.

The separate `mobile/alix-sync` application embeds Syncthing for continuous
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
wormhole = ["dep:magic-wormhole", "dep:<reviewed-executor>", "dep:zip"]
full = [
  "wormhole",
  "dep:tiny_http",
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

The study application lives at `mobile/alix` (renamed from `apps/mobile`
before this record was implemented; the rename is baseline, not work this
record claims). **In the first scope it depends on the Alix library with
default features disabled and no `wormhole` feature**: sharing reaches it
through the paired server (see Context, ruled 2026-08-18). The deferred
on-device arc enables the `wormhole` feature on this same dependency line,
receiving the shared learning domain plus Wormhole transfer without the
desktop HTTP and AI stack; nothing else about the application layout changes.

The new `mobile/alix-sync` application embeds Syncthing and does not depend on
the Alix Rust library merely to reuse paths, configuration, QR rendering, or
domain types.

Wormhole is an explicit user-initiated transfer in `mobile/alix`. Syncthing is
continuous file convergence in `mobile/alix-sync`. Their feature and process
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
boundary. Wormhole transports the staged file-or-directory payload using the
private framing below and does not decide which workspace files are public.

### The payload framing is the existing zip container

**Amended 2026-08-18 after an adversarial re-validation found this decision
missing.** Ordinary Alix bundles are directories: a shared workspace always, and
any initialized deck with augmentation or owned assets (`share::stage_path`).
The `magic-wormhole` crate sends a folder as a tarball the receiver must
unpack by hand, and the feasibility prototype proved regular-file transport
only. Without a ruled framing, a builder either rejects the ordinary share or
delivers residue Alix does not land.

Ruled: **a staged directory travels as the same zip container the `--zip`
share path already produces** (`share::zip_to`). The sender frames a staged
directory before offering it; a staged single file is offered as-is. The
receiver detects and strips the framing (`share::unzip_to`, whose extraction
already defends against hostile paths) **before** the existing sanitization,
collision, and atomic landing boundary, so landing sees exactly what it sees
today and no user-visible archive residue survives a successful receive.

This is a private framing between Alix instances, not a public format: both
ends are Alix, the container is internal to the transfer, and changing it
later is an ordinary pre-1.0 change. It reuses the reviewed `zip` dependency
and the existing land-a-zip receive path rather than adding a `tar` dependency
to do a job an existing one already does; the `zip` crate therefore moves from
`full` into the `wormhole` capability, which is the one dependency-placement
change this amendment makes.

Receiving is a network trust boundary: the receiver enforces explicit bounds
on entry count, per-entry expanded size, and total expanded size before
extraction, and rejects an over-limit payload without landing anything. The
concrete limits are implementation values documented with the security
regression evidence, not frozen here.

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

The focused engineering review ran 2026-08-18 (working notes in the local
`docs/research/`; this record carries every load-bearing conclusion, since the
notes are not tracked). Findings: the full dependency tree contains exactly one
copyleft component, the crate itself; the European Commission's published FAQ
states that static and dynamic linking do not make the linking application a
derivative work; and the `EUPL-1.2` versus or-later metadata discrepancy
dissolved, because or-later is the licence text's own default and SPDX cannot
express the distinction.

**The first distributed binary containing the crate is gated on:**

- a `NOTICE` entry (crate, copyright holder, EUPL-1.2) and the EUPL-1.2 text
  shipped with distributed artifacts;
- a corresponding-source pointer (exact version, crates.io URL, upstream tag)
  in the release notes;
- the license inventory re-run on the release lockfile, still showing exactly
  one copyleft component, else this review reopens;
- counsel sign-off that an unmodified statically linked EUPL crate does not
  make the distributing binary a derivative work, and on the or-later reading.

**A store build containing the crate carries one further sign-off:** store
terms against the licence's no-additional-restrictions clause. Precedent
exists (an EUPL-licensed national application shipped through the Play Store
with the tension acknowledged), and the capability flag remains the escape: a
store build can ship without `wormhole` entirely. **Under the first-scope
ruling no planned build triggers this sign-off**: mobile stays domain-only and
the crate ships only in direct-distributed desktop binaries. It becomes due
with the deferred on-device arc, and not before.

## Consequences

- Desktop sharing no longer requires Python or an external `wormhole`
  executable.
- The Alix study application shares and receives through the paired server in
  the first scope; it sends and receives on-device only in the deferred arc.
- Domain-only builds remain a strong dependency-direction check.
- Mobile carries no Wormhole networking closure in the first scope; the
  deferred arc adds it by deliberate capability choice.
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

**Sized and kept as the named licensing fallback, 2026-08-18.** The rejection
above stands, and one fact could overturn it: the EUPL review concluding that
shipping the crate is untenable for a distributed build. The permissive route
exists: `spake2` (the protocol's own PAKE primitive, `MIT OR Apache-2.0`,
RustCrypto) plus permissive crates for every other primitive, with alix
implementing the protocol layers above them, which are the rendezvous mailbox
state machine over WebSocket, HKDF phase-key derivation and secretbox message
encryption, the transfer offer protocol, and transit with hint gathering,
connection races, relay fallback, and Noise-framed records. Reimplementing from
the published protocol documents is legally clean, third-party clients exist
(`wormhole-william`, Go, MIT), and the public rendezvous and relay
infrastructure serves them. The cost is multiple focused days against the
Python peer as interop oracle and permanent ownership of a security-sensitive
protocol client, where the primitives come from crates but the derivation
strings, handshakes, and record framing are hand-rolled glue whose mistakes can
be silent. The cheaper per-platform escape stays available first: a build can
ship without the `wormhole` capability at all.

### Merge Wormhole transfer into `mobile/alix-sync`

`mobile/alix-sync` owns continuous Syncthing convergence. Requiring it for an
explicit one-time share would couple independent products and make transfer
availability depend on the companion application.

## Compatibility

Deck, workspace, progress, augmentation, and public-bundle formats do not
change.

The feature graph is an in-repository build contract. `mobile/alix` stays
domain-only in the first scope and changes to domain-plus-Wormhole in the
deferred on-device arc. The completed tree contains no fallback subprocess
path.

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

The feature boundary does not grant `mobile/alix-sync` access to Alix AI or
learning internals, and does not grant `mobile/alix` continuous synchronization
authority.

## Verification

- `cargo build --no-default-features --lib` proves the domain-only dependency
  firewall.
- `cargo build --no-default-features --features wormhole --lib` proves the
  Wormhole-only library composition (no application enables it on mobile in
  the first scope; the build proves the boundary, not a shipped product).
- The `full` build proves the desktop application composes the same transfer
  implementation.
- Dependency inventories prove HTTP and AI families remain absent from the
  Wormhole-only build.
- Desktop transfer tests exchange the same staged public bundle and land it
  through the same receive workflow, and a paired client drives a share and a
  receive end to end through the server's job-shaped endpoints.
- Landed-bundle interoperability runs in both directions against the Python
  `wormhole` CLI for the two directory cases: a workspace, and an initialized
  deck carrying an asset and augmentation. Each run feeds the received object
  through the existing landing path and asserts identical public files, no
  progress or personal state, and no user-visible archive residue.
- A framed receive over the entry-count or size bounds is rejected without
  landing anything.
- Cancellation and shutdown tests prove no transfer thread or file writer is
  detached.
- Android size evidence moves to the deferred on-device arc with the
  capability itself. iOS evidence is deliberately absent: first product scope
  is desktop and Android, and the iOS compile/link and lifecycle gate is the
  first task of the later iOS arc (see Context).
- Source and release audits verify license notices and pinned provenance.
- Production searches prove the external `wormhole` subprocess path is gone.

## Reversal

Replace Magic Wormhole when security, maintenance, platform, size, or
interoperability evidence shows that the embedded crate is unsuitable.

A replacement must preserve one desktop/mobile transfer boundary, explicit
user initiation, bounded receive landing, cancellation, and the domain-only
dependency firewall.
