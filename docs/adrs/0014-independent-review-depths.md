# 0014: Independent review depths

- Status: Accepted
- Recorded: 2026-07-24
- Retrospective: Yes

## Decision history

The current model replaced an unreleased difficulty ladder on 2026-07-07.
Commit `1185ee3` introduced one schedule per card and review level,
`4293346` made levels session-owned, and `f22eaf0` persisted independent
schedules plus an unscheduled recognition flag. Commit `7706a44` settled the
public term on depth. Commit `e2fe493` made Recognize a choice-only depth on
2026-07-18.

## Context

Recognizing a correct answer, recalling it without cues, and reconstructing it
precisely are different retrieval demands. A learner may want to practice one
of them in a sitting. Treating success at an easier demand as proof of a harder
one would overstate mastery, while an author-prescribed interaction mode would
prevent the learner from choosing the current challenge.

Card shape still matters. Cloze, ordered lines, and atomic answers need
different checks at the same depth without becoming independent scheduling
dimensions.

## Decision

Alix exposes three learner-selected session depths:

- Recognize presents a choice question and records a recognition timestamp
  without an FSRS schedule.
- Recall uses its own FSRS state for retrieval with ordinary answer reveal.
- Reconstruct uses a separate FSRS state for typed or explained production.

Progress does not cross-credit between Recall and Reconstruct. Recognition
does not advance either FSRS schedule.

The author selects the card's reveal shape, not a fixed difficulty mode. The
core derives the concrete check from depth, reveal shape, and answer shape.
Recognize is available only when the card has enough authored or current
generated distractors to build an honest choice.

Depth is a session choice carried through the shared review contract. Clients
render the selected depth but do not derive its grading or scheduling rules.

## Consequences

- Learners can choose the retrieval demand for a sitting.
- A card can have independent Recall and Reconstruct due dates.
- Practicing several depths creates several queues and more review work.
- Recognition remains a lightweight exposure signal rather than a memory
  schedule.
- Authors describe reveal structure without freezing the learner into one
  interaction.
- Store, listing, exams, badges, and clients must name the relevant depth when
  reporting progress.

## Alternatives considered

### One schedule shared across depths

A successful recognition or recall could then postpone reconstruction despite
not demonstrating it. Schedule behavior would depend on which interaction
happened to appear.

### Propagate success from harder to easier depths

This is intuitively tempting but creates hidden coupling and makes historical
FSRS state harder to interpret. The current design keeps evidence independent.

### Author-prescribed modes

Fixed modes let an author control interaction but prevent a learner from
changing retrieval demand without editing the deck.

### A global difficulty ladder

The replaced ladder moved cards through modes as stages. It mixed content
shape, session intent, and memory scheduling into one progression.

### FSRS scheduling for Recognize

FSRS models recall. Applying it to recognition-only evidence would make the
stored stability claim stronger than the learner's action.

## Compatibility

`recognized_ms`, `recall`, and `reconstruct` in each card's progress are
persisted state. Depth names also appear in configuration, CLI arguments,
mobile bindings, and the HTTP contract.

Combining or renaming depths requires a policy for two potentially divergent
FSRS schedules and cannot be treated as a display-label change.

## Security

This decision adds no remote authorization boundary. Choice construction must
continue to withhold the correct index until feedback, as required by ADR
0007, so Recognize does not disclose its answer before the learner responds.

## Verification

- `src/depth.rs` owns depth names and the depth/reveal-to-check mapping.
- `src/store.rs` persists independent Recall and Reconstruct state and the
  recognition timestamp.
- `src/session.rs` builds depth-specific queues and applies grades to the
  selected schedule.
- `src/review.rs` constructs Recognize choices and exposes the shared state.
- Store, session, choice, contract, and mobile tests cover independent progress
  and unavailable Recognize behavior.

## Reversal

Replace this model only with learning evidence that justifies schedule
coupling or a product need that outweighs independent mastery claims. A
migration must reconcile existing Recall and Reconstruct schedules, preserve
review history honestly, and update every client and configuration surface.
