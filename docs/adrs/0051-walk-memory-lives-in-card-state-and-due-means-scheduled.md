# 0051: Walk memory lives in the card state, and due means scheduled

- Status: Accepted
- Recorded: 2026-10-05
- Retrospective: No
- Evidence: pub walked_ms: Option<u64> in src/store.rs
- Evidence: fn law_an_engaged_ungraded_card_takes_a_new_slot_never_a_due_slot in src/session.rs
- Evidence: fn law_a_walked_card_is_graded_at_first_sight_once_the_settle_gap_passes in src/session.rs
- Evidence: fn law_the_only_store_write_a_walk_makes_is_walked_ms_on_next in src/walk.rs

## Context

Walk is a session mode for reading a deck with an attempt before each answer,
without grading it: material for a talk or a review that is not meant to
enter long-term drilling. A walk continues where the previous one stopped and
rotates through the deck, so it needs per-item memory of when each item was
last walked.

The progress store already holds one `CardState` per drill item (a card, a
cloze blank, a direction, a table row), and its document carries
`deny_unknown_fields`. Progress syncs between a phone and the desktop as a
whole document with a revision check; there is no per-field merge.

Before this record the drill treated every item with a progress entry and no
schedule as due once the settle gap after its introduction had passed. An
introduction writes no schedule, so "due" did not imply a schedule.

## Decision

1. Each item's walk memory is one field, `CardState.walked_ms`, in the
   existing progress document. No separate walk file.
2. Walking counts for the drill fully: `walked_ms` is part of
   `CardState::engaged()`. A walked item is no longer new, is not introduced
   a second time, and is graded at first sight when the drill serves it. The
   drill's settle gap counts from the later of `introduced_ms` and
   `walked_ms`.
3. Due means a schedule exists and its date has come. An engaged item with no
   schedule (introduced or walked, never graded) competes in the drill's
   new-card share, not the due pool.

## Consequences

- A walked item shows at least as "seen" on every drill surface (tiers, the
  drawer, reset, doctor), because they read any progress entry.
- A walk saves the progress document at every item, so a desktop walk and a
  phone's unsynced drill session conflict on the next sync, and resolving it
  keeps one side. Per-card merge stays the named deferral of ADR 0042.
- A binary without `walked_ms` rejects a document that contains it; phone and
  desktop update together.
- A large walk cannot crowd real reviews out of the drill: its items wait in
  the new-card share.
- The drill's own introductions change across sittings: an item introduced
  and left ungraded takes a new-card slot in the next sitting instead of a
  due slot. Within one sitting nothing changes; the roster is fixed when the
  sitting opens.

## Alternatives considered

- A separate per-deck walk document. It isolates walk memory from the drill
  and from sync conflicts, at the cost of a new file and per-device memory.
  Rejected by the project owner.
- Walk memory as "seen" only, outside `engaged()`, so the drill still
  introduces walked items. Rejected: Walk already gives each item an attempt
  and its answer, which is what the drill's introduction does.
- Keeping unscheduled items in the due pool and ranking them after real
  reviews. Rejected in favour of fixing the meaning of "due".

## Compatibility

Pre-1.0: no reader of the old document shape is kept. The new field is
optional and skipped when absent, so existing progress documents load
unchanged.

## Verification

`session::tests::law_a_walked_card_is_graded_at_first_sight_once_the_settle_gap_passes`
walks an item and checks it is engaged, not new, graded at first sight, and
not served before `walked_ms` plus the settle gap.
`session::tests::law_an_engaged_ungraded_card_takes_a_new_slot_never_a_due_slot`
checks an engaged item with no schedule is served from the new-card share and
never from the due pool. `walk::tests::law_the_only_store_write_a_walk_makes_is_walked_ms_on_next`
checks a walk writes nothing but `walked_ms`.

## Reversal

Moving walk memory out of `CardState` needs a new document and a conversion
of existing `walked_ms` values by external tooling; restoring "due" for
unscheduled items is a change to `build_queue` and `is_due` alone.
