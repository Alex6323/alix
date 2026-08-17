# 0033: Recognize is a scheduled depth, not a one-way gate

- Status: Proposed
- Recorded: 2026-08-17
- Retrospective: No

## Context

Alix has three review depths. Recall and Reconstruct each carry an FSRS
schedule. Recognize carries a single boolean timestamp, `recognized_ms`, and
its eligibility test is `recognized_ms.is_none()`. Passing a recognition
question once removes the card from every ordinary Recognize session
permanently; only `cram` reaches it again.

The implicit theory was that Recognize is a gate rather than a depth a learner
can live at: pass it once, move up, and let Recall maintain the card, since
producing an answer is strictly harder than picking one.

That theory holds only for cards the learner advances. A card that stays at
Recognize is maintained by nothing at all. Alex observed exactly this in use
(the "Autos" quiz, 2026-08-17): recognition has much longer durability than
recall, but longer is not infinite, and a boolean can only express infinite.

A measurement made the shape of the fix clear. Applying six passing reviews at
several desired retentions produces:

    retention 0.70 -> [0, 19, 188, 1364, 7699, 35279]  days
    retention 0.85 -> [0,  7,  36,  150,  536,  1677]
    retention 0.90 -> [0,  4,  15,   48,  136,   351]

At 0.70 the sixth interval is 96 years. **The current boolean is the limit case
of a scheduled depth, not a rival design.** So this is not "add scheduling to a
depth that had none"; it is "replace an implicit infinite interval with a
finite one".

## Decision

Recognize becomes an ordinary scheduled depth.

1. `CardState` gains `recognize: Option<FsrsState>` beside `recall` and
   `reconstruct`. `schedule` and `schedule_slot` stop special-casing the depth.
   `recognized_ms` is removed.
2. `session::is_reviewable` loses its Recognize branch; all three depths run
   through `is_due`.
3. A `Pass` at Recall or Reconstruct propagates to the Recognize schedule when
   Recognize is due, and re-anchors it when it is not. This extends the
   existing Reconstruct-to-Recall propagation one step down rather than
   introducing a new mechanism.
4. Recognize counts in `reviews`, `passed` and `failed`. Its private
   `recognized` / `recognize_partly` / `recognize_missed` tallies are retired
   in favour of a generic `partial` counter serving all three depths.
5. Longer durability is expressed as tuning, not as a second mechanism:
   `review.recognize_retention`, default 0.85, reusing the existing 0.70 to
   0.99 clamp.

**The boundary this decision does not cross:** uniformity applies to progress
and scheduling, never to what a mode requires in order to ask a question. A
recognition question needs distractors, so the `assemble` partition that drops
cards which cannot build a pick, the `can_recognize` picker gate, and
`recognize_gap` all remain. Without this boundary, "no extra behaviour for
Recognize" reads as deleting the partition, which would strand cards in a
session unable to ask them anything.

## Consequences

Easier: one progress model across three depths; a card nobody maintains at a
deeper depth is now maintained at Recognize instead of being permanently
invisible; per-depth due information becomes expressible, and `alix list`
replaces its `✓` and its Recall-only `due` column with three per-depth cells.

Harder, and accepted: a card engaged at another depth but never recognized now
waits out the acquire cooldown before appearing in a Recognize session, where
today it appears at once. A Recognize sitting begins reporting an accuracy
percentage, since session counters are single-depth
(`options.depth` is fixed at construction).

Deliberately unsupported: cross-crediting of stored FSRS state between depths.
Credit flows downward on a pass; the states stay independent.

## Alternatives considered

**A fixed expiry on `recognized_ms`.** Captures fading at a fraction of the
cost, and adds a second scheduling concept beside FSRS. Rejected on conceptual
surface area.

**Cross-crediting from a lapsed Recall schedule** to make a card
Recognize-eligible again. Couples the depths, which the existing code avoided on
purpose, and still leaves a card that never reached Recall unmaintained.

**Keeping Recognize out of `reviews` while scheduling it.** Coherent, and it
keeps Recognize special in exactly the way this decision exists to end.

**A multiplier or distinct FSRS parameters** for the durability difference.
Unnecessary: retention alone spans two orders of magnitude, as measured above.

## Compatibility

Pre-1.0, so no migration. `CardState` carries
`#[serde(deny_unknown_fields)]`, verified by execution 2026-08-17: renaming a
field in a saved store yields ``unknown field `...`, expected one of
`acquired_ms`, `presented_ms`, `recall`, `reconstruct`, `recognized_ms`, ...``.
So removing `recognized_ms` makes an old store fail to open loudly rather than
dropping the field silently. That is the sanctioned pre-1.0 behaviour; recognize
progress is one boolean per card and is recoverable by one pass.

This breaks the published client contract, and `docs/API.md`, the `contract.rs`
snapshots and the `tests/contracts/*.json` corpus change with it:

- `:599` promises *"Recognize work never increments `reviews` (it is not an
  FSRS review)"*. Removed.
- `:603` describes `due_left` at Recognize as "met-but-unrecognized". Becomes
  ordinary due-ness.
- `:708` describes `reviewable_recognize` as "unrecognized **and**
  recognizable". Becomes "due **and** recognizable".

`web/alix/review/study.js` reads `reviews` at five sites, one of which computes
`state.passed / state.reviews`; it changes in the same release.

## Security

No trust boundary changes. No new input is parsed, no new file is written, and
nothing crosses a process or network boundary that did not before.

## Verification

- `CardState::schedule(Depth::Recognize)` returns the new state rather than
  `None`, and `schedule_slot` likewise.
- A test that a Recognize pass creates a schedule and a second pass extends it.
- A test that a Recall pass propagates to a due Recognize schedule and
  re-anchors a not-yet-due one, mirroring
  `a_due_reconstruct_cram_pass_credits_recall_like_a_normal_review` and
  `an_early_reconstruct_cram_pass_propagates_nothing`.
- A test that `partials_and_fails_never_propagate` still holds at the new depth.
- A test that a card with no store entry stays immediately eligible at
  Recognize while a card engaged elsewhere waits out the acquire cooldown.
- The `contract.rs` snapshots, which fail until `docs/API.md` and the corpus
  agree.

## Reversal

Evidence that would justify replacing this: recognition shown to fade while
recall stays healthy, which would break the implication that a Recall pass
proves recognition and so invalidate the propagation rule; or an observed
fading timescale far from 0.85's curve, which would move the default rather
than the decision. Reversal means restoring a boolean and dropping the
schedule, which pre-1.0 is again a loud store break rather than a migration.

**Provenance of the default, stated so it is not mistaken for a measurement:**
0.85 is inferred from the shape of the interval curve. The datum that would
validate it, the timescale on which recognition actually faded in the Autos
quiz, was asked for and never supplied. That is why it is a config key and not
a constant.
