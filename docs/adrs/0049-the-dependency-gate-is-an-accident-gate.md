# 0049: The dependency gate prevents accidents, not attacks

- Status: Accepted
- Evidence: none, this record narrows the claim made for an existing check and
  adds no identifier of its own.
- Recorded: 2026-09-16
- Retrospective: No
- Supersedes:
  [ADR 0048](0048-root-cargo-configuration-is-reviewed.md), whose framing of
  the root-configuration rule as a supply-chain control this record replaces.
  The rule itself is unchanged.

## Context

ADR 0047 introduced one offline policy over every dependency root, and ADR 0048
added the requirement that a root Cargo configuration be tracked. Both were
recorded as supply-chain controls, the security guide described the result as a
gate that fails closed, and the roadmap item behind them is classified as
security work derived from the 2026-07-23 engineering review finding 8.

Reviewing that claim against the code produced no attacker for it. The
manifests, lockfiles, and policy file the checker reads are tracked, so the
actor it could stop already holds commit authority, and an actor with commit
authority can edit the policy file in the same change or write ordinary code
into the product. One input is not tracked, the presence of a root Cargo
configuration, and the same authority adds that file to the index in the change
that introduces it. Cargo itself already refuses a lockfile with a missing
checksum under `--locked`, and a dependency that is correctly declared, pinned
and checksummed passes every rule the checker has, which is the shape most
published supply-chain attacks take.

Finding 8 is mostly about the build environment and the released artifact:
toolchain and action pinning, advisories, licenses, SBOM, provenance, and
installer verification. Its dependency recommendation names a standard tool.
The checker answers a narrower question than the finding asked.

## Decision

The dependency gate is recorded as an accident gate over this repository's own
dependency declarations. It exists to make an unintended change to a
dependency root visible at commit time. It is not a defence against a hostile
contributor, a compromised dependency, or a compromised machine, and no
document may describe it as one.

Its rules stay as they are. No rule is added for an input this repository does
not contain, and the requirement that files included by a root Cargo
configuration be tracked is not adopted, because no such configuration exists
here. A check that stays silent when it cannot read its input contradicts the
purpose of an accident gate and is a defect.

## Consequences

The security guide states what the gate does and what it leaves open. The
roadmap item is reclassified, and the remainder of finding 8 is tracked where
its real exposure is, in release artifact provenance.

Growing the checker further requires naming the ordinary action that produces
the input it would catch. Refusing an untracked root configuration has one, a
local build setting written into `.cargo/config.toml` and left uncommitted,
where the gate is what makes a machine-only build difference visible. The
walker over a tracked configuration has no such action today and stays only
because a repository configuration may be added later.

## Alternatives considered

Removing the checker entirely. Its accident cases are real and cheap to keep:
a dependency added to a surface declared dependency-free, a git dependency on a
moving branch, a path dependency outside the checkout, an undeclared dependency
root, an uncommitted lockfile, and a lockfile damaged by a merge.

Keeping the security framing and adding compensating rules. Rejected. Six
changes in fifteen hours each closed a bypass found in the previous one, every
input was written by the reviewer, and none of them had an actor.

## Compatibility

No persisted data, public contract, or client boundary is affected.

## Security

The claim made for the check is narrowed, and the exposures it does not cover
are named in the security guide: a correctly pinned malicious dependency, any
configuration outside the checkout, dependency build scripts and proc macros
executing with the authority of whoever runs the build, and artifact
provenance.

## Verification

`scripts/test_dependency_policy.py` covers every denied class, including the
two silent-pass defects this record calls defects: a lockfile layout the pub
reader cannot parse, and an integrity value that is not a full digest.

## Reversal

Regular outside contributions would change the actor model, since a stranger's
pull request is exactly the case where a change hiding in a lockfile is not
reviewed as carefully as a change in source code. That evidence would justify
restoring a security framing and extending the rules.
