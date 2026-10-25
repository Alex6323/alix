# 0056: A pending note settles on the card it was asked on

- Status: Accepted
- Recorded: 2026-10-25
- Retrospective: No
- Evidence: fn a_tutor_note_arriving_after_a_card_advance_lands_on_the_card_it_was_asked_on in tests/api.rs
- Evidence: fn a_tutor_answer_arriving_after_a_card_advance_is_dropped in tests/api.rs
- Evidence: fn a_tutor_draft_arriving_after_a_card_advance_is_dropped in tests/api.rs
- Amends: 0027 (the tutor paragraph: advancing the card no longer makes a
  pending note ineligible)

## Context

ADR 0027 made every tutor completion ineligible once the learner advanced
the card: the pending exchange was dropped and its subprocess reaped, so a
late answer never appeared under the wrong card. A note condense is a
different kind of completion. Its result is addressed to a card id and
written to the deck's personal sidecar, not to the transcript, so landing it
after an advance can never touch the new card. Dropping it lost work the
learner had asked for, silently: the condense takes seconds, a learner who
pressed Make this a note and moved on got nothing, and the web panel's leave
text said the note was kept. The phone client already kept such a
conversation alive until its note landed.

## Decision

1. On a card change, a pending exchange whose purpose is a note condense
   keeps running. When it completes, the note is written to the card the
   exchange was started on, through the same path a note on the current
   card takes. The new card's transcript and its note bookkeeping are not
   touched.
2. A pending question or card draft left behind by a card change is still
   dropped and its subprocess reaped, as ADR 0027 says: their result belongs
   to a transcript that no longer exists.
3. While the left card's note settles, the single tutor slot stays held:
   no new call starts, and the Ask contract reports `previous_card` so a
   client can say the tutor is still working on the previous card. The poll
   that sees the note land reports `status: "note saved"`.
4. Leaving the deck or closing the study context still drops every pending
   exchange, notes included.

## Consequences

A note asked for just before moving on lands instead of vanishing, on web
and phone alike. The next card's tutor waits for the slot for the seconds
the condense still needs. A card the learner left is not updated in memory
within the same sitting; the written note is read with the deck the next
time it opens.

## Alternatives considered

- A second worker so the next card's tutor need not wait: more machinery
  than a wait of seconds is worth.
- Keep dropping and fix only the leave text: honest, but the work the
  learner asked for is still lost.

## Compatibility

`AskDto` gains `previous_card: bool`; the contract snapshots and
`docs/API.md` carry it. No persisted format changes.

## Security

None. The note is written by the path a current-card note already uses, to
the same sidecar, addressed by the id captured when the exchange started.

## Verification

The three evidence tests drive the fake backend through a FIFO so the reply
arrives after the advance: the note lands under the left card's id and not
the new card's, the held slot reports `previous_card`, and a late answer
and a late draft leave nothing behind.

## Reversal

If the tutor ever gains a transcript per card that survives an advance, a
late answer could settle the same way and this record would widen; if the
slot wait proves costly, a second worker would replace rule 3.
