# 0039: Span blanks are the only cloze; the inline `\blank` marker retires

- Status: Accepted
- Evidence: a_blank_marker_is_literal_answer_text in src/parser/mod.rs
- Evidence: is_blank_card in src/card.rs
- Recorded: 2026-08-31
- Retrospective: No

## Context

The format carried two cloze systems. The older one wrapped the hidden
text inline (`\blank{answer}`, optionally `\blank[name]{answer}` per
ADR 0032), derived positional sub-ids (`card-<token>-N`), and kept
schedules across edits through a fingerprint realignment cascade. The
newer one, built for image regions and extended to text (ADR 0034),
leaves the prose untouched and hides a span by a trailing directive
(`<!-- blank: span hidden="..." b:<stamp> -->`) whose minted stamp is
the card's identity.

Two systems for one concept meant two grammars for authors, two
identity models, prose that reads badly in any other Markdown renderer,
and a second deck-breaking change later if the older grammar survived
0.8.

## Decision

**1. Span and rect blanks are the only cloze.** `\blank{...}` is not
deck syntax: it parses as ordinary literal answer text, with no
recognition, no lint, and no dedicated message (the pre-1.0 rule).

**2. Positional sub-ids die with the marker.** `card-<token>-N` is no
longer minted, composed, or parsed; an id carrying a numeric suffix
fails as an ordinary invalid id. Blank-card identity is the region
stamp (`-b<stamp>`) or the derived group hash (`-g<hash>`, ADR 0034).

**3. The fingerprint realignment cascade is deleted.** Stamp identity
makes positional remapping unnecessary: an edit moves the directive,
not the identity. The store's hole-records family (`HoleFingerprint`,
`CardRecords`, the realign and remap paths) is gone, and an old store
document carrying it fails loud on its unknown field.

**4. Per-hole addressing carries over keyed on region names.** The
addressed-note semantics of ADR 0032 (`> g: text` replaces the shared
block note for the card named g, `> g+: text` appends) survive verbatim,
addressed to region `[name]`s instead of hole names. A name addresses a
lone named region's card the same way; the group and its derived
identity begin at the second member (ADR 0034 as amended).

**5. One predicate discriminates blank-derived cards.** Behavior that
means "this is a blank-derived study card" (generate style validation,
depth grading, answer rendering, augment eligibility, direction
expansion) asks `Card::is_blank_card`, never a field of the retired
system.

## Consequences

Easier: one grammar, one identity model, prose that stays readable
everywhere, and a deck format that can stabilize for 0.8.

Harder, and accepted: hole review history resets on conversion (the
stamp is new identity). The formula draw default survives the field it
rode on (amended by ruling, 2026-08-31): a math-classed span defaults
to `input: draw` on its card, and the authored block `input:` reaches
span cards, so an explicit pin wins in either direction.

Deliberately external: converting existing decks is disposable tooling
outside this repository, per the pre-1.0 rule. Overlapping and
grapheme-splitting legacy holes have no span equivalent; the tooling
aborts on them rather than approximating.

## Alternatives considered

**Keep both systems.** Rejected: the cost is permanent (two grammars in
the frozen format), the benefit temporary (no conversion day).

**Translate `\blank` to spans at parse time.** Rejected: that is a
compatibility shim, which pre-1.0 forbids, and it would keep the old
spelling alive in decks indefinitely.

## Compatibility

Pre-1.0, so no old shape is recognized. A deck containing `\blank{...}`
parses as prose; a store document carrying `records` fails to parse; an
id carrying `-N` is invalid input.

## Security

Unchanged. The retirement removes parsing surface and store state; it
adds none.

## Verification

- `\blank{x}` in an answer is literal text and derives no sub-card.
- A span blank satisfies the generator's cloze style and fails plain.
- An unpinned math span reviews as draw; a block `input:` pin reaches
  its span cards.
- Name-addressed notes replace or extend the shared block note for
  the named region's card alone, lone named regions included.
- An old store document with a `records` field fails loud.

## Reversal

Evidence that would justify revisiting: authors measurably refusing the
directive spelling for plain-text cloze. Reversal after 1.0 means a new
inline grammar under a new ADR, never resurrecting `\blank`.
