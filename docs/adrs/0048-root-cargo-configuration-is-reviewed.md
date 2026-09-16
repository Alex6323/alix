# 0048: Root Cargo configuration must be tracked

- Status: Superseded by
  [ADR 0049](0049-the-dependency-gate-is-an-accident-gate.md)
- Evidence: untracked Cargo config is refused in scripts/check-dependency-policy.py
- Recorded: 2026-09-14
- Retrospective: No
- Supersedes:
  [ADR 0047](0047-one-offline-policy-covers-every-dependency-root.md), whose
  live untracked root-configuration allowance this record replaces.

## Context

ADR 0047 made both repository-root Cargo configuration spellings live inputs
to the offline dependency policy whether or not Git tracked them. That closed
gaps in Cargo's source, patch, and path tables, but left a root build entry
point invisible to review and absent from a fresh clone. A harmless-looking
untracked `.cargo/config.toml` can also be forgotten after local tooling or an
agent creates it.

The checker runs with the local user's filesystem authority. It cannot defend
against an attacker who can rewrite files on that machine. It can make an
accidental or tool-created untracked root configuration visible before the
build proceeds.

## Decision

If `.cargo/config.toml` or `.cargo/config` exists at the repository root, the
dependency policy requires that exact path to appear in `git ls-files`. An
untracked root configuration fails before any of its Cargo tables are checked,
with an instruction to track or remove it.

A tracked root configuration retains ADR 0047's existing checks for includes,
Git revisions, path containment, source declarations, cycles, and include
depth. Explicitly included files remain live checkout inputs under those
checks; this record adds a tracked-file requirement only to Cargo's two root
configuration entry points.

This is deliberately narrower than requiring every included file to be
tracked. An include must be declared by a tracked root, and the existing walker
fails if it is missing or violates the containment and table rules. Its content
may still be an untracked live input, so this decision does not claim that the
complete effective Cargo configuration is reproducible from Git.

## Consequences

- The root configuration path and its recorded baseline become visible in
  review and fresh clones.
- Tooling, agents, and forgotten local files trip the ordinary offline gate.
- A deliberate local root configuration must be committed or removed before
  the dependency gate passes.
- Included configuration files keep their existing containment and table
  checks without gaining a new tracking rule.

## Alternatives considered

### Continue checking untracked root configurations in place

Parsing catches disallowed table values, but harmless or newly supported Cargo
settings can still alter a build without appearing in review. Reading the file
is not a substitute for recording it.

### Ignore untracked root configurations

Cargo still reads them. Ignoring the files would make the policy describe a
different build environment from the one Cargo uses.

### Treat this as protection from a machine-local attacker

A process with the user's write authority can alter tracked files or Git's
index too. This is a reviewability and forgotten-file tripwire, not a sandbox
or authentication boundary.

## Compatibility

No deck, progress, API, client, or user-facing command contract changes. A
developer checkout with an untracked root Cargo configuration must track or
remove that file before `make deps-check` passes.

## Security

The gate reduces build-input changes that exist only in one checkout. It does
not protect against an attacker with write access to the machine or establish
the authenticity of tracked configuration.

## Verification

`scripts/test_dependency_policy.py` covers harmless untracked files under both
root spellings and proves that the same tracked configuration continues into
the existing Cargo table checks. `make deps-check` applies the rule to the
current checkout without network access.

## Reversal

Supersede this record when another deterministic gate can admit an untracked
root entry point without hiding that entry point from review or a fresh clone,
or when the tracking boundary expands to included configuration files.
