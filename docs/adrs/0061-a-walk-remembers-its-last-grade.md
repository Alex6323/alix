# 0061: A walk remembers its last grade

- Status: Accepted
- Recorded: 2026-11-01
- Retrospective: No
- Evidence: pub walk_grade: Option<Grade> in src/store.rs
- Evidence: fn law_a_walk_writes_only_walked_ms_and_walk_grade_on_grade in src/walk.rs
- Evidence: fn law_a_walk_grade_follows_the_accept_table_for_every_item_kind in src/walk.rs
- Amends: 0051 (decision 1: walk memory is no longer one field)

## Context

A walk reads a deck one item at a time, an attempt before each answer, and
never schedules. Under 0051 it remembered only when each item was last
walked (`walked_ms`) and ended each item with a plain Next. The learner does
attempt every answer, so a walk can tell which items were missed and serve
those again, without touching review's schedules.

## Decision

1. A walk ends each item with review's grades. The last walk grade of each
   item is a second field in its progress record,
   `CardState.walk_grade: Option<Grade>`, holding review's `Grade` values
   (`Fail`, `Partial`, `Pass`). The last walk only; it never enters
   `history`, the counters or any schedule.
2. One write per item, when the learner leaves its answer: `walked_ms` and
   `walk_grade` together. A pick, a reveal or an opened hint writes
   nothing.
3. The accepted grades follow the item: a flip or line item takes any
   grade; a correct pick takes `Pass` or `Fail`; a wrong pick takes `Fail`.
   An opened hint turns a `Pass` into `Partial`, as in review.
4. An item is waiting unless its `walk_grade` is `Pass`. The next walk
   serves the waiting items in deck order; when none wait, it serves every
   item, least recently walked first.

## Consequences

- Two devices that both walked conflict as whole progress documents, as
  `walked_ms` already did; resolution keeps one side's walk grades.
- A binary without `walk_grade` rejects a document that contains it
  (`deny_unknown_fields`); phone and desktop update together.
- An item walked before this record has `walked_ms` and no `walk_grade` and
  counts as waiting; nothing special-cases it.
- A got item whose content is later edited stays got until the next full
  pass.

## Alternatives considered

- Store walk grades in `history`. Other features read `history` as review
  evidence, so a walk grade there would be counted as a review.
- A separate walk enum (`missed`, `partly`, `got`). The field name already
  marks the value as a walk's; a second spelling of the same three grades
  adds a vocabulary and buys nothing.

## Compatibility

Adds one optional field to the progress document. Pre-1.0: older binaries
fail loud on it, and no reader special-cases its absence.

## Security

No trust boundary changes.

## Verification

`walk::tests::law_a_walk_writes_only_walked_ms_and_walk_grade_on_grade`
walks every item kind and checks only `walked_ms` and `walk_grade` change.
`walk::tests::law_a_walk_grade_follows_the_accept_table_for_every_item_kind`
checks the accepted grades and the hint cap for every item kind.

## Reversal

If walk grades prove unused (no walk serves by them and no reader needs
them), remove the field and return to 0051's single field; old documents
are rewritten by external tooling.
