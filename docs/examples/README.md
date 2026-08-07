# Example decks

Two sets, answering two different questions.

**`shapes/`** answers *which shape does this material deserve?* One deck
per row of [`../card-shapes.md`](../card-shapes.md), the guide the book
and the deck generator both read. Each deck carries the syntax the guide
deliberately leaves out, and each is held to the shape it advertises by
`every_shape_example_produces_the_shape_it_advertises` in `tests/api.rs`.
A file that parses can still teach the wrong thing.

**`syntax/`** answers *how do I write this at all?* Escaping, fenced
code, math, frontmatter. These make no claim about when to reach for a
shape, so they are checked by `alix doctor` and nothing more.

`workspace-showcase/` is neither: it demonstrates a workspace rather than
a card.

Every deck here is executed by `tests/example_decks.rs`. An example
nothing runs rots into a lie, which is not hypothetical: the math
showcase carried a warning on every build for a while before anyone
looked at it.
