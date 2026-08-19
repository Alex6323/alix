# 6 · Cloze, dual-direction & image cards

Three extensions to the basic card, each a small addition on top of the format
from chapter 3.

## Cloze cards: fill in the blank

A cloze card hides part of the answer; you create one by wrapping the hidden
text in `\blank{...}`.

Wrap any span of an answer in `\blank{...}` and the card becomes a **cloze**: each
`\blank{...}` is a blank, and the card expands into one sub-card per blank. No
directive is needed; the marker itself is the trigger.

```
## Complete the Rust declaration
let \blank{mut} x: \blank{u64} = 0;
```

This makes two cards. One blanks `mut` and shows the rest; the other blanks `u64`.
The asked blank shows as `____`; the *other* blanks are hidden as `[…]`, so no card
gives away its siblings' answers. You only produce the hidden text.

Braces outside a `\blank{}` are ordinary text, so `let p = Foo {};` is fine in a
cloze answer. If you need a literal brace *inside* a `\blank{...}`, escape it as
`\{` or `\}`.

`alix` keeps a card's cloze siblings apart in the queue when other cards are
available, so you don't see `mut` right after `u64`. Editing is safe: identity is
the card's token (chapter 3), so rewording the question, or a hole's text, keeps
your history.

Reach for cloze when the *context* is the cue: a definition with its key term
removed, a line of code with the operative token blanked.

A blank inside `$...$` or `$$...$$` is a piece of the formula, and is treated
as one. It reveals typeset (`$x = -b \blank{\pm} \sqrt{d}$` shows ±, not the
characters `\pm`), and at Reconstruct it is **sketched rather than typed**,
since a formula's piece has no keyboard spelling. Write `input: type` on the
card or the deck to keep the keyboard: an authored `input:` always wins, and
the rule only fills in where you said nothing. `alix doctor` warns when a hole
that stays typed holds a LaTeX command, since `\blank{\pm}` then asks for the
spelling of `\pm` rather than for the sign.

### A note for one blank

A `>` note belongs to the card you wrote, so every blank of it shows the same
note, and a note that spells out one blank's answer gives it away on all the
others. Name the blank you mean and write the note to that name:

```
## The test pyramid, bottom to top
\blank[base]{Unit}, \blank{integration}, \blank{end-to-end}
> base: Fastest and most numerous, which is why they sit at the bottom.
```

Only the `Unit` card shows that line; the other two show nothing. Written as
`> base+: ...` it is added below the shared note instead of replacing it. A
name is one or more of `a-z`, `A-Z`, `0-9`, `_` or `-`, and two blanks of one
card can't share it.

Everything else stays prose. A note line is only an address when the card
names a blank and the name before the `:` is one of them, so a note that opens
`2: the second one` is still a note. `alix doctor` reports an address that
names no blank of its card, and shows the line as an ordinary note.

### Blanks that belong together

Give two blanks the *same* name and they become one card asking both, instead
of two cards each asking half a fact:

```
## The TCP three-way handshake, in order
\blank[open]{SYN}, \blank[open]{SYN-ACK}, \blank{ACK}
```

That is two cards, not three: one asking `SYN` and `SYN-ACK` together, one
asking `ACK`. Both spans show as `____` on the merged card, and you answer them
as a list, one line per span. They don't have to sit next to each other or even
on the same line, so `\blank[c]{Berlin} is the capital of \blank[c]{Germany}`
groups too.

**Grouping starts that card's history over.** Two spans recalled together are a
harder question than either alone, so the merged card takes no schedule from
the blanks it replaces: it comes back as if it were new. The blanks you did
*not* group keep their history, even though grouping renumbered them. Group
early if you are going to.

**A name addresses a blank; it is not its id.** Identity is the card's token
(chapter 3), so renaming a blank, or adding a name to one you have already
drilled, keeps your history exactly as rewording the question does. The name
means nothing outside the card it is written in: two cards may each have a
`base`.

## Dual-direction cards: `direction:`

Reviewing a card *both ways* is what you want for vocabulary and other reversible
facts. Set it per card with `<!-- direction: both -->`, or deck-wide with a
`direction:` line in the frontmatter:

```
## purported
angeblich
<!-- direction: both -->
```

- `both` makes two cards: `purported` → `angeblich` and the swap `angeblich` →
  `purported`.
- `reverse` keeps only the swapped one.
- `forward` (the default) is the card as written.

The two directions get **distinct progress**, are kept apart in the queue, and are
removed together; the reversed card keeps the note. It's best for single-line
cards, and it doesn't apply to cloze cards. When a reversed card's question side
comes from several answer lines, they render as separate centred lines rather than
running together.

## Image cards

Write a standard Markdown image on its own line, and its position decides
the side: an image in the question is a front image, one in the answer is a
back image, and a card can carry more than one per side. An image sharing a
line with prose is rejected: alix displays images as media beside the text,
not inline within a sentence, so a mixed line would silently lose its shape.

A one-line front needs a blank line before the `---` divider to carry an
image (otherwise the divider is just more content, and the image lands on
the back):

```
## What phase is the moon in?
![](moon-waxing.png)

---
Waxing gibbous

## Play this chord:
G major
---
The open-position shape.
![](g-major-tab.png)
```

An image `src` is a path relative to the deck file, exactly the way a standard
Markdown viewer resolves it: a bare filename means the image sits next to the
deck, and `sub/moon.png` means a subdirectory. An absolute path is used as-is.
The brackets can carry alt text: `![the open-position shape](g-major-tab.png)`.

Because the paths are ordinary Markdown, the same deck renders identically in
the web app and in any Markdown viewer that opens the file directly (GitHub,
Obsidian, a plain preview pane). `alix doctor` warns about an image file it
can't find, but doesn't fail on it.

## Source citations

A plain fact card can show *where its answer comes from*. Declare the deck's source
with a `source:` line in the frontmatter, give the card an `<!-- at: ... -->`
locator into it, and on reveal the card offers to swap the worded answer for the
exact source lines:

```
---
source: src/string.rs
---

## What does the `String` struct hold?
A `Vec<u8>` (its bytes).
<!-- at: src/string.rs:1-3 fingerprint: xxh64-0123456789abcdef -->
```

The locator is the same shape a [trace checkpoint](13-trace-decks.md) uses. Its
fields are named and ordered: `at:` is the source path and line range (e.g.
`src/string.rs:1-3`, just `lines` when `source:` is a single file, or a
range-less path or URL to cite the whole source, the form a frozen URL
source uses), and
`fingerprint:` is an `xxh64-<hex>` digest of the displayed source text. alix
writes the fingerprint when it creates a cited card or when you explicitly
repair a hand-authored citation. A fact card may repeat the whole directive when
its answer rests on several disjoint source ranges:

```markdown
<!-- at: src/state.rs:64-74 fingerprint: xxh64-0123456789abcdef -->
<!-- at: src/state.rs:114-118 fingerprint: xxh64-123456789abcdef0 -->
<!-- at: src/state.rs:152-158 fingerprint: xxh64-23456789abcdef01 -->
```

Each locator remains one contiguous range; separate directives never imply
that disjoint code is adjacent. On reveal a `</>` marker appears on the answer:
**click the answer** (or press `s`) to swap it for the same editor-style source
panels used by trace walks, and back. Multiple excerpts are stacked in authored
order inside the one scrollable answer region. For a live citation, alix shows
the lines only when their fingerprint still matches. A moved, changed, deleted,
ambiguous, or unfingerprinted excerpt shows a warning instead of unrelated
lines, without hiding the other citations. Short evidence keeps the answer's
centered vertical alignment; long evidence aligns to the top and scrolls.

This is the same machinery trace walks use to reveal source, brought to ordinary
fact cards. Like every directive, `<!-- at: -->` is not part of a card's identity:
adding a citation never resets its progress.

You rarely write these by hand. Generating a deck from a local source
([`alix generate <path>`](11-generating-decks.md)) cites the lines each fact came
from and fingerprints every citation. Plain
[`alix doctor`](17-command-reference.md) reports missing fingerprints and
fingerprint drift without writing. After reviewing the cited text,
`alix doctor <deck> --repair-source-locators` stamps a missing fingerprint or
rebases a uniquely relocated exact excerpt; changed or ambiguous excerpts
remain untouched for semantic review. Initializing a workspace member goes one
further and **freezes** its source evidence and local images below
`assets/deck-<token>/`, so the deck travels without the original source and the
quotes never shift. Freezing stores only the cited excerpt, never the whole
file, and leaves the deck's `source:` pointed at the real material: each frozen
citation keeps its real path in `at:` and gains an `asset:` field naming the
content-addressed object, so a drift check can still compare against the live
source when it is reachable.
