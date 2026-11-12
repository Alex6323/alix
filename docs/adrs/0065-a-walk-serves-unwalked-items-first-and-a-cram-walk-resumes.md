# 0065: A walk serves unwalked items first, and a cram walk resumes

- Status: Accepted
- Recorded: 2026-11-13
- Retrospective: No
- Evidence: fn law_a_new_walk_serves_never_walked_items_before_earlier_misses in src/walk.rs
- Evidence: fn law_a_walk_serves_ungraded_then_missed_items_in_deck_order_else_every_item_oldest_first in src/walk.rs
- Evidence: fn law_a_cram_walk_resumes_after_the_last_walked_item_and_wraps in src/walk.rs
- Evidence: fn cram_order in src/walk.rs
- Amends: 0061 (decision 4: the order of waiting items)

## Context

Under 0061 a walk served every waiting item in deck order. A miss early in
the deck therefore came back before the cards never seen, so leaving a walk
and coming back repeated old misses before it continued. Separately, a
learner who wants to read on through a deck regardless of grades had no way
to do so: a walk always narrowed to the waiting items.

## Decision

1. While any item waits, a walk serves the items with no walk grade first,
   then the items whose last walk grade is `Fail` or `Partial`, each group
   in deck order with the existing topic and sibling spacing. When none
   wait, a walk serves every item, least recently walked first (unchanged).
2. A walk opened with cram serves every item in deck order, starting after
   the item with the newest `walked_ms` and wrapping around; with no walked
   item it starts at the first. Restarting a walk keeps its cram choice.
3. Cram is a per-open choice carried by the request (`POST /api/walk`
   `cram`, the bridge's `WalkSession::open` `cram`), not persisted.

## Consequences

- Coming back to a walk continues with unseen cards; misses follow them.
- A cram walk ignores walk grades for ordering but still writes them, so a
  later plain walk serves what the cram walk left missed.
- The walk progress fields of 0061 are unchanged.

## Alternatives considered

- Persist a walk cursor. `walked_ms` already identifies the last walked
  item, so a cursor would be a second source of the same fact.
- Make resuming the only walk order. Serving the unseen cards and then the
  misses keeps the walk's job of bringing missed cards back.

## Compatibility

No persisted format change.

## Security

No trust boundary changes.

## Verification

`walk::tests::law_a_new_walk_serves_never_walked_items_before_earlier_misses`
and `walk::tests::law_a_walk_serves_ungraded_then_missed_items_in_deck_order_else_every_item_oldest_first`
check decision 1. `walk::tests::law_a_cram_walk_resumes_after_the_last_walked_item_and_wraps`
checks decision 2, including restart; `tests/api.rs`
`a_cram_walk_opens_after_the_last_walked_item_and_serves_every_item` checks
the route.

## Reversal

If cram walks go unused, drop the flag and `cram_order`; decision 1 stands
on its own.
