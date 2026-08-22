# 3 · The deck format

A deck is a plain-text Markdown file. You can write one in any editor with no
tooling, read it back at a glance, and because it's real Markdown, it renders
sensibly anywhere else too: a preview pane, your file host, GitHub.

When you write a deck by hand, initialize it once before it appears in the
picker:

```sh
alix deck init ~/decks/my-deck.md
```

The command adds stable deck and card IDs without rewriting the authored
content. A valid `id: deck-<token>` in the opening frontmatter marks the file as
an initialized deck. The `deck-` prefix on the value is what carries the
meaning: it is how alix tells its own decks apart, and the same prefixed string
travels everywhere the id appears, in a frontmatter key, a card marker, a
filename, a prerequisite reference, or an error message. Markdown without that
prefixed id is never listed or modified, so ordinary documents with `##`
headings can sit in a decks folder untouched. Generated, imported, received, and
tutorial decks are initialized when they are created.

## Choosing a card shape

Read the material before choosing its card shape. The shared guide below names
the useful choices and distinguishes structural matches from judgement calls.
The sections after it show the exact syntax for each shape.

{{#include ../../include/card-shapes.md:guide}}

## Cards

A card starts with `##` at **column 0**, the front (the question). The lines
beneath it are the answer (the back), written plainly, and may span several
lines:

```
## What is the capital of France?
Paris.

## Name the three additive primary colors.
Red
Green
Blue
<!-- reveal: line -->
```

A physical newline inside an ordinary flip answer is a Markdown soft wrap: the
adult and mobile clients display it as a space, so you can wrap long source
lines for editing without creating visual gaps on the card. Add
`<!-- reveal: line -->` when the lines themselves are the learning sequence;
line reveal and line typing preserve them individually.

## Inline formatting

Card fronts, answer lines, and note prose support `**bold**`, `*italic*` or
`_italic_`, and inline `` `code` ``. Inline code is verbatim, so
`` `**literal**` `` displays the asterisks instead of bold text.

Formatting has two projections: styled display and plain content. Grading uses
the plain content, so type `Paris`, not `**Paris**`. To keep emphasis markers
literal, escape them with backslashes such as `2\*3\*4`, or wrap the text in
inline code such as `` `2*3*4` ``. Run `alix doctor <deck>` to find card text
that will render as emphasis.

## LaTeX math

Use `$...$` for a formula inside prose:

```markdown
## Why does $a^2 + b^2 = c^2$ describe a right triangle?
It is the Pythagorean theorem.
```

Use `$$...$$` for display math. The two delimiters and the formula must occupy
one whole logical line; a multi-line `$$` block is not supported:

```markdown
## What is the Gaussian integral?
$$\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}$$
```

An opening dollar must touch the first formula character, and a closing dollar
must touch the last one. A closing inline `$` cannot be followed by a digit, so
`$5 and $10` stays literal currency. Escape a literal dollar as `\$`. Unmatched
dollars and `$$...$$` surrounded by prose also stay literal. Dollars inside
inline code or fenced code are always verbatim.

Graphical clients render recognized math with the shared RaTeX renderer. If the
delimiters are valid but the LaTeX is malformed, the card still loads and shows
the source with a visible "math could not render" message. `alix doctor <deck>`
reports the card line, formula snippet, and renderer error.

Grading uses the content between the delimiters, so type `x^2`, not `$x^2$`.
Adding or removing math delimiters does not change a card token or stale its
cached augmentations. Generated output that otherwise parses as a deck is
checked before placement; malformed math cannot replace an existing deck.

A `##` only starts a card *at column 0 and outside a code fence*. A `##` that is
indented, or sits inside a fenced block, is ordinary answer content, so a Markdown
heading in a sample, a shell comment, or a Dockerfile line needs no escaping:

````
## What does this script print?
```bash
echo hi
## this line is just part of the answer, inside the fence
```
````

## Multi-line fronts

When the question itself spans more than one line, a `---` divider marks where it
ends and the answer begins:

```
## What does `lo` control in this signature?
def bisect_right(a, x, lo=0, hi=None)
---
The lowest index the search considers; entries below `lo` are ignored.
```

Here the front is two lines (the prose question plus the code it's asking about),
and without the `---` alix couldn't tell where the question stops and the answer
starts. (A one-line question needs no divider: the answer just follows on the next
line, as in the cards above.)

## Multiple-choice (checkbox) cards

Write the answer as a GitHub task list to supply your own Recognize options:

```
## Which number is prime?
- [ ] 4
- [x] 5
- [ ] 6
```

The single `[x]` item is the correct answer. Alix shows only that answer at
Recall and expects it at Reconstruct; the `[ ]` items are distractors shown with
it at Recognize. Every option is used, so the card needs no AI `choices`
augmentation and is skipped by that augment target. The Rust core shuffles the
options: their order stays fixed while one question is on screen, then receives
a fresh seed when the card reappears or a new study session starts. As with any
shuffle, two appearances can still produce the same order by chance.

A checkbox card needs exactly one checked item and at least one unchecked item.
Use `-`, `*`, or `+` bullets, with `[x]` or `[X]` for the answer. Put a literal
task list inside a fenced code block to keep it a plain card answer. Task lists
inside notes or a card's front before the `---` divider render as static
checkboxes rather than interactive choices.

## Card tables

Flat material at scale (a vocabulary list, countries and capitals, dates) can
be one Markdown pipe table instead of a `##` block per fact. Each row is a
card: first column front, second column back, optional third column note. The
header row is shown as the card's context, never tested:

```
| word      | meaning   | note                 |
|-----------|-----------|----------------------|
| purported | angeblich | often in legal prose |
| feasible  | machbar   |                      |
```

The table must start at column 0 with a header row and a delimiter row
(alignment colons are fine), exactly like GitHub renders it; every line starts
and ends with `|`. Inside cells, inline formatting and math work as in any
card text. A table inside a fenced code block stays literal text.

Give a table a title by putting a `##` heading directly above it, with
nothing between them but blank lines:

```
## Verbs of arguing
| English   | German      | usage                |
|-----------|-------------|----------------------|
| to refute | widerlegen  | eine These widerlegen |
```

The heading names the group and is shown as the card's first context line,
above the column labels; it is a title only when its body is empty, so a
heading with an answer under it is an ordinary card that happens to be
followed by a table. Directives written on the title line (including the
table's own ID) belong to the table.

At Recognize, a table card's wrong options are drawn from its own column: the
other rows' answers are the distractors, so a table needs no AI `choices`
augmentation and no authored options (though both take precedence if present).
A row only gets a pick when its column offers at least three other distinct
values; smaller tables stay reviewable at the other depths.

That column sampling is on by default. Turn it off for a table whose rows are
not interchangeable (a mixed list, a table of one-off facts) with
`<!-- sampling: off -->` among its directive comments, or set `sampling: off`
in the frontmatter to make that the deck's default and re-enable single tables
with `<!-- sampling: on -->`. A table with sampling off and no other option
source is simply not offered at Recognize, and `alix doctor` reports a
`sampling:` key that can affect nothing.

Identity works like card IDs, per row. `alix deck init` (or opening the deck
for review) mints one container ID line after the table, and a short stamp at
the end of each row, after the closing pipe:

```
| purported | angeblich | often in legal prose | <!-- r:4k2x9w -->
```

Renderers drop cells beyond the header count, so the stamps stay invisible in
a rendered view while keeping the source columns aligned. Both kinds of marker
are machine-maintained, never hand-authored, and they travel with their row,
so sorting, inserting, and editing rows preserves review history.

Directive comments between the table and its ID line (`direction`, `reveal`,
`input`, `sampling`) apply to every row. `direction: both` doubles each row
into a reversed card, which samples its options from the front column.

The format is deliberately narrow: two or three columns only, no cloze
blanks or images inside cells, and nothing but directive comments between a
table and the next card. Anything outside that shape is a parse error rather
than a guess. And a table earns its place at dozens of rows; under roughly
ten cards, plain `##` cards read better and can carry everything a card can.

## Notes

A line beginning with `>` is a **note**: shown *after* you answer, never part of
what's tested. Consecutive `>` lines join into one note:

```
## Why does TCP open with a three-way handshake?
To agree on initial sequence numbers in both directions.
> SYN, SYN-ACK, ACK: each side learns the other's starting sequence.
```

Keep the *answer* to the thing you want to recall, and put the *why*, the example,
or the mnemonic in a note.

A cloze card can give one blank a note of its own, so a note that names an
answer doesn't give it away on the sibling cards. See
[a note for one blank](06-cloze-direction-images.md#a-note-for-one-blank).

## Sections and sub-cards

Heading depth decides a line's role. `##` is a card front, `#` opens a section,
and `###`/`####` are sub-cards.

### `#` — section context

A single-`#` heading opens a **section**. Its text, plus any prose that follows
it outside a card, is the shared context for every card below it until the next
`#`. Prose written before the first heading is section context too.

```
# Ocean depths

Pressure rises by about one atmosphere per ten metres.

## What is the pressure at 30 m?
About 4 atmospheres.
```

Context is not shown while you answer; that would give the card away. Ask for
it: in the web app press `c` and the section replaces the question until you
press `c` again.

A section heading is a heading, nothing more. It takes no directives and no card
ID, and an empty one is an error, because a section owns no card to bind either
to.

### `###` and `####` — sub-cards

A card written one level deeper than the card above it is a **sub-card** of it:
the same card syntax, gated on the parent. A sub-card stays out of review until
its parent has graduated (reached FSRS's review phase, see
[Scheduling](05-scheduling.md)), so a deck can teach the general case first and
release the specialisations once it has stuck.

```
## What does a TCP handshake establish?
An agreed starting sequence number in each direction.

### Why does the client resend SYN if no SYN-ACK arrives?
The SYN may have been lost; nothing else can distinguish that from a slow peer.
```

Depth stacks: `####` hangs off the `###` above it. Two rules are enforced when
the deck is read, so a mis-indented heading fails loudly instead of silently
becoming a top-level card:

- a sub-card needs its parent one level shallower actually open, so a `###`
  with no `##` above it, or a `####` directly under a `##`, is an error;
- nothing goes deeper than `####`.

A `##` closes every open sub-card chain, and a `#` clears the chain entirely.

## Title, and deck-wide settings

A deck's name, its deck-wide settings, and its machine-maintained deck ID all
live in **frontmatter**: a `---`-fenced YAML block at the very top of the file.

The name comes from `title:`. A deck without one falls back to a condensed form
of its `trace:` sentence, and a deck with neither is named by its filename stem.
A `#` heading is never the name: it is [section context](#sections-and-sub-cards)
for the cards below it.

```
---
format-version: 1
id: "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f"
title: French vocabulary, chapter 4
description: The verbs from the chapter's dialogue, plus their prepositions.
authors: [Alex, "Claude (Opus 5)"]
license: CC-BY-4.0
created-at: 2026-07-31
reveal: line
order: sequential
---
```

`description` is a short summary. The web picker shows it when you open a
deck's drawer; nothing else reads it.

`format-version` is the version of the deck *format*, not of the deck itself.
`alix deck init` writes it above `id`, it stays `1`, and alix refuses a deck
declaring any other number rather than guessing at a format it does not know.
It is written first because it says how to read everything below it, but alix
accepts it anywhere in the block.

`authors` takes one value or a list; `title`, `description`, `license`, and
`created-at` are single strings, by convention an SPDX identifier and an ISO
8601 date for the last two. Put both people and any AI that helped in `authors`.
These five are yours to fill in and alix never changes them.

Apart from `id` and `format-version`, frontmatter carries only what differs from
the defaults, and a command-line flag always overrides it. The full set of
frontmatter and per-card keys gets its own *Directives reference* chapter.

## Escaping

Because `##`, `>`, `---`, and the fence and cloze markers are structural, an answer
line that must *start* with one literally is escaped with a leading backslash:
`\##`, `\>`, `\---`. The backslash is consumed; the line displays without it.

```
## How do you write a second-level heading in Markdown?
\## Section title
```

## Why editing a deck is safe

Every initialized deck and card carries a stable identity. `alix deck init`
writes the deck ID as `id: deck-<token>` in frontmatter and each card ID as a
`<!-- id: card-<token> -->` line. If you later add a card to that initialized deck,
opening review or augmentation assigns the missing card ID. Those tokens, not
the text, are what your review history hangs on. You don't type or manage them;
alix adds and maintains them after you explicitly initialize the file.

Because identity is the token and not the words, you can edit **anything** (reword
the question, fix a typo in the answer, rewrite a note, reorder cards) and its
history follows. The only thing that starts a card's
history over is deliberately replacing it. (`alix doctor` warns if an id line goes
missing, for instance if an external tool stripped the HTML comments.)

So a deck is safe to refactor freely: your progress rides on the token, not on the
words.

## Your personal file

A deck you didn't write is still yours to annotate. Anything alix or you add to
someone else's deck goes into a **personal file** beside it, never into the deck
itself: `spanish.md` gets `spanish.personal.md`. The deck file stays
byte-identical, so you can pull an updated copy of it without losing your work,
and your notes never leak back when you share the deck.

It is an ordinary Markdown file with one extra frontmatter key naming the deck
it belongs to:

```
---
format-version: 1
for: deck-9w2c7x4k1m8q3z5t0v6b2n4d8f
---

<!-- note: card-3f7k2m9q1x8w5z0t6v4b2n8d7c -->
> the "cuenta" is the tally you finally add up

## a gap the exam found
the answer
<!-- id: card-5k1m8q3z5t0v6b2n4d8f7c2x9w -->
```

Two kinds of block live there, in any order:

- A **note**: a `<!-- note: <card-id> -->` marker followed by `>` lines. Those
  lines are appended to that card's own note when you review it. If you want a
  label above your note, write a `## ` heading; it is yours, alix never writes
  or rewrites one.
- A **card**, written exactly like a deck card and closed by its own
  `<!-- id: -->` line. It joins the session after the deck's own cards and is
  drilled and scheduled like any other, but it does not count toward the deck's
  card count.

The two machine lines sit at opposite ends of their block on purpose. A card's
id closes it, because the id names the card itself. A note's marker opens it,
because a note is an attachment: it has to say which card it belongs to before
anything below it means anything.

Nothing in this file is a copy of anything in the deck, so nothing here can go
stale. The card id is the only link, and it never changes.

alix writes this file for you: the tutor's **Make this a card**, its
**Make a note**, and the exam's remediation cards all land here. You can also
edit it by hand. Personal files are never listed as decks of their own, and
`alix share` leaves them at home in both directions: a bundle you send never
carries one, and a bundle you receive can't overwrite yours.

A note addressed to a card that no longer exists is left alone rather than
dropped, and `alix doctor` reports it.
