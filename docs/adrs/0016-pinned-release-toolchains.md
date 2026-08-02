# 0016: Pinned release toolchains with separate drift probes

- Status: Accepted
- Recorded: 2026-07-25
- Retrospective: No

## Context

Alix builds a Rust CLI, a Flutter application with an NDK-built Rust library,
and a generated website. A release therefore executes code from several
toolchains and GitHub Actions before it produces an artifact users trust.

Using `stable`, `nightly`, movable Action tags, or a dependency's `main` branch
lets the same Alix commit build with different code on different days. That
drift is useful as an early compatibility signal, but it must not occur inside
the production CI or release path.

The repository already has scheduled backend and mobile drift workflows.
Reproducible release inputs and early warning can therefore be separate jobs
rather than conflicting goals.

## Decision

Blocking CI and release workflows use exact toolchain selections:

- `rust-toolchain.toml` pins the default Rust release.
- `.rust-nightly-version` pins the date-named nightly used for formatting and
  coverage.
- `mobile/alix/.fvmrc` pins Flutter.
- the Android application pins its NDK in `build.gradle.kts`.
- workflow inputs pin Java, Node, mdBook, `cargo-llvm-cov`, and FRB codegen.
- release Cargo builds and codegen installation use locked dependency graphs.

Every external GitHub Action reference uses a full commit SHA. A weekly grouped
Dependabot update proposes new Action commits; the maintainer reviews those
changes like executable dependency updates.

`make toolchain-check` is a deterministic blocking gate. It rejects movable
Action references, floating production Rust selections, missing exact version
files, a mutable cargo-binstall Action, and drift between coupled workflow
pins.

Scheduled drift workflows are the explicit exception for tool versions, not
Action code:

- backend drift follows current stable Rust and the current Node 22 line;
- mobile drift follows current stable Rust, Flutter, Java 21, and Flutter's
  current NDK in a disposable checkout;
- FRB remains at the project's exact version so the job varies the platform
  toolchain rather than two compatibility dimensions at once.

No artifact from a drift workflow is published.

## Consequences

- A release commit no longer silently changes Rust, Flutter, Java, Node, NDK,
  FRB codegen, documentation tooling, coverage tooling, or directly referenced
  Action code merely because upstream moved a tag or channel.
- Local Cargo commands automatically select the supported Rust release.
- Formatting and coverage may require installing the date-named nightly.
- Toolchain upgrades become reviewable diffs that must pass the full gates.
- Scheduled jobs still report future ecosystem breakage before an intentional
  pin update.
- Compiling FRB codegen from its locked crate graph is slower than executing a
  mutable prebuilt installer Action.
- GitHub-hosted runner images, operating-system packages, registries, and
  transitive behavior inside third-party Actions are not made hermetic by this
  decision.

## Alternatives considered

### Follow stable channels everywhere

This minimizes maintenance but makes CI and release failures depend on when a
job happens to run. A tag would not identify the toolchain that built it.

### Pin the scheduled drift jobs too

That produces quieter dashboards by removing the early warning the jobs exist
to provide. Drift jobs must encounter new upstream tools before releases do.

### Use movable major Action tags with Dependabot

Dependabot does not remove the interval during which a compromised or retagged
major reference can execute. Full SHAs make every direct Action-code change a
repository diff.

### Adopt a hermetic build system immediately

Nix, Bazel, or containerized toolchain images could control more inputs, but
they add a second build architecture before Alix has completed artifact
checksums, provenance, and cross-platform release promotion. Exact native pins
are the bounded first step.

### Duplicate all versions in one repository-specific manifest

Rust and Flutter already have native version files. Keeping those authoritative
and machine-checking the necessary workflow copies is simpler than adding a
custom version loader to every job.

## Compatibility

No deck, progress, API, or card-identity format changes. Contributors may need
to install Rust 1.97.1, nightly-2026-07-23, or Flutter 3.44.4 when entering the
checkout or running platform-specific gates.

Changing a pin is an ordinary pre-1.0 development change, but it must be
atomic across coupled files and generated outputs.

## Security

Full Action SHAs remove movable direct references from the workflow trust
boundary. Exact tool versions reduce unreviewed code changes in build and
release jobs. Locked Cargo invocations prevent release tooling from
re-resolving a different allowed dependency graph.

This is not complete supply-chain integrity. The project still needs hardened
runner images, dependency policy, SBOMs, checksums, signed provenance, artifact
promotion, and installer verification. Dependabot PRs are executable-code
updates and require review; automatic merging would defeat the purpose of the
pins.

## Verification

- `scripts/toolchain-check.sh` enforces the pin and exception policy.
- `make check` runs `make toolchain-check`.
- `.github/workflows/ci.yml`, `release.yml`, `mobile-release.yml`, and
  `pages.yml` use exact production toolchains.
- `.github/workflows/backend-drift.yml` and `mobile-drift.yml` name their
  intentional floating inputs.
- `.github/dependabot.yml` proposes weekly grouped Action updates.
- `scripts/frb-check.sh` verifies that the exact app NDK is installed and that
  Rust, Dart, and FRB codegen versions agree.

## Reversal

Replace this scheme when a hermetic cross-platform build definition can produce
and verify all desktop and mobile artifacts with less duplicated configuration.
The replacement must preserve immutable reviewed Action code, deliberate
upstream drift detection, exact release inputs, and reviewable toolchain
updates.
