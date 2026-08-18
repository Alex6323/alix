# 0006: Lean Rust core

- Status: Accepted.
  [ADR 0028](0028-capability-gated-builds-and-embedded-wormhole.md) is the
  accepted successor design but is not yet implemented; this record stays
  binding until it ships.
- Evidence: cargo build --no-default-features --lib in Makefile
- Recorded: 2026-07-24
- Retrospective: Yes

## Decision history

Commits `0200893` and `5ba54e5` separated AI-facing code on 2026-07-13.
Commit `3a40c82` introduced the default-on `full` feature so the library could
compile without AI and server capabilities. Commit `e6b75e4` added the lean
build to CI, and `6b6e7ce` removed `clap` from the core dependency closure on
2026-07-14.

## Context

Desktop Alix includes a web server, AI providers, sharing, and CLI machinery.
Mobile review needs parser, scheduler, session, store, and presentation
projection without carrying desktop-only capabilities and their dependency
closure.

Reimplementing the domain in Dart would create two products whose scheduling,
file parsing, identity, and review behavior could drift. Splitting every
capability into a separate crate or service would add boundaries before the
codebase has measured a need for them.

## Decision

The Rust library is the single source of parser, identity, scheduler, session,
store, and review behavior.

The crate's default feature set includes `full`, which enables desktop-only
capabilities and dependencies. Building with `--no-default-features` produces
the lean core embedded by mobile. A module belongs in the lean core when it is
required by offline domain behavior and its complete dependency closure is
mobile-appropriate. AI provider execution, the HTTP server, sharing transport,
and CLI-only orchestration remain behind `full`.

The lean partition is a supported build target. CI and `make build-core`
compile it continuously so feature-gate drift cannot accumulate unnoticed.

## Consequences

- Desktop and mobile execute the same learning and file semantics.
- Core modules cannot casually depend on server, CLI, subprocess, or
  desktop-only libraries.
- Some types must live below feature-gated orchestration even when desktop is
  their first consumer.
- Feature boundaries add conditional-compilation maintenance.
- The default desktop build remains convenient and feature-complete.

## Alternatives considered

### Reimplement domain behavior in Dart

This would make mobile independently responsible for parsing, scheduling,
choice construction, grading transitions, and compatibility. Tests in one
implementation would not protect the other.

### Keep mobile dependent on a running desktop server

A permanent thin client would avoid cross-compiling Rust, but it would remove
standalone offline review and make availability depend on another device.

### Split the project into services or many crates

More packages can enforce boundaries, but they also add versioning and
integration overhead. The current feature and module boundaries provide the
needed partition without speculative distribution.

### Ship the complete desktop dependency closure on mobile

Code that merely compiles for a target may still be unusable, oversized, or
inappropriate there. It would also weaken the architectural signal about what
offline review actually requires.

## Compatibility

The feature names are build interfaces for in-repository consumers. More
importantly, both partitions must preserve the same deck and progress formats.
Moving a domain behavior behind `full` would be a client compatibility break.

## Security

The lean build reduces mobile attack surface by excluding server listeners,
provider subprocesses, and sharing transports. It is not a sandbox; included
parsers and filesystem code still process untrusted local content.

## Verification

- `Cargo.toml` defines `default = ["full"]` and the optional dependency set.
- `src/lib.rs` makes the module partition explicit.
- `make build-core` runs `cargo build --no-default-features --lib`.
- `.github/workflows/ci.yml` keeps the lean build in CI.
- The mobile bridge crate depends on `alix` with default features disabled.

## Reversal

Change this structure when measured build, ownership, or release constraints
show that feature gates no longer enforce a coherent boundary. A replacement
must retain one implementation of domain behavior and prove both desktop and
mobile file compatibility before the lean target is removed.
