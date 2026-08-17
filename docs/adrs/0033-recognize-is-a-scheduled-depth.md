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
3. Propagation copies the existing Reconstruct-to-Recall rule exactly, all five
   clauses: only a `Pass` propagates; in cram only when the source was due; a
   missing target schedule is **not** created; an existing due target takes
   propagated credit; an existing not-due target is re-anchored. A card with no
   Recognize schedule therefore reads as available at that depth rather than as
   debt, which is what `due_at` already means when another depth has a schedule
   and this one does not.
4. **A pass propagates to every SHALLOWER depth, not one step down.**
   Reconstruct targets Recall and Recognize; Recall targets Recognize; the five
   clauses apply per source-target pair. Stated because the clauses above fix
   when propagation happens and not to whom. Read as a one-step chain, clause 3
   would break at a missing Recall and leave an existing Recognize schedule
   unmaintained while the learner demonstrably performs above it, which is the
   failure this record exists to end. It is also today's direction:
   `session.rs:420-421` marks recognition on every `Pass` before branching on
   depth, so this narrows that behaviour with guards rather than reversing it.
5. Recognize counts in `reviews`, `passed` and `failed`. Its private
   `recognized` / `recognize_partly` / `recognize_missed` tallies are retired
   in favour of a generic `partial` counter serving all three depths.
6. Longer durability is expressed as tuning, not as a second mechanism:
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

Unchanged, contrary to an earlier draft of this record: a card holding a
schedule at another depth is available at Recognize immediately, both before and
after. `due_at` returns 0 for a depth with no schedule when another depth has
one, by an explicit branch, and generalizing `schedule(Recognize)` does not
touch it. Only a card with an introduction timestamp and no schedule anywhere
waits out the introduction cooldown, which is also true today. This record
previously claimed the change introduced that wait; it does not, and the spacing
gap it implied is `{#cross-depth-recency}`, tracked separately.

Harder, and accepted: a Recognize sitting begins reporting an accuracy
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

No trust boundary changes. `review.recognize_retention` is a new config input,
read from the same already-trusted config file as the rest of `[review]` and
clamped by the existing 0.70 to 0.99 rule, so it widens no boundary. No new file
is written, and nothing crosses a process or network boundary that did not
before.

## Verification

- `CardState::schedule(Depth::Recognize)` returns the new state rather than
  `None`, and `schedule_slot` likewise.
- A test that a Recognize pass creates a schedule and a second pass extends it.
- A test that a **Reconstruct** pass credits an existing Recognize schedule
  while Recall has none. This is the discriminator between the target set ruled
  here and a one-step chain, so it is the single test that pins clause 4; under
  the chain reading it would assert the opposite.
- A test that a Recall pass takes propagated credit on a due Recognize
  schedule, mirroring
  `a_due_reconstruct_cram_pass_credits_recall_like_a_normal_review`.
- A test that it re-anchors a not-yet-due one, mirroring
  `a_reconstruct_pass_on_a_not_yet_due_recall_reanchors_without_reward`.
- A test that an early cram pass leaves a lower schedule untouched, mirroring
  `an_early_reconstruct_cram_pass_propagates_nothing`. This record previously
  cited that test as the mirror for re-anchoring, which it is not: it asserts
  "no recall credit, not even a re-anchor". Two distinct laws, corrected after
  the fifth adversarial pass.
- A test that a Recall pass does **not** create a missing Recognize schedule,
  mirroring `no_propagation_without_a_recall_schedule`. This is the clause the
  first draft of the decision got wrong, so it is the one most worth pinning.
- A test that `partials_and_fails_never_propagate` still holds at the new depth.
- A test that a card with no store entry stays immediately eligible at
  Recognize, and that a card holding a schedule at another depth is likewise
  immediately available there rather than waiting. An earlier version of this
  bullet asked for the opposite and could not have passed.
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
