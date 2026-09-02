# 0041: An undecided construct degrades to a default, and doctor warns

- Status: Accepted
- Evidence: UndecidedTable in src/parser/mod.rs
- Recorded: 2026-09-01
- Retrospective: No

## Context

Some content shapes carry more than one plausible semantics in a deck. A
pipe table can be a literal display table or a card table whose rows are
cards; a future list unit could be answer prose or an ordered reveal
sequence. The deck format resolves such shapes by explicit invocation
(`<!-- cards -->`, `<!-- plain -->`, deck-wide `table:` in frontmatter),
and a shape with no invocation renders in its literal form.

> Note (2026-09-02): the deck-wide `table:` key has since been removed from
> the format; a table is decided by its trailing invocation alone, and the
> default-and-warn behavior below is unconditional.

That opt-in model has one silent failure: an author who pastes a
fifty-row vocabulary table and forgets the invocation gets a single
explain-this-table card instead of fifty drills, and nothing tells them.
The failure is invisible exactly when the intent was the richer
semantics, and no plausibility heuristic can separate the forgotten
invocation from the deliberate display table.

## Decision

Every content shape that can carry more than one semantics follows one
contract:

1. an explicit, extensible meta-line vocabulary names each semantics;
2. a shape with no meta-line degrades to a safe default (for tables:
   plain literal rendering), never to a guess;
3. doctor warns on the undecided shape until the author attaches a
   meta-line, and every vocabulary member silences the warning, the
   default included.

Tables are the first instance: `cards` and `plain` are the vocabulary,
plain is the default, and `LintKind::UndecidedTable` is the warning.
Runtime behavior never depends on the warning; parsing is unchanged.

## Consequences

- The author decides; the tool never infers intent from shape. A
  deliberate display table costs one `<!-- plain -->` line, which also
  documents the intent for the next reader.
- The vocabulary grows additively: a new mapping name is a new directive
  value, not a new subsystem, and adding one cannot change existing
  decks' meaning.
- A future multi-semantic construct (the roadmap list unit, or any
  later addition) inherits the same contract instead of re-litigating
  the silent-failure question.
- A deck built entirely of display tables carries one meta-line per
  table; a deck-wide default spelling for the literal side would be an
  additive extension if that tax proves real.
