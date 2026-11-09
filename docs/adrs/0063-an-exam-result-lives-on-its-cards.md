# 0063: An exam result lives on its cards; the deck keeps two instants and its last full count

- Status: Accepted
- Recorded: 2026-11-04
- Retrospective: No
- Amended by: 0064 (the card-level `CardState.exam_result` and the section
  status derived from member records, replaced by per-topic
  `DeckProgress.topic_results` keyed by topic id with the source
  fingerprint; the `--exam` reset clause's "card records", now the topic
  results and the evidence stamp)
- Evidence: pub passed_at_ms: Option<u64> in src/store.rs
- Evidence: pub full_exam_result: Option<FullExamResult> in src/store.rs
- Evidence: pub fn exam_status(deck: &Deck, store: &Store) -> ExamStatus in src/exam_status.rs

## Context

The exam used to be a one-shot gate: one pass set the deck's
`mastered_at_ms`, a fail set `exam_failed_at_ms`, and both described the
whole deck. The exam is becoming repeatable practice that examines a deck
section by section (its `#` headings) and can be filtered to the sections
that still need work. A deck-level flag cannot say which sections passed,
and sections have no stable identity of their own: a heading can be
renamed, cards move between headings, and new cards arrive.

Unlocking also changed meaning. A dependent deck's exam unlocks when every
section of its prerequisite passes at `[exam] pass_threshold`, while
"mastered" is reserved for a full sitting in which every answer was PASS.
One flag cannot carry both.

## Decision

- Each examined card stores its section's latest verdict as
  `CardState.exam_result {at_ms, pass}`. A later sitting overwrites it. A
  card moved to another section carries its record.
- Section status is derived at read time from the live membership, never
  stored: unexamined (no member has a record), needs work (any member's
  record is a fail), has new cards (some members lack a record, every
  present record is a pass), passed (every member passed). Any fail wins.
- `DeckProgress.passed_at_ms` is set the first time every section of the
  deck reads passed. It is the unlock flag. `DeckProgress.mastered_at_ms`
  is set the first time a full sitting scores every answer PASS. Neither is
  cleared by a later result or an added section; only a reset clears them,
  both together.
- `DeckProgress.full_exam_result {at_ms, pass, partial, total}` holds the
  raw verdict counts of the last full sitting. A filtered sitting never
  writes it. Points are derived (pass + partial / 2).
- A deck that has ever sat its exam (any member record, or a stored full
  count) is never locked by its prerequisites again. The lock check still
  recurses through a passed prerequisite's own requirements, so a deck that
  never sat locks again when an ancestor's exam is reset.
- `alix reset <deck> --exam` clears the card records, both instants and the
  full count, keeps drilling progress, and first names the decks whose
  exams it will lock.
- `exam_failed_at_ms` is removed.

## Consequences

- Section filters, badges and lock checks read one lib function,
  `exam_status`, from the cards and the store; no section id is persisted.
- A re-pass never moves the Mastered window, because the instants are
  monotonic.
- A card examined before any drilling stays new: `CardState::engaged()`
  ignores `exam_result`.
- A section whose members were all reset with `--card` reads unexamined.

## Alternatives considered

- Section records keyed by heading text: breaks on every rename and merge.
- A per-deck list of section results: needs section identity the deck
  format does not have.
- Keeping one deck flag for both unlocking and mastery: makes "mastered"
  mean "passed at the threshold", which below a 1.0 threshold is untrue.

## Compatibility

Pre-1.0, no compatibility: a progress document that still carries
`exam_failed_at_ms` fails to parse (`deny_unknown_fields`). Existing exam
state is reset outside the repository, not converted.

## Security

No trust boundary changes.

## Verification

- `store::tests::recording_follows_the_four_steps`
- `exam::tests::section_status_follows_member_records_and_any_fail_wins`
- `deck::tests::locks_gate_exams_by_passed_at_ms_exempt_ever_sat_and_recurse_f44`
- `library::tests::an_exam_reset_names_the_never_examined_dependents_it_locks_transitively`
- `reset_exam_clears_exam_state_only_and_lists_never_examined_dependents`
  in tests/cli.rs

## Reversal

If sections gain a stable identity in the deck format, records could move
to sections. Existing card records would then be reset, not converted.
