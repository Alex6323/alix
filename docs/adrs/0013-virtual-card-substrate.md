# 0013: Virtual-card substrate

- Status: Accepted
- Recorded: 2026-07-24
- Retrospective: Yes
- Refined by: [ADR 0017](0017-per-deck-state-documents.md), which keeps each
  virtual card in its parent deck's progress document instead of an aggregate
  workspace file.

## Decision history

Commit `50fc328` added virtual cards to the progress store on 2026-07-05.
Commits `9ed351f`, `39b1c58`, and `33662c5` integrated their scheduling,
due-counting, and retirement with normal sessions. Commit `5f65c9e` made exam
remediation create virtual cards instead of editing a deck. Commits `66ac3ad`
and `6330d8c` added explicit promotion while preserving schedule state.
Commit `d599aa5` unified virtual and authored schedules in the same card-state
map.

Tutor cards became a second producer in commit `f628e76` on 2026-07-11.
Commit `1724326` moved virtual cards to the minted identity model on
2026-07-19.

## Context

AI remediation and tutor conversations can suggest useful cards, but writing
model output directly into an authored deck would make unreviewed generated
content permanent. Keeping suggestions only in memory would lose learning
state between sessions and prevent them from participating in normal
scheduling.

Generated cards need a probationary lifecycle: drill them with real review
state, discard them without touching the deck, or deliberately promote them
into authored material.

## Decision

Generated learning cards may live as virtual cards in their parent deck's
`progress/<alix-id>.json` document.
A virtual card records minted identity, kind, parent deck, Markdown card text,
and creation metadata. Its schedule is stored in the same card-state map and
uses the same FSRS and session behavior as an authored card.

Sessions join due virtual cards to their parent deck's roster. Reset, due
counts, retirement, deduplication, remote outcome application, and review
projection treat them as real learning items while preserving their virtual
provenance.

Creating remediation or tutor cards does not edit the authored deck. Promotion
is an explicit learner action that:

1. validates and appends the card to the target Markdown deck;
2. preserves its identity and review schedule; and
3. removes the virtual entry after the authored write succeeds.

Remediation and Tutor are producers and kinds built on this substrate, not
separate persistence architectures.

## Consequences

- AI-generated cards can be tested through actual review before becoming
  authored content.
- Generated cards survive application restarts and participate in scheduling.
- A deck's progress document contains regenerable card text in addition to
  learner state.
- Store reset and cleanup rules must account for both authored and virtual
  cards.
- Promotion crosses from personal state into a user-authored file and therefore
  requires explicit intent and atomic ordering.
- New virtual-card kinds reuse the substrate but still need product-specific
  lifecycle rules.

## Alternatives considered

### Append generated remediation directly to the deck

This makes model output permanent before the learner has evaluated it and
creates noisy authored-file changes during an exam.

### Keep generated cards in memory

In-memory cards disappear across sessions and cannot accumulate meaningful
schedule state.

### Maintain a separate scheduler for generated cards

This would duplicate due, grade, retirement, and promotion behavior and make a
promoted card change learning semantics.

### Store generated decks as hidden Markdown files

Hidden decks would complicate scanning, sharing, identity, and parent-deck
membership while still requiring a promotion protocol.

## Compatibility

Virtual-card records and their shared schedule keys are persisted personal
state. Their shape currently follows the pre-1.0 best-effort policy in ADR
0005; stale regenerable virtual entries may be dropped without failing the
whole store.

Promotion must preserve the token and schedule so the same learning item does
not restart under a new identity.

## Security

Generated Markdown is untrusted model output. Creation and promotion must parse
and validate its shape, mint or verify identity, reject invalid embedded
content, and avoid overwriting authored files after partial failure.

Virtual cards remain personal until promoted. Sharing rules from ADR 0001 must
not leak them as authored deck content accidentally.

## Verification

- `src/store.rs` owns `VirtualCard`, kinds, persistence, creation, promotion,
  deduplication, and failure-order tests.
- `src/session.rs` proves virtual cards join due queues and use normal schedule
  state.
- `src/exam.rs` verifies remediation leaves the deck file unchanged.
- `src/review.rs` and client contract tests preserve virtual provenance and
  promotion availability.
- Mobile bridge tests apply remote-generated cards to the phone-owned store.

## Reversal

Replace this substrate when generated cards need collaboration, provenance, or
editing semantics that personal progress cannot represent. A migration must
preserve identity and schedules, define what happens to unpromoted content,
and keep promotion from duplicating or losing learner state.
