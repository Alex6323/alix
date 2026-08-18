# Architecture decision records

This folder contains the durable reasons behind Alix's load-bearing technical
constraints. An architecture decision record (ADR) explains why the project
chose a direction that future changes must preserve or deliberately replace.

Specs and implementation plans may remain local working material. A decision
must become a tracked ADR before, or alongside, implementation when it
constrains one or more of:

- persisted data or compatibility;
- card identity;
- security or trust boundaries;
- public API and client boundaries;
- cross-cutting system structure;
- dependencies that are difficult to remove.

Routine implementation choices do not need ADRs.

## File names and status

Use the next four-digit sequence number and a short noun phrase:
`0002-card-identity.md`.

Every Accepted record names what proves it is in force:

```
- Evidence: <literal string> in <path>
```

`make adr-check` fails when a named string is absent from that path, so a status
cannot drift ahead of the code. Repeat the line for several markers. Use
`- Evidence: none, <why>` for a record that constrains no identifier, such as a
scope boundary or a policy. Records predating this rule are listed by the check
rather than failing it, since a marker invented in a hurry gives false assurance.

An ADR has one of these statuses:

- **Proposed:** under review and not yet binding.
- **Accepted:** the current architectural constraint.
- **Rejected:** considered but not adopted.
- **Superseded by NNNN:** replaced by a later ADR.

Accepted records are historical evidence. Do not rewrite their decision when
the architecture changes. Add a new ADR, mark the old one as superseded, and
link the two records. Small clarifications that do not alter the decision are
fine.

## Template

```markdown
# NNNN: Decision title

- Status: Proposed
- Recorded: YYYY-MM-DD
- Retrospective: No

## Decision history

Retrospective records use this optional section to identify when the decision
was introduced, the commits that verify that history, and any uncertainty.
Contemporaneous records may omit it.

## Context

What problem and user impact require a durable decision?

## Decision

What constraint does the project adopt?

## Consequences

What becomes easier, harder, or deliberately unsupported?

## Alternatives considered

What credible alternatives were rejected, and why?

## Compatibility

What persisted data, public contract, or migration boundary is affected?

## Security

What trust boundaries or threats change?

## Verification

Which tests, checks, or implementation seams enforce the decision?

## Reversal

What evidence would justify replacing this decision, and what migration would
be required?
```
