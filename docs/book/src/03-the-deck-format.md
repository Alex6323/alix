# 3 · The deck format

A deck is a plain-text Markdown file. You can write one in any editor with no
tooling, read it back at a glance, and because it's real Markdown, it renders
sensibly anywhere else too: a preview pane, your file host, GitHub.

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
```

## Inline formatting

Card fronts, answer lines, and note prose support `**bold**`, `*italic*` or
`_italic_`, and inline `` `code` ``. Inline code is verbatim, so
`` `**literal**` `` displays the asterisks instead of bold text.

Formatting has two projections: styled display and plain content. Grading uses
the plain content, so type `Paris`, not `**Paris**`. To keep emphasis markers
literal, escape them with backslashes such as `2\*3\*4`, or wrap the text in
inline code such as `` `2*3*4` ``. Run `alix doctor <deck>` to find card text
that will render as emphasis.

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

## Title, and deck-wide settings

A deck's title is a single-`#` heading. Deck-wide settings live in
**frontmatter**: a `---`-fenced YAML block at the very top of the file, above the
title.

```
---
reveal: line
order: sequential
---

# French vocabulary, chapter 4
```

Frontmatter carries only what differs from the defaults, and a command-line flag
always overrides it. Anything else you write before the first card is just prose
(context, a reading order, whatever you like), so a deck can also read as a normal
document. The full set of frontmatter and per-card keys gets its own *Directives
reference* chapter.

## Escaping

Because `##`, `>`, `---`, and the fence and cloze markers are structural, an answer
line that must *start* with one literally is escaped with a leading backslash:
`\##`, `\>`, `\---`. The backslash is consumed; the line displays without it.

```
## How do you write a second-level heading in Markdown?
\## Section title
```

## Why editing a deck is safe

Every card carries a stable identity: a short token alix writes into the file as an
`<!-- id: ... -->` line the first time it sees the card. That token, not the card's
text, is what your review history hangs on. You don't type or manage those lines;
alix adds and maintains them.

Because identity is the token and not the words, you can edit **anything** (reword
the question, fix a typo in the answer, rewrite a note, reorder cards, even move a
card to another deck) and its history follows. The only thing that starts a card's
history over is deliberately replacing it. (`alix doctor` warns if an id line goes
missing, for instance if an external tool stripped the HTML comments.)

So a deck is safe to refactor freely: your progress rides on the token, not on the
words.
