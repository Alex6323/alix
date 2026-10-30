# 0059: A hint is a tip on the question side

- Status: Accepted
- Recorded: 2026-10-30
- Retrospective: No
- Evidence: fn every_question_side_tip_becomes_a_hint_in_authored_order in src/parser/mod.rs
- Evidence: fn a_misplaced_question_side_note_names_the_first_rule_it_breaks in src/parser/mod.rs
- Evidence: fn adding_or_editing_a_hint_changes_no_fingerprint in src/parser/mod.rs

## Context

A learner stuck on a card can only reveal the whole answer. A hint
offered before the answer, in steps that move closer to it, keeps the
attempt alive. The deck format needs a place to write one.

A card already has two sides in its Markdown: the lines between the
`##` heading and the `---` divider are the question side (a front image
sits there), and the lines below the divider are the answer side, where
GitHub's alert badges (`> [!NOTE]` and four others) write notes shown
after answering. Before this decision a badged block above the divider
was an error.

## Decision

1. **A hint is a `> [!TIP]` block on the question side**, between the
   card's front and its divider. Several are that many hints, in
   authored order. A hint is not front text and not an answer line, and
   it is in neither of the card's fingerprints.
2. **Only `[!TIP]` may sit above the divider.** Any other badge there is
   an error at its badge line: "only a `[!TIP]` (a hint) may sit above
   the divider; move this note below the answer". A `[!TIP]` below the
   answer stays a note.
3. **A card with a `blank:`, `cover:` or `crop:` region takes no hint**:
   an error at the first tip's line.
4. Mixed input has one answer, checked in this order: a non-TIP badge,
   then a directive or `ignore` between a hint and the divider (reported
   as the existing directive-above-content error), then a region, then
   the hints.

## Consequences

Easier: no new marker; the rule "position decides the side" already
governs front images. In another Markdown viewer the hint renders as a
Tip box under the question.

Harder, and accepted: a one-line `## q` card needs a `---` divider, with
a blank line above it, to carry a hint, as it does for a front image.
The same `[!TIP]` means a hint above the divider and a note below it.

## Alternatives considered

**A bare quote marked `<!-- hint -->` below the answer.** Rejected: a
new marker with its own errors (no quote above it, a value, a heading
line), invisible in other Markdown viewers, and it turned answer content
into something else.

**Any badge above the divider is a hint.** Rejected: a misplaced
`[!WARNING]` meant as a visible caveat would silently become a hidden
hint; with only `[!TIP]` the mistake stays an error.

## Compatibility

Every input this decision gives a meaning to was an error before it, so
no valid deck changes meaning. A `> [!TIP]` line inside an already open
ordinary quote stays part of that quote.

## Security

None changed: a hint renders through the same path as a note.

## Verification

The tests in the Evidence lines: each accepted shape and its hints, each
misplaced shape and its exact error, and fingerprints that ignore hints.

## Reversal

If authors need visible question-side notes, another badge above the
divider could be given that meaning; today it is an error, so the space
is free.
