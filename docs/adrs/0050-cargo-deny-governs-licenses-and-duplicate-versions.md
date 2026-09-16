# 0050: cargo-deny governs licenses and duplicate versions

- Status: Accepted
- Evidence: multiple-versions = "deny" in deny.toml
- Evidence: "Unicode-3.0" in deny.toml
- Evidence: cargo-deny --manifest-path Cargo.toml in scripts/deps-check.sh
- Evidence: tool: cargo-deny@0.20.2 in .github/workflows/ci.yml
- Recorded: 2026-09-17
- Retrospective: No

## Context

The root Rust graph had a hand-written shell pipeline and text baseline for
duplicate compatibility families, but no enforced license decision. That
pipeline duplicated a standard cargo-deny check and reduced versions to custom
families before comparing them. The broader dependency policy still has a
different job: it inventories all Cargo, npm, pub, and uv roots and validates
their tracked sources, paths, registries, and integrity fields.

ADR 0049 establishes the actor model for these checks. They catch unintended
repository changes and do not defend against a contributor with commit
authority or a correctly declared malicious dependency. This record extends
that accident gate without restoring the withdrawn supply-chain claim.

## Decision

`make deps-check` runs cargo-deny 0.20.2 offline against the published root
Cargo graph before the multi-ecosystem policy checker. CI installs that exact cargo-deny
version. The configuration denies a new duplicate version or wildcard registry
requirement and records exact exceptions for duplicate versions already in the
graph. Unmatched and unnecessary duplicate exceptions fail, so the list cannot
quietly become stale.

The allowed licenses are MIT, Apache-2.0, Apache-2.0 with the LLVM exception,
BSD-2-Clause, BSD-3-Clause, 0BSD, Zlib, Unicode-3.0, and Unlicense. MPL-2.0 is
allowed only for `option-ext` 0.2.0. `rs-fsrs` 1.2.1 is clarified as MIT only
while its packaged `LICENSE` file retains the reviewed hash. A missing manifest
license is denied, making that clarification fail closed.

The custom `scripts/deps-duplicates.txt` baseline is removed. The Python
checker remains responsible for every declared dependency root and for the
non-Cargo ecosystems. RustSec advisories remain a separate release and drift
gate.

## Consequences

- A dependency update that introduces a license outside the allowlist or a new
  duplicate version fails `make deps-check`.
- Every duplicate exception names one exact version and a reason.
- Contributors need the pinned cargo-deny executable to run `make check`.
- Fuzz and standalone Rust tool roots remain covered for source and integrity
  by the repository policy; the license and duplicate gate represents the
  published root graph.

## Alternatives considered

Keep the shell baseline and add a second license scanner. Rejected because it
would retain custom duplicate logic beside the standard tool that already
performs both checks.

Allow every detected license globally. Rejected because the one MPL-2.0 crate
and the crate without a manifest expression need narrow, reviewable treatment.

Run a networked license service. Rejected because the ordinary inner-loop gate
must remain deterministic and offline after `cargo fetch` has populated the
registry.

## Compatibility

No persisted data, public API, CLI, deck, or client contract changes.

## Security

This is compliance and accidental-change evidence, not an isolation boundary.
It trusts crate metadata and the reviewed clarification, and anyone able to
change product code can also change `deny.toml`.

## Verification

`scripts/test_dependency_gate.py` proves the version pin, command ordering,
license decision, clarification, and removal of the old baseline. Its red-first
run failed before the cargo-deny configuration existed. A hand-applied wrong
`rs-fsrs` license hash makes `make deps-check` fail on `no-license-field`.

## Reversal

Replace cargo-deny only with a pinned offline tool that enforces the same
license allowlist, crate-specific exceptions, and duplicate-version review in
one gate. Supersede this record before weakening any of those checks.
