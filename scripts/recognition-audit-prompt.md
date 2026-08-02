# Old-format recognition audit

You are auditing the alix repository for violations of its pre-1.0
no-recognition rule. Print a verdict as the FIRST line of your reply,
exactly one of:

    RECOGNITION AUDIT: PASS
    RECOGNITION AUDIT: FAIL

followed by your findings (empty on PASS).

## The rule

Pre-1.0 alix has no backwards compatibility, and production and diagnostic
code never recognizes an old format. Concretely, a violation is ANY of:

- Code that matches on, names, or special-cases a retired key, field,
  grammar, id shape, filename shape, or vocabulary from an earlier format
  (for example a dedicated error, message, lint, classification, or code
  path for a key that the current format no longer defines).
- Dual readers, version fences, markers, sentinels, aliases, old-path
  derived addressing, or any repair/upgrade path for old artifacts.
- A user-facing message that suggests conversion or migration tooling, or
  that classifies an artifact using retired-format vocabulary
  ("un-converted", "old format", a retired key's name).
- Identifiers, comments, or filenames whose purpose is to carry knowledge
  of a format the current design no longer defines.

## What is NOT a violation

- Strict validation of the CURRENT grammar: unknown keys linting
  generically, invalid ids or locators failing the current grammar,
  unreadable documents erroring generically. Rejecting bad input is the
  design; recognizing WHICH old format the input came from is the
  violation.
- Test fixtures that feed arbitrary invalid input (including strings that
  happen to be shapes an old format once used) and assert generic
  rejection. A test becomes a violation only when it asserts
  retired-format-specific handling (a dedicated message or class for the
  old shape).
- Version pins and format-version checks of the current design
  (`format-version: 1` is current, not compat).
- Prose history in ADRs and the changelog. The book and other user docs
  must not describe recognition behavior that no longer exists, but naming
  history as history is fine.

## How to audit

Read the audit manifest below. Use Grep to sweep for candidate markers,
then Read the surrounding code and judge semantically; do not report a
match without reading its context. Judge every hit against the two lists
above. Report each finding as `file:line`, the offending excerpt, and one
sentence on why it is recognition rather than current-grammar validation.
Be exhaustive: a silent miss here ships the violation.
