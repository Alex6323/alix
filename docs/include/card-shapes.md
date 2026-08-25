# Card shapes: the shared authoring guide

This file is consumed twice, and in neither place as a file:

- compiled into the deck generator's prompt
  (`src/generate.rs`, `card_shape_guide()`);
- included in the manual under that chapter's own heading
  (`docs/book/src/03-the-deck-format.md`).

Only the content between the `ANCHOR` lines reaches either consumer; this
preamble exists for a human opening the file, which is why the anchored content
itself starts headingless and mid-thought. Editing the anchored content edits
the generator's prompt: deterministic tests prove it still *reaches* the
prompt, and the costed `make shape-eval` gate is what proves it still
*steers*.

<!-- ANCHOR: guide -->
Which card shape suits which material. Read the material first, then
pick the shape; do not pick a shape and bend the material into it.

Some rows are **structural**: the material has a property the shape
exploits, and any other shape wastes it. Some are **judgement**: more
than one shape is defensible and the choice is yours. The difference is
marked, because a rule that claims uniform authority gets followed
badly exactly where thought was needed.

| material | shape | kind | why |
| --- | --- | --- | --- |
| Paired items: a word and its meaning, a term and its definition, a symbol and its name. | A card table: a GitHub pipe table with `<!-- cards -->` on the line below it (or `table: cards` in frontmatter), one row per pair, columns front, back, and an optional note. | structural | One row per pair, and each row's Recognize options come from its own column, so the wrong answers are real siblings and cost no AI call. Prose wastes both. |
| Ordered steps that must be reproduced in order: a recipe, an algorithm, a procedure, a verse. | `reveal: line`, with one step per answer line. | structural | Order is graded, and the answer uncovers one line at a time so recall is stepwise rather than all-or-nothing. A flip card cannot test order at all. |
| An answer that cannot be typed: a diagram, a circuit, a glyph, notation. | `input: draw` | structural | The learner sketches and self-grades against the reveal. Typing a diagram is not a check, it is a workaround. |
| A statement turning on one term, where the sentence around it is the cue. | Cloze: wrap the hidden span as `\blank{...}` in an answer line. | judgement | The context does the cueing, so recall is anchored where it will be used. If nothing in the sentence is a natural target, this is a plain card wearing a disguise. |
| A fact whose common confusions are known and nameable. | Authored choice: a task list with one `- [x]` and one or more `- [ ]`, closed by `<!-- choices-single -->` on the line below it, ideally two or three (or `tasklist: choices-single` in frontmatter, once for the deck). | judgement | The distractors are the teaching. Write them only if you can say what mistaken belief each one represents; if you cannot, the shape is doing nothing. |
| A term that must be recalled from either side: vocabulary, symbols, names. | `direction: both` | judgement | One authoring act, two cards. Reach for it when both directions are genuinely useful, not by default: it doubles the review load. |
| Anything else: a definition, an explanation, a cause, a comparison. | A plain card: `## ` front, answer lines below. | judgement | The default, and not a failure. Most material has no structure to exploit, and a plain card drilled well beats a clever shape drilled badly. |

Rules that hold whatever the shape:

- Every card needs at least one answer line.
- Most cards deserve a `> [!NOTE]` note: an example, a caveat, a mnemonic,
  or why it matters. Never a restatement of the answer. The badge line is
  required; a blockquote without one is a quotation and belongs to the answer.
- A choice card's note names the option's claim or mistaken premise, never
  its number, letter, or screen position. Options shuffle between appearances.
- Every `<!-- ... -->` line trails its card: it goes below the card's
  last content line. Nothing belonging to the card may follow a
  directive, so never place one above an answer line, an option list,
  or a table.
- One idea per card. Split compound facts rather than nesting them.
- No two cards may test the same fact. Vary what is asked; do not
  rephrase.

`order:` and `sampling:` are not shapes. They modify how an existing
deck is served and are documented with the other directives.
<!-- ANCHOR_END: guide -->
