# 0035: A card tier reflects what the learner did

- Status: Proposed
- Recorded: 2026-08-18
- Retrospective: No

## Context

Alix paints each card in a deck with a tier, published to clients as a string
and rendered as a colour in the heatmap, the topology grid and the mobile
breadcrumb strip. The contract in `docs/API.md` defines `acquired` as "was
correct at least once but has not graduated" and `seen` as "was presented at
least once but never yet answered correctly".

The code does not implement that. `acquired_ms` is set by several events that
involve no correct answer, so a card the learner merely acknowledged, or whose
answer they revealed and then closed the session, is coloured as one they got
right. The contract and the code disagree, and the contract is the coherent one.

The vocabulary is wrong in the same direction. "Acquire" describes gaining
something, and the event it names is the learner pressing a button that says
they have seen a new card. Nothing has been acquired at that moment. The word
runs through an endpoint, a data transfer object field, a config key and roughly
180 identifiers, so it is not a local naming slip.

Underneath both is one question nobody had answered: **which event introduces a
card?** Presentation, revealing the answer, pressing the acknowledgment and
grading all wrote the same timestamp, so the field meant four different things
and could not be relied on for any of them.

## Decision

**A card's tier is a function of what the learner did, and the only event that
introduces a card is the acknowledgment.** The rest follows.

### The tier is derived from behaviour, not from a timestamp

|   | tier             | condition                          |
| - | ---------------- | ---------------------------------- |
| 1 | `unseen`         | no store entry                     |
| 2 | `seen`           | entry exists, nothing below matches |
| 3 | `learning`       | `total_passes > 0`, not graduated  |
| 4 | `learned-weak`   | graduated, retrievability < 0.7    |
| 5 | `learned-fading` | graduated, 0.7 <= retrievability < 0.9 |
| 6 | `learned-strong` | graduated, retrievability >= 0.9   |
| 7 | `retired`        | Recall interval at or above the cap |

`total_passes` is already incremented exactly when a grade passes, so the
published contract becomes true by pointing the tier at it.

The ladder is ordered, and `retired` is rung 7 rather than a state beside the
ladder: reaching it requires a Recall schedule at or above the cap, which only
exists after graduation and a run of successes. It is nonetheless evaluated
**before** the bands, because a retired card is graduated and its interval is so
long that its current retrievability has decayed, so band evaluation would paint
a card that is known best as `learned-weak`.

`learning` rather than `learned` for rung 3: `learned` would sit directly below
`learned-weak` while sounding stronger, leaving a reader unable to order the
family. `learning` also matches the scheduler's own state 1 label, so the wire
word and the internal word finally agree, and the -ing versus -ed contrast marks
the graduation boundary between rung 3 and rungs 4 to 6.

### Only the acknowledgment introduces

`introduced_ms` is written by the acknowledgment and by nothing else.

- **Presentation writes nothing.** Showing a card creates no store entry, so
  `unseen` and `seen` are separated by the acknowledgment alone. `presented_ms`
  is deleted rather than renamed.
- **Revealing writes nothing.** A learner who reveals an answer and leaves
  without acknowledging persists nothing and meets the card as new next sitting.
  This reverses shipped behaviour deliberately: if revealing introduces, the
  acknowledgment records nothing that has not already happened and the control
  is decorative.
- **Grading writes nothing.** See the dependency below, which is what makes this
  safe.
- **The constructor writes nothing.** Two construction paths disagreed, one
  stamping and one not, so any future call that materialised an entry for a
  never-introduced card marked it introduced. Both now agree.

### Durable state changes only when leaving a card

Generalising the above into the rule that prevents its recurrence: **for every
session entry point that can mutate the store, either the current card changes
or the store is unchanged.** Grading and acknowledging both write and then
advance, so they satisfy it; presentation wrote on arrival and revealing wrote
mid-card, which is why both were wrong. This is pinned as one law with a row per
entry point, so a future mid-card writer fails immediately rather than shipping.

### The vocabulary is replaced, not aliased

A card is **introduced**; the learner presses the acknowledgment;
`introduced_ms` records it; `stats.introduced` counts it. The word "acquire"
is retired everywhere it appears, including the `/api/acquire` endpoint, the
`ReviewState.acquire` field, the `acquire_cooldown` config key and its resolved
milliseconds.

### `/api/reveal` is deleted

Once revealing writes nothing, the endpoint has no observable result: the
session-local revealed set fed two decisions, and the only reason it changed
either was that revealing first created progress. Publishing an endpoint that
does nothing bills every learner a round trip per first reveal and offers third
parties a stable-looking surface that is a no-op. The whole slice goes: the
store write, the session state, the command, the handle, the route, all client
call sites, and its documentation.

## Dependency on ADR 0033

**Deleting the grade writer is only safe once Recognize is a scheduled depth.**
The writer existed as an anchor: a card with no schedule and no introduction
timestamp falls through to an epoch anchor and becomes immediately due. Under
ADR 0033 the scheduler creates a missing schedule on any grade at any depth,
including Recognize, so a graded card always has a schedule to be scheduled by
and never consults the introduction anchor.

Landing this record without ADR 0033 reintroduces the epoch anchor for any card
graded at Recognize. The two ship together, or this one keeps the grade writer
until the other lands. This dependency is stated because it was missed once: the
two decisions were correct separately and wrong composed, and only an
adversarial pass across both documents caught it.

## Consequences

The published tier vocabulary changes, so every client that maps tier strings to
colours changes with it, including the mobile breadcrumb strip, which currently
cases on `acquired` and paints anything unrecognised with the same fill as an
untouched card. Missing that file would paint every actively-learning card as
one the learner has never worked on.

A learner who reveals an answer and leaves is met by that card as new next
sitting. That is the intended reading of an unfinished introduction.

Progress files written before this change fail to open, loudly, because
`CardState` carries `deny_unknown_fields`. That is the sanctioned pre-1.0
behaviour and is preferable to silently ignoring a renamed field and dropping
its data on the next save.

## Alternatives considered

**Rename the field to match the code** (`engaged_ms` was proposed and
withdrawn). The field is write-once and all its writers guard it, so a
continuous-tense name misleads, and `CardState::engaged()` already names a
broader union of five facts, so the name would collide with an existing concept.

**Change the documentation to match the code.** Rejected on the merits: a tier
ladder in which "correct at least once" is reachable without ever answering is
not a ladder anyone can reason about, and the colours would keep lying.

**Strip the writers and keep the tier reading a timestamp.** This was the first
recommendation here and it destroyed information: the record that a learner saw
an answer is worth keeping, it was simply being asked to also mean "introduced".
Pointing the tier at `total_passes` keeps both facts and needs one predicate.

**Keep the grade writer as an anchor backstop.** Correct only while Recognize
has no schedule slot. See the dependency above.

## Compatibility

Pre-1.0, so no migration and no old-format recognition. Old progress files fail
to open as ordinary invalid input, with no dedicated message and no conversion
tooling. Any conversion is done by disposable tooling outside this repository.

The tier strings, the renamed statistics field, the renamed and deleted
endpoints and the renamed config key are all published surface, so `docs/API.md`,
the contract snapshots, the contract corpus, both web clients, the mobile bridge
and generated Dart move together, with a changelog entry under Changed.

## Security

No trust boundary changes. One endpoint is removed and one renamed, which
narrows the surface rather than widening it. No new input is parsed and nothing
crosses a process or network boundary that did not before.

## Verification

- A first-sight failure leaves a card without an introduction timestamp, without
  passes, and at tier `seen`.
- Each of the seven tiers, asserted from a constructed state, including that a
  retired card reads `retired` rather than falling into a band.
- Revealing and then abandoning a sitting persists nothing, and the card is
  `unseen` next sitting.
- Grading without acknowledging leaves no introduction timestamp, while the
  grade itself supplies the schedule, at all three depths and for a partial.
- A never-introduced card touched by the store's get-or-insert path stays
  un-introduced.
- The departure law, one row per session entry point that takes a mutable store.
- Revealing produces a byte-identical store, the same current card, the same
  selection decision, the same transfer object and the same revision as not
  revealing. This must be run before the endpoint is deleted, because a wrong
  deletion is silent.
- An exhaustive tier-to-colour test in each client, so a renamed tier cannot
  fall through to a default fill.

## Reversal

Evidence that would justify replacing this: a learner-visible need for the
"saw the answer" fact to survive an abandoned introduction, which would restore
a durable reveal record under its own field rather than reinstating the
overloaded one. Reversing the tier definition would mean accepting that a colour
meaning "you got this right" is shown for cards never answered, which is the
defect this record exists to remove.
