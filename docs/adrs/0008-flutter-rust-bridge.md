# 0008: Flutter with Flutter Rust Bridge

- Status: Accepted
- Evidence: flutter_rust_bridge = in mobile/alix/rust/Cargo.toml
- Recorded: 2026-07-24
- Retrospective: Yes
- Refined by:
  [ADR 0028](0028-capability-gated-builds-and-embedded-wormhole.md), which
  replaces the mobile-as-domain-only feature boundary while preserving the
  Flutter Rust Bridge decision.

## Decision history

Commit `fcb4b89` added the Flutter mobile application and Flutter Rust Bridge
walking skeleton on 2026-07-13. Commit `3a36d88` connected the full review
contract. Commit `ca2e749` added toolchain-alignment checks, `088216d` made
bridge host tests blocking in CI, and `a9fe1a7` added a scheduled end-to-end
mobile drift build.

The decision selected Flutter Rust Bridge over UniFFI because the former is
Flutter-specialized and has a mature Dart path. UniFFI's Dart support was a
less mature third-party path, while its primary fit was shared Kotlin and
Swift bindings.

## Context

Alix needs a standalone mobile UI without reimplementing its Rust domain or
requiring a reachable desktop. The bridge must carry rich presentation-neutral
state, retain mutable sessions in Rust, and support Android now without
blocking a future iOS target.

Binding generation also creates a coupled toolchain: Rust bridge macros,
codegen, Dart runtime, Flutter, Gradle, and native target tooling must agree.

## Decision

Flutter is the mobile UI toolkit. Flutter Rust Bridge is the binding layer
between Dart and the embedded Rust library. ADR 0028 defines the current
mobile capability composition.

Long-lived review and walk sessions remain opaque Rust objects. Dart receives
generated or mirrored presentation-neutral types such as `ReviewState` and
`CardView`, invokes explicit session methods, and renders the returned state.
It does not reproduce parser, scheduler, store, or review logic.

Bridge crate, code generator, and Dart package versions are kept aligned.
Generated-code drift and host tests are blocking checks; a scheduled native
build detects ecosystem drift that normal Rust CI cannot expose.

Flutter keeps iOS as a supported future target. This record does not claim an
iOS application has shipped.

## Consequences

- Android can review local decks offline with the real Rust implementation.
- Flutter provides one mobile UI codebase for Android and anticipated iOS.
- Rust types crossing the bridge must remain codegen-compatible.
- Generated Dart and bridge glue are tracked artifacts that must be refreshed
  intentionally.
- Toolchain upgrades can fail at Rust, codegen, Flutter, Gradle, Android NDK,
  or future Xcode boundaries.
- Bridge-version exactness and drift checks are architectural maintenance, not
  incidental build cleanup.

## Alternatives considered

### Reimplement the review loop in Dart

This would violate the shared-core decision and create independent scheduling,
parsing, persistence, and grading behavior.

### Permanent LAN thin client

A server-only mobile application would fail when the desktop is absent and
would not provide standalone offline review.

### UniFFI

UniFFI is strong for generating native Kotlin and Swift bindings from one Rust
API. At the decision point, its Dart route was third-party and less mature
than the Flutter-specialized FRB path.

### Separate native Android and iOS UIs

Kotlin and Swift could use a different bridge strategy, but they would require
two UI implementations before product needs justify that cost.

## Compatibility

The bridge API and generated Dart models are internal client contracts. They
may evolve pre-1.0, but Rust API changes must regenerate code and update the
mobile client atomically. Deck and progress compatibility remains owned by the
core, not the bridge.

## Security

The bridge does not create a network boundary. Dart can invoke the filesystem
and state operations intentionally exposed by the Rust API, so the API surface
must remain narrower than the full desktop library. Remote pairing security is
recorded in ADR 0010.

## Verification

- `mobile/alix/rust/Cargo.toml` exactly pins `flutter_rust_bridge`.
- `mobile/alix/rust/src/api/review.rs` mirrors core views and owns opaque
  session wrappers.
- `scripts/frb-check.sh` and `make frb-check` verify version and generation
  alignment.
- Mobile bridge tests exercise host-callable behavior.
- `.github/workflows/mobile-drift.yml` builds through the native toolchain on a
  schedule.

## Reversal

Replace FRB when it cannot support a required Flutter or native platform,
measured maintenance exceeds a credible alternative, or the project abandons
Flutter. A migration must preserve the Rust-owned domain, port every bridge
operation, and validate offline storage and review on each supported platform.
