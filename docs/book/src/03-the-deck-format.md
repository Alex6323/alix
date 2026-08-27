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
`_italic_`, `~~strikethrough~~` (the double-tilde pair exactly; a single or
triple tilde run stays ordinary text), and inline `` `code` ``. Inline code is
verbatim, so
`` `**literal**` `` displays the asterisks instead of bold text.

Formatting has two projections: styled display and plain content. Grading uses
the plain content, so type `Paris`, not `**Paris**`. To keep emphasis markers
literal, escape them with backslashes such as `2\*3\*4`, or wrap the text in
inline code such as `` `2*3*4` ``. Run `alix doctor <deck>` to find card text
that will render as emphasis.

Inline escaping follows the CommonMark rule: a backslash before any ASCII
punctuation character yields that literal character, and a backslash before
anything else (letters, digits, spaces) stays a literal backslash. That is
why `\blank{...}` needs no escaping to survive.

## Links

A complete `[label](destination)` displays as just the label, styled as a
link; the brackets, parentheses, and destination never reach the screen,
and the deck file keeps what you wrote. Destinations are inert on study
surfaces: a card is for recalling, not browsing. Every destination form
displays the same way, including relative paths and `#anchor` fragments
(alix has no heading anchors, deliberately: a card's identity survives
editing its text, so nothing may address a card by its words).

Emphasis works inside a label and across a whole link. Grading compares
the label text, so type `see the docs`, not the syntax. An incomplete
pattern such as `[brackets]` alone stays ordinary prose, an escaped
`\[label](d)` stays literal, and inline code or a fence keeps the whole
syntax verbatim. Autolinks (`<https://...>`) are covered under "HTML in
a deck".

A link-definition line (`[label]: destination`) in an answer is deck
metadata, not content: it never displays and is never graded, and a card
whose whole answer is definitions fails loud as answerless. Reference
links resolve against the deck's own definitions and render like inline
links, in all three GFM forms: `[text][label]`, `[text][]`, and bare
`[text]`. Labels match case-insensitively with interior whitespace
collapsed, a label never resolves across deck boundaries, and a
reference whose label is undefined stays ordinary prose.

## LaTeX math

Use `$...$` for a formula inside prose:

```markdown
## Why does $a^2 + b^2 = c^2$ describe a right triangle?
It is the Pythagorean theorem.
```

Use `$$...$$` for display math, either on one whole logical line or as a
block: a line holding only `$$` opens the block, the next such line closes
it, and everything between is one formula. A ```` ```math ```` fence carries
its body the same way. All three render identically:

```markdown
## What is the Gaussian integral?
$$\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}$$

## State the quadratic formula.
$$
x = \frac{-b \pm \sqrt{b^2 - 4ac}}
         {2a}
$$
```

A card that opens a `$$` block without closing it fails to load, with an
error naming the opener's line. A closed pair in section or context prose
renders as the same display formula. An unmatched opener on those surfaces
stays ordinary text and does not consume the lines after it.

An opening dollar must touch the first formula character, and a closing dollar
must touch the last one. A closing inline `$` cannot be followed by a digit, so
`$5 and $10` stays literal currency. GitHub's backtick-anchored spelling also
works: ``$`x^2`$`` renders exactly like `$x^2$`, with the backtick-quoted body
as the verbatim formula. Escape a literal dollar as `\$`. Unmatched
dollars and `$$...$$` surrounded by prose also stay literal. Dollars inside
inline code or fenced code (other than a `math` fence) are always verbatim.

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

A fence closes only on a delimiter of its own character at least as long as
its opener (the CommonMark closing-length rule); that is what lets the
four-backtick fence above contain the three-backtick one.

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

The divider's shape is strict, so a stray break fails loudly instead of silently
joining a card: a front divider sits directly above its answer, with a blank line
(or the card's own heading) above it, once per card. `---`, `***`, and `___` are
interchangeable: the position decides what a break means, never which of the
three you typed. A break anywhere else is either a literal rule in
[section context](#-section-context) or a parse error; a literal break line is
written `\---` (see [Escaping](#escaping)), and a `<!-- plain -->` on the line
*below* a break also keeps it literal.

A break alone with blank lines on both sides has no meaning inside a card and is
an error: move the material to a section, or delete the line.

## Choice cards (task lists)

A bare GitHub task list is a literal checklist: mappings are opt-in. Name the
mapping on the line below the list to make it a choice card:

```
## Which number is prime?
- [ ] 4
- [x] 5
- [ ] 6
<!-- choices-single -->
```

Under `choices-single` the one `[x]` item is the correct answer. Alix shows
only that answer at Recall and expects it at Reconstruct; the `[ ]` items are
distractors shown with it at Recognize. Every option is used, so the card
needs no AI `choices` augmentation and is skipped by that augment target. The
Rust core shuffles the options: their order stays fixed while one question is
on screen, then receives a fresh seed when the card reappears or a new study
session starts. As with any shuffle, two appearances can still produce the
same order by chance.

`choices-single` demands exactly one checked item and at least one unchecked
item; any other shape fails loudly. Use `-`, `*`, or `+` bullets, with `[x]`
or `[X]` for the answer. Task lists inside notes or a card's front before the
`---` divider render as static checkboxes rather than interactive choices.

`choices-multiple` is select-all-that-apply: every `[x]` is a correct option
and the reviewer picks all of them. A `choices-multiple` list that checks
exactly one item is legal: a one-answer select-all is a fair question when
the learner must discover how many options are correct.

A deck built of choice cards declares the mapping once in frontmatter instead
of once per card: `tasklist: choices-single` (or `choices-multiple`). A
per-card invocation overrides the deck default, and `<!-- plain -->` on the
line below one task list keeps that one literal.

## Card tables

Flat material at scale (a vocabulary list, countries and capitals, dates) can
be one Markdown pipe table instead of a `##` block per fact. A bare pipe
table renders as a real aligned table: the delimiter row's alignment colons
set each column's alignment, short rows pad with empty cells, and long rows
truncate to the header width. It never becomes cards on its own;
`<!-- cards -->` on the line below maps it (or `table:
cards` in frontmatter maps every table in the deck, with `<!-- plain -->`
below one table keeping it literal). Each row is a card: first column front,
second column back, optional third column note. The header row is shown as
the card's context, never tested:

```
| word      | meaning   | note                 |
|-----------|-----------|----------------------|
| purported | angeblich | often in legal prose |
| feasible  | machbar   |                      |
<!-- cards -->
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

## Notes and quotes

A blockquote whose first line is one of GitHub's five alert badges is a
**note**: shown *after* you answer, never part of what's tested. The badge sits
alone on that line, and the `>` lines under it are the note's body:

```
## Why does TCP open with a three-way handshake?
To agree on initial sequence numbers in both directions.
> [!NOTE]
> SYN, SYN-ACK, ACK: each side learns the other's starting sequence.
```

The five are `[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, and
`[!CAUTION]`, spelled exactly as GitHub spells them. Each opens its own
callout with a chip naming it, coloured from whatever theme you are using
rather than from GitHub's palette. Several notes on one card stack in the
order you wrote them. Spelling them GitHub's way is the point: a deck pasted
from a repository keeps its meaning without editing.

Every **other** blockquote is a quote, and a quote is content. It belongs to
the answer and reveals with it, so you can finally put someone's actual words
on a card:

```
## What did Dijkstra say about testing?
That it shows the presence of bugs, never their absence.
> Program testing can be used to show the presence of bugs, but never
> to show their absence.
```

A quotation is one block however many `>` lines you wrapped it across, and it
reads as one: a rule down its left edge marks it off, and the `>` markers are
gone. Under
`<!-- reveal: line -->` it takes a single reveal rather than arriving a marker
line at a time, and at Reconstruct you are never asked to type it: typing
someone else's words back tests transcription, not understanding, so you type
the answer's own prose while the quotation stands beside it. A card whose whole
answer is a quotation has nothing to type, so Reconstruct asks you to explain
it instead.

A badge alix does not recognize, a badge in the wrong case, or a badge with
text after it on the same line is a quote, exactly as on GitHub. That is a
quiet change of meaning, so `alix doctor` names it rather than leaving you to
find it. A badge whose body is empty, or only blank `>` lines, draws a warning
too and shows nothing.

A note trails its card the way a directive comment does, so a `<!-- reveal:
line -->` may stand above one, and answer content may not come after one. Prose
or a quotation below a note fails loud pointing at the badge line: move the
content above the note.

Keep the *answer* to the thing you want to recall, and put the *why*, the example,
or the mnemonic in a note.

A cloze card can give one blank a note of its own, so a note that names an
answer doesn't give it away on the sibling cards. See
[a note for one blank](06-cloze-direction-images.md#a-note-for-one-blank).

## Sections and sub-cards

Heading depth decides a line's role. `##` is a card front, `#` opens a section,
and `###` through `######` are sub-cards.

### `#` — section context

A single-`#` heading opens a **section**. Its text, plus any prose that follows
it outside a card, is the shared context for every card below it until the next
`#`.

A deck body starts with a heading. Before the first one there is no section to
belong to and no card to be part of, so a line there is an error rather than
text that lands nowhere. A deck of plain cards needs no section at all: opening
with `##` is fine, and those cards simply have no context. Whatever a deck is
about goes in `title:` and `description:`.

```
# Ocean depths

Pressure rises by about one atmosphere per ten metres.

## What is the pressure at 30 m?
About 4 atmospheres.
```

The adult web app keeps section context behind the compact `§ c` control below
the question. Press `c` to replace the answer area with that context while the
question stays in place; press it again to return to the answer. The kids client
does not expose section context.

A section heading is a heading, nothing more. It takes no directives and no card
ID, because a section owns no card to bind either to.

A section runs to the next `#` heading. To end one without opening another,
write a **bare `#`**: a `#` with no title opens an *empty* context, so the cards
after it carry none until the next titled `#` (a sub-card chain does not cross it
either). Prose under a bare `#` belongs to the new, empty section, exactly as it
would under a titled one.

A bare `#` needs a blank line above it, like any other block. Attached directly
under a card's own lines it is an error, since a reset there would silently
truncate the card.

### `###` to `######` — sub-cards

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

Depth stacks: `####` hangs off the `###` above it, down to `######` at the
deepest, matching ATX's own ceiling. One rule is enforced when the deck is
read, so a mis-indented heading fails loudly instead of silently becoming a
top-level card: a sub-card needs its parent one level shallower actually
open, so a `###` with no `##` above it, or a `####` directly under a `##`,
is an error. Seven or more hashes are not a heading at all and stay ordinary
answer text, as CommonMark reads them.

A `##` closes every open sub-card chain, and a `#` clears the chain entirely.

## Reserved Markdown shapes

Four Markdown spellings have no alix meaning and fail loudly with the line
number and a suggested rewrite, instead of showing their markers to the
learner: a setext `===` underline directly beneath a prose line (write an
ATX `#`-prefixed heading), four-space/tab indented code opening after a
blank line or a heading (wrap the code in a ``` fence; ordinary paragraph
and task-list continuation lines are unaffected), a nested `> >` quote
(notes are flat, one `>` deep; put literal `>` text in a fence), and a
blank-surrounded thematic break inside a card, whichever of `---`, `***`, or
`___` you type (delete it, or move the material to a section, where the break
renders as a horizontal rule in the section view). Inside fenced code every
shape is literal, as always. A trailing-two-space hard break is not a spelling
at all: content lines shed trailing whitespace when the deck is read.

## HTML in a deck

alix renders Markdown, never HTML, so a tag shape is reserved rather than
silently shown as text: a `<` directly followed by a letter, or `</`, on any
deck surface fails to load, naming the line, the column, and the two outs
(wrap literal markup in backticks, or escape a lone bracket as `\<`).
Ordinary brackets are unaffected: `a < b`, `a<3`, and `<1>` are plain text.

Three HTML spellings do render, each with a fixed meaning:

- **Autolinks.** `<https://alix.study>` and `<user@host>` display as the
  bracket-free URL styled as a link, with no navigation attached: the deck
  is a study surface, not a browser.
- **The styled subset.** `<sub>…</sub>`, `<sup>…</sup>`, and `<ins>…</ins>`
  display as subscript, superscript, and underline on every client, tags
  dropped. One element opens at a time on a line, and each pair must close
  in order; a mismatched, doubled, or unclosed pair is a tag-shape error.
  Grading compares the inner text, so type `H2O` for `H<sub>2</sub>O`. A
  dropped tag still separates what stood on either side of it, so
  `x<sup>2</sup>_i_` italicizes instead of reading as one word.
- **Entities.** The full HTML5 named set (`&amp;`, `&euro;`, …) and the
  numeric forms `&#65;` / `&#x41;` decode to their characters on display,
  per CommonMark. Anything that is not a complete, valid entity stays
  literal. A decoded character is content, never markup: `&#42;x&#42;`
  shows `*x*` without italics, and `&lt;div&gt;` shows `<div>` without the
  tag-shape error. Markup beside an entity is judged on the spelling in
  the file, where the neighbouring character is `&` or `;`, so
  `&Aopf;_x_` italicizes `x`. Grading compares the decoded text, and the
  deck file keeps the entity exactly as you wrote it.

The usual protections apply on top: inline code, fenced blocks, and math
bodies keep every one of these spellings literal, a whole-line `<!-- -->`
comment protects its interior, and a complete image destination in angle
brackets (`![diagram](<a b.png>)`) is an address, not a tag.

## Title, and deck-wide settings

A deck's name, its deck-wide settings, and its machine-maintained deck ID all
live in **frontmatter**: a `---`-fenced YAML block at the very top of the file.

The name comes from `title:`. A deck without one falls back to a condensed form
of its `trace:` sentence, and a deck with neither is named by its filename stem.
A `#` heading is never the name: it is [section context](#sections-and-sub-cards)
for the cards below it.

```
---
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

`format-version` is a reserved key: the version of the deck *format*, not of
the deck itself. A deck that does not declare it is format version 1, so alix
never writes the key. Declaring `format-version: 1` by hand is accepted; alix
refuses a deck declaring any other number rather than guessing at a format it
does not know.

`authors` takes one value or a list; `title`, `description`, `license`, and
`created-at` are single strings, by convention an SPDX identifier and an ISO
8601 date for the last two. Put both people and any AI that helped in `authors`.
These five are yours to fill in and alix never changes them.

Apart from `id`, frontmatter carries only what differs from the defaults, and a command-line flag always overrides it. The full set of
frontmatter and per-card keys gets its own *Directives reference* chapter.

Key order never matters and yours is never diagnosed. Frontmatter alix itself
writes follows one canonical order: authored keys first (`title` and
`description` up front), machine lines like `id` last. To rewrite an existing
deck into that order, opt in with `alix doctor <deck>
--repair-frontmatter-order`. The same applies to a card's trailing comment
machinery (any order parses, the id last is canonical):
`--repair-comment-order` rewrites each machinery run into the canonical
order without touching content. A run holds recognized machinery only, so an
invocation reads down through the card's own directive comments to the block
above them, and an editorial comment or an unknown key ends the run: the
invocation below one maps nothing and fails loud. One invocation consumes the
block it maps, so a second below the first fails the same way.

## Escaping

Because `##`, `>`, `---`, and the fence and cloze markers are structural, an answer
line that must *start* with one literally is escaped with a leading backslash:
`\##`, `\>`, `\---`. The backslash is consumed; the line displays without it.
For a line that is exactly `---`, `<!-- plain -->` on the line below keeps it as
content too, as a literal thematic break.

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
