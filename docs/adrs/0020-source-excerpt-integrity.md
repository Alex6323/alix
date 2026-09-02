# 0020: Source excerpt integrity

- Status: Accepted
- Evidence: a_trace_does_not_reveal_a_relocated_excerpt_before_repair in src/trace.rs
- Details evolved by
  [ADR 0026](0026-self-describing-ids-and-named-locator-fields.md): the citation
  locator uses named fields (`at:` then `fingerprint:` then `asset:`) and the
  fingerprint value is `xxh64-<hex>`; the fail-closed fingerprint model is unchanged.
- Recorded: 2026-07-26
- Retrospective: No
- Refines: [ADR 0015](0015-frozen-source-snapshots.md),
  [ADR 0018](0018-explicit-deck-initialization.md)
- Refined by: [ADR 0021](0021-deck-owned-frozen-assets.md)

## Context

ADR 0015 makes copied source excerpts authoritative inside generated
workspaces. Live-source decks remain useful for implementation review, but
their numeric line locators can slide onto unrelated content after an earlier
edit. A range that still resolves is not evidence that it still names the
authored excerpt.

ADR 0018 keeps ordinary doctor read-only. Source repair needs an explicit write
boundary without making routine diagnosis mutate decks.

## Decision

Every complete `at:` citation carries a non-identity xxHash64 fingerprint of
its normalized displayed excerpt. Source consumers fail closed when the
current range does not match: they show an actionable warning and do not use
the newly addressed lines for review, tutor grounding, or grading.

The hash uses seed zero and covers the displayed lines joined by newline
characters. It ignores line numbers, paths, and trailing whitespace while
preserving leading whitespace. This lets an excerpt move without changing its
fingerprint while retaining indentation as meaningful source content.

Alix may locate the exact fingerprint in another contiguous range of the same
file. A unique match is a safe locator-rebase candidate. No match means the
excerpt changed or disappeared; several matches are ambiguous. Neither case is
repaired automatically.

Plain `alix doctor` reports integrity findings and proposed unique rebases
without writing. `alix doctor --repair-source-locators` is an explicit,
narrow mutation boundary. It may stamp the current range of an incomplete
citation or apply a unique exact rebase. It writes atomically and preserves
deck and card IDs.

Generated workspaces continue to snapshot cited excerpts at the end of
`alix workspace generate`. Snapshot rewriting stamps each copied asset citation. Frozen
asset fingerprints protect the captured evidence itself; origin drift remains
a separate comparison between that evidence and live upstream source.

## Consequences

- A stale numeric range cannot silently present unrelated source.
- Live implementation-review decks remain live instead of being converted into
  snapshots.
- Exact line insertions can be repaired without reminting identity.
- Edited or ambiguous excerpts require semantic review.
- New hand-authored citations need one explicit fingerprint-stamping action.
- Fingerprints add machine metadata to each citation and must remain paired
  with repeatable `at:` directives.

## Alternatives considered

### Snapshot every cited deck

This preserves evidence but makes live development decks historical and
duplicates source where portability was not requested.

### Address excerpts by quoted text

Literal anchors are noisy, escape-heavy, and ambiguous for repeated code. A
fingerprint keeps the directive compact while preserving the same safe stop on
ambiguity.

### Repair changed excerpts fuzzily

Similarity cannot establish semantic equivalence. An edited source block is a
review event, not a locator-maintenance event.

### Make ordinary doctor mutate

Diagnostics must remain safe to run for inspection. A dedicated flag exposes
the exceptional write boundary.

### Use a Git revision

Sources include papers and ordinary folders, not only repositories. A
repository revision also identifies a whole tree rather than the bounded
evidence shown on a card.

## Compatibility

This is a pre-1.0 deck-format break. Existing citations are semantically
reviewed and fingerprinted outside production before release. Production has
one citation model; an absent fingerprint represents a newly authored,
incomplete citation and is never trusted for display.

The fingerprint is not part of deck identity, card identity, or scheduling
state. Repair changes only source metadata and preserves every ID.

## Security

The fingerprint detects accidental local drift. It is not a signature and
does not authenticate a shared deck or its source. A malicious author controls
both the locator and expected hash.

Failing closed prevents a stale locator from widening bounded source grounding
to unrelated lines. Repair searches only the already resolved file and never
uses fuzzy matching or a broader filesystem search.

## Verification

- Parser tests pin fingerprint syntax, repetition, and provenance pairing.
- Source tests cover current, uniquely moved, changed, missing, and ambiguous
  excerpts.
- Doctor tests prove default report-only behavior, explicit atomic repair, and
  ID preservation.
- Browser fact-review and trace tests prove mismatched excerpts are withheld.
- Tutor-grounding tests prove stale frozen source is not supplied.
- Snapshot tests require a fingerprint on every rewritten asset citation.
- Browser tests reproduce a shifted live locator and assert an actionable
  warning rather than unrelated source.

## Reversal

Replace fingerprints only with another source anchor that detects positional
drift, supports bounded offline evidence, fails closed on ambiguity, and keeps
identity independent from source maintenance.
