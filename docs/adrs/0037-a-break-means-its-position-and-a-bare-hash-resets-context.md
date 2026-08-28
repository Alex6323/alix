# 0037: A break means its position, and a bare `#` resets context

- Status: Accepted (standalone-rule clause superseded by 0038)
- Evidence: ContextResetInCard in src/parser/mod.rs
- Evidence: thematic_break in src/parser/mod.rs
- Recorded: 2026-08-27
- Retrospective: No

Supersedes the section-terminator clause of
[0036](0036-a-heading-is-study-structure-not-deck-metadata.md). The rest of
0036 (section context, sub-cards, frontmatter metadata) stands unchanged.

## Context

0036 froze the section terminator as a blank-surrounded `---`, an amended
ruling that had itself replaced a same-day bare-`#` terminator. That put three
jobs on one spelling: frontmatter fence, front divider, and section terminator.

The cost surfaced when GFM alignment reached thematic breaks. GFM treats
`---`, `***`, and `___` as the same construct. Alix could not: giving the other
two the terminator meaning would let pasted prose containing a rule silently
truncate a section, and withholding it left authors guessing which of three
equivalent spellings alix accepts. Measured before the change, the two families
were not even close to equal. `---` was recognized at exactly three dashes and
carried every structural role; `***` and `___` were recognized at three or more
with spaces, were rejected outright anywhere inside a card, could not divide a
front, and could not be kept literal by `<!-- plain -->`.

Separately, the terminator was a concept with no counterpart in the format. It
had to be taught, it drew its own lint when it terminated nothing, and it made
prose after it a distinct error class.

## Decision

**Position decides what a break means; the spelling never does. A section ends
by opening another one, including an empty one.**

1. **`---`, `***`, and `___` are one construct.** A break attached above a
   non-blank line inside a card divides a front. Blank-surrounded with no card
   open it is a literal rule joining the section context. Blank-surrounded
   inside a card it is a line-numbered error, preserving 0036's reading that
   decoration inside a card is noise. Anything else is a stray-break error. A
   `<!-- plain -->` on the line *below* a break keeps it literal, on all three
   spellings.

2. **A bare `#` opens an empty section context.** A `#` with no title resets:
   cards after it carry no context until the next titled `#`, and a sub-card
   chain does not cross it. Prose under a bare `#` joins that empty section
   exactly as it would under a titled one, so the "prose belongs to no section"
   error class disappears with the terminator that created it.

3. **A bare `#` attached under a card's own lines is an error.** A reset there
   would silently truncate the card. This is the one position where a heading
   is rejected rather than treated as a boundary.

4. **The terminator and its lint are deleted, not carried.** No production or
   diagnostic code recognizes a blank-surrounded break as a terminator.

The asymmetry between headings is principled: `#` is the context level, so an
empty one means an empty context; `##` and deeper are card levels, and a card
with no question is meaningless, so an empty one stays an error.

## Consequences

Decks that used a `---` terminator *between cards* now fail loudly as a
break-inside-a-card error. Decks that used one *before any card* convert
silently instead: the break becomes a literal rule and joins the section, so
the cards after it gain context they did not have. That silent row is the only
meaning-to-different-meaning change here; every other row moves from error to
meaning, which cannot corrupt an existing deck. Exposure was measured at zero
across every deck corpus available to us and the repository examples,
with the detector proven able to fire against known-positive fixtures first.
Conversion is disposable tooling outside this repository, per the pre-1.0 rule.

`***` and `___` gain the front-divider role and the `<!-- plain -->` escape.
An attached break with no card open is now stray for all three spellings, where
previously only `---` was; that reverses a narrow earlier ruling that a `***`
under a heading was ordinary section prose.

A break landing in section context is carried as ordinary text, because the
content-unit vocabulary has no rule variant. It therefore displays as the
literal characters rather than as a horizontal rule. This predates the
decision: the `<!-- plain -->` path already placed `---` into section context
the same way. Adding a rule variant is a wire-contract change and is tracked
separately rather than bundled here.

## Alternatives considered

**Keep the `---` terminator and leave `***`/`___` unequal.** Rejected: it
freezes a three-way overload of one spelling at 1.0, which no later release can
undo, and it leaves the guessing problem permanently.

**Give all three spellings the terminator meaning.** Rejected on the original
0036 reasoning: pasted material containing a rule would silently truncate a
section, which is the failure mode the position rule exists to prevent.

**Warn on a bare `#`.** Rejected by the maintainer: a reset is deliberate
syntax with a visible effect, and a lint on intended syntax trains the reader
to skim doctor output.

**A `PointlessTerminator`-style lint for a reset that resets nothing.**
Rejected for the same reason, after being proposed and withdrawn during review:
the argument against warning on a deliberate bare `#` applies unchanged to a
redundant one.

## Compatibility

Pre-1.0, so no migration and no old-format recognition. `format-version` stays
1. An old deck fails or re-means as ordinary current-design behavior, never a
recognized legacy path.

## Security

No trust boundary changes. The grammar reads the same document text through the
same parser; no new input channel, endpoint, or execution path.

## Verification

A law-shaped table over three spellings and four positions, asserting the
verdict per row and growing by a row rather than a test. Plus the heading
grammar matrix row for the reset, the attached-in-card reset error, prose
joining an emptied section, and a sub-card chain failing to cross a reset. Each
new assertion was mutation-tested: the old behavior reintroduced, the row
watched go red, then reversed.

## Reversal

Evidence that would justify replacing this: authors reaching for a section end
that is not a heading often enough that the bare `#` reads as a workaround, or
the silent before-any-card conversion row proving to bite real decks despite
the measured zero. Reversal restores a distinct terminator token; identity
never entered this grammar, so no progress is at risk in either direction.
