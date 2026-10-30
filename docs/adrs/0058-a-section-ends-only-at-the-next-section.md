# 0058: A section ends only at the next section

- Status: Accepted
- Recorded: 2026-10-30
- Retrospective: No
- Evidence: fn every_empty_section_heading_is_an_error_wherever_it_sits in src/parser/mod.rs
- Evidence: fn an_empty_card_heading_and_an_empty_table_front_cell_name_no_section in src/parser/mod.rs
- Evidence: fn a_sub_card_chain_does_not_cross_a_section in src/parser/mod.rs
- Amends: 0037 (rules 2 and 3)

## Context

ADR 0037 rule 2 let a `#` with no title end a section: the cards after it
carried no section context until the next titled `#`. Rule 3 made that
heading an error when it sat attached under a card's lines.

The reset is a second way to shape context that a deck author has to
learn, and it renders as an empty heading in any other Markdown viewer.
Its one use is a group of cards that should have no context at all. A
card under an unrelated or thin section loses little, so ending a
section is left to the author: put such cards above the first `#`, or
give them a section of their own.

## Decision

1. **A card's section context is the nearest titled `#` above it.**
   Nothing ends a section except the next titled `#`. Cards above the
   first `#` have no section context.
2. **A depth-1 heading whose text is empty is an error**, wherever it
   sits: "section heading is empty; write its title". Empty is measured
   after the heading's trailing comments and closing hashes are stripped,
   so `#`, `# ##` and `# <!-- an aside -->` are all empty.
3. **An empty card-table front cell has its own error**, "table row has
   an empty front cell", since the card-heading wording ("write the
   question on the heading line") does not fit a table row. Empty card
   headings at depths 2 to 6 keep their error.

## Consequences

Easier: one rule for where a card's context comes from; no heading that
means something by being empty.

Harder, and accepted: a deck that used an empty `#` to drop context fails
to parse until the line is removed or titled. There is no way to end a
section; a card that should carry none goes above the first `#`.

## Alternatives considered

**Keep the empty `#` as a reset.** Rejected: a second construct for a
case the maintainer judged an authoring choice, kept alive by accident.

**A dedicated `<!-- end-context -->` marker.** Rejected for the same
reason; it was designed and dropped before being built.

## Compatibility

The deck format changes: an empty `#` heading stops parsing. Before 1.0
there is no compatibility path; the error names an empty heading and
nothing else.

## Security

None changed.

## Verification

The three tests in the Evidence lines: the seven empty spellings in three
positions, the two neighbouring errors' wording, and a titled `#` still
closing a sub-card chain.

## Reversal

Evidence that authors need context-free cards after a section in real
decks would justify a construct for it; it would be a new syntax, since
an empty heading must stay an error.
