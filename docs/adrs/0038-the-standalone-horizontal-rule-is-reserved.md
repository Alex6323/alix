# 0038: The standalone horizontal rule is reserved

- Status: Accepted
- Evidence: StrayDivider in src/parser/mod.rs
- Recorded: 2026-08-28
- Clarified 2026-08-28: the decision said "a multi-line front", which is
  narrower than the parser it describes. A break directly under a card's
  heading divides a one-line front too
  (`an_attached_divider_right_under_the_heading_divides`). The decision is
  unchanged; only the wording was wrong.
- Retrospective: No

Supersedes the standalone-rule clause of
[0037](0037-a-break-means-its-position-and-a-bare-hash-resets-context.md). The
rest of 0037 (one lexical class over `-`, `*`, `_`; the bare `#` reset; the
deleted section terminator) stands unchanged.

## Context

0037 gave a blank-surrounded break with no card open one meaning: a literal
rule joining the section context. It did not render as a rule. The unit
vocabulary has no rule variant, so the line travelled as ordinary text and
displayed as the characters an author typed, and 0037 recorded the wire change
as tracked separately.

Building it priced the construct. Three different section lines arrive at a
client as the identical string `---`: a real break, a `\---` escape, and a
break kept literal by `<!-- plain -->`. No client-side shape test can separate
them, which is why the web page's own detector was already wrong in the other
direction (it accepted `*` and `_` and not `-`, so it drew a rule for two
spellings of one construct and text for the third). Correctness therefore
requires the wire to say, per line, which one it is. `section_context` is a
`Vec<String>` that the review DTO, the tutor, and the exam prompts all read, so
carrying that distinction means threading a parallel index list through the
scan state, `RawBlock`, `RawCard`, and `Card`, publishing a new `kind` tag, and
teaching three client renderers to use it.

Against that cost stands no demonstrated use. Section context is authored under
a `#` heading and is meant to be a short orienting line, not a document with
sections of its own. Measured over every deck file available to us, the shared
study workspace, the repository examples, and the end-to-end fixtures, a
blank-surrounded break occurs zero times. A wider sweep including ordinary Markdown that alix never
parses found matches only in vendored dependency READMEs.

## Decision

**Dividing a card's front from the answer attached below it is the only
meaning a thematic break has. Every other position is a line-numbered error,
blank-surrounded outside a card included. alix has no standalone horizontal
rule, and the shape is reserved.**

1. **One error for every non-dividing break.** `StrayDivider` names the whole
   class, at any spelling and in any position. `CardThematicBreak`, which
   covered only the blank-surrounded-inside-a-card position, is deleted along
   with the section position it used to contrast against.

2. **The literal escapes are unchanged.** A `<!-- plain -->` on the line below
   a break keeps it literal at every spelling, and the backslash escape still
   covers the dash spellings. An author who wants the characters still writes
   them; only the construct is withheld.

3. **`ContentUnit` gains no rule variant, and no client detects a rule from a
   line.** The page's `isRuleLine` and its `hr.context-rule` styling are
   deleted rather than corrected, because no line reaching a client is a rule
   any more.

## Consequences

A deck that carried a rule in its section context now fails to load with a
line-numbered error instead of displaying literal characters. Exposure is the
measured zero above; the fix is deleting the line or marking it `<!-- plain -->`.

The break grammar collapses to one sentence a reader can hold: a break divides
a front, everywhere else it is an error. Three error classes over four
positions become one class over one position, and the manual loses a paragraph
of position bookkeeping.

The web page stops carrying a Markdown predicate. That predicate was the last
place a client re-derived block grammar from a line, and its removal is why the
divergence cannot silently return.

Adding the construct later is additive: it needs the unit variant and the
per-line wire signal described above, and nothing shipped now blocks it.

## Alternatives considered

**`ContentUnit::Rule { at }`, carrying the index of the line it replaces.**
The design that would have worked. Rejected on cost against a use nobody has:
the plumbing spans the parser scan state, three card structures, a published
wire tag, and three client renderers, all to draw a line inside a context
short enough that it should not need one.

**A positional `Rule` unit with no index, spliced into the stream like a
fence.** Rejected as unsound rather than expensive. The client must know which
lines consume a unit, so it would re-detect break shape in JavaScript; any
drift between that predicate and the Rust one misaligns every unit after it,
which is a worse failure than the one being fixed and is exactly the
client-side grammar the decision removes.

**Keep 0037's literal-rule row.** Rejected: it renders `---` as characters to
an author who meant a rule, and it leaves three distinct constructs sharing one
string on the wire with nothing able to tell them apart.

## Compatibility

Pre-1.0, so no migration and no old-format recognition. `format-version` stays
1. A deck holding a standalone rule fails as ordinary invalid input.

## Security

No trust boundary changes. Same parser, same document text, no new input
channel or execution path.

## Verification

The position law table over five spellings asserts the front-divider row and
an error row for each of the other positions, and grows by a row rather than a
test. The parser arm was mutation-tested: the section-literal path was
reintroduced, the law row watched go red, then reversed. The corpus harness
baselines were regenerated and reviewed for movement; every changed entry is a
message rewrite, no example moved between parsing and failing.

## Reversal

Evidence that would justify replacing this: authors asking for a rule inside
section context, or section context growing long enough that a reader needs
one. Reversal is additive and needs no migration, since the shape is an error
today rather than a meaning that would have to change.
