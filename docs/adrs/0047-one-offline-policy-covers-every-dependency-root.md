# 0047: One offline policy covers every dependency root

- Status: Accepted
- Evidence: cargo = "https://github.com/rust-lang/crates.io-index" in scripts/dependency-policy.toml
- Evidence: git dependency requires a 40-hex rev in scripts/check-dependency-policy.py
- Evidence: python3 scripts/check-dependency-policy.py in scripts/deps-check.sh
- Recorded: 2026-09-14
- Retrospective: No

## Context

Alix resolves build and test code through Cargo, npm, pub, and uv. Exact tool
pins and committed lockfiles make resolutions reviewable, but no single gate
proved that every manifest was governed, every external package came from an
approved registry, or every registry artifact carried the integrity data its
lock format supports.

The existing Cargo version-family check and RustSec audit answer different
questions. The family check catches avoidable parallel versions. RustSec
reports known advisories. Both remain necessary, but neither covers the other
package managers or rejects an unlisted dependency root.

## Decision

`scripts/dependency-policy.toml` is the only hand-edited declaration of
dependency roots and approved registries. `scripts/check-dependency-policy.py`
enumerates tracked manifests and fails when one is absent from that policy.
Separately, it reads both repository-root Cargo configuration spellings
(`.cargo/config.toml` and `.cargo/config`) from disk whether tracked or
untracked. It applies the same Git revision and path-containment rules to
`[patch.<source>]`, requires every top-level `paths` override to stay inside
the checkout, requires every `[source.<name>]` table to appear in the policy's
optional `[cargo].sources` list, and recursively checks explicitly included
configuration files with a cycle and depth guard. It does not discover nested
Cargo configuration files.

The four approved registry hosts and their exact origins are:

- Cargo: `github.com` at `https://github.com/rust-lang/crates.io-index`;
- npm: `registry.npmjs.org` at `https://registry.npmjs.org`;
- pub: `pub.dev` at `https://pub.dev`;
- uv: `pypi.org` at `https://pypi.org/simple`.

A Cargo Git dependency carries a full 40-hex `rev`. Git packages in a Cargo
lock carry the resolved 40-hex revision. Repository-local Cargo paths, npm
links, pub paths, and uv editable roots must resolve inside the checkout.

Registry packages carry the integrity field native to their lock format:
Cargo `checksum`, npm `integrity`, pub `sha256`, and a SHA-256 hash on every uv
artifact. Manifest and lock checks use only committed files; the two root
Cargo configuration paths and the files they include are the only
live-checkout inputs. Every check uses the Python standard library, so `make
deps-check` has no network step.

The existing Cargo version-family baseline stays in
`scripts/deps-duplicates.txt`. `scripts/deps-check.sh` runs that check and the
repository-wide policy checker. License rules, embedded-asset registration,
time-bounded exceptions, and automated update schedules are separate
decisions.

## Consequences

- A new dependency manifest cannot silently create an ungated package graph.
- A changed registry or missing lock digest fails before compilation or a
  package download.
- Generated lock formats need a small, deterministic parser in the checker.
- Adding an approved registry requires a visible policy and ADR change.
- Local path dependencies remain possible without weakening checkout
  containment.

## Alternatives considered

### Separate checker for every package manager

Native commands are useful resolution tests, but separate entry points can
miss a newly added ecosystem or manifest. One census makes coverage itself a
checked invariant while retaining native lock formats.

### A networked scanner in the normal gate

Registry queries can add advisory and license evidence, but they make ordinary
checks depend on external availability and mutable data. The blocking source
and integrity rules use committed inputs. Networked advisory drift remains a
separate job.

### Treat committed lockfiles as sufficient

A lockfile records a resolution but does not prove that every manifest has a
lock root, that its sources are approved, or that integrity fields remain
present. The policy makes those assumptions executable.

## Persisted and public boundaries

No deck, progress, API, client, or command-line contract changes. The policy
governs developer and CI inputs only.

## Security

The gate reduces unreviewed dependency-source changes and fails closed when a
tracked dependency surface is absent from policy. It does not decide license
allow-lists, query live advisory data, authenticate a registry, or produce an
artifact bill of materials.

## Verification

- `scripts/test_dependency_policy.py` plants each denied source, integrity,
  path, dependency-free, root Cargo configuration, and census case and asserts
  its exact failure line.
- `python3 scripts/check-dependency-policy.py` checks every current manifest
  and lock root without network access.
- `make deps-check` runs both the retained Cargo family check and the policy
  checker.
- `make adr-check` holds this record's evidence strings against their paths.

## Reversal

Replace the checker only when one deterministic offline gate can enumerate the
same tracked dependency roots and enforce the same source, revision,
containment, and integrity decisions. Supersede this record before changing an
approved registry or weakening a denied class.
