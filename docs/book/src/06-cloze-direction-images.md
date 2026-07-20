# 6 · Cloze, dual-direction & image cards

Three extensions to the basic card, each a small addition on top of the format
from chapter 3.

## Cloze cards: fill in the blank

Wrap any span of an answer in `\cloze{...}` and the card becomes a **cloze**: each
`\cloze{...}` is a blank, and the card expands into one sub-card per blank. No
directive is needed; the marker itself is the trigger.

```
## Complete the Rust declaration
let \cloze{mut} x: \cloze{u64} = 0;
```

This makes two cards. One blanks `mut` and shows the rest; the other blanks `u64`.
The asked blank shows as `____`; the *other* blanks are hidden as `[…]`, so no card
gives away its siblings' answers. You only produce the hidden text.

Braces outside a `\cloze{}` are ordinary text, so `let p = Foo {};` is fine in a
cloze answer. If you need a literal brace *inside* a `\cloze{...}`, escape it as
`\{` or `\}`.

`alix` keeps a card's cloze siblings apart in the queue when other cards are
available, so you don't see `mut` right after `u64`. Editing is safe: identity is
the card's token (chapter 3), so rewording the question, or a hole's text, keeps
your history.

Reach for cloze when the *context* is the cue: a definition with its key term
removed, a line of code with the operative token blanked.

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

Write a standard Markdown image where you want one to appear, and its
position decides the side: an image in the question is a front image, one
in the answer is a back image, and a card can carry more than one per side.

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
<!-- at: src/string.rs:1-3 -->
```

The locator is the same shape a [trace checkpoint](13-trace-decks.md) uses:
`file:lines` (e.g. `src/string.rs:1-3`), or just `lines` when `source:` is a single
file. On reveal a `</>` marker appears on the answer: **click the answer** (or press
`s`) to flip it to the line-numbered excerpt and back. The lines are read *live*
from the source, so a moved or deleted file shows "source unavailable" rather than a
stale quote.

This is the same machinery trace walks use to reveal source, brought to ordinary
fact cards. Like every directive, `<!-- at: -->` is not part of a card's identity:
adding a citation never resets its progress.

You rarely write these by hand. Generating a deck from a local source
([`alix generate <path>`](11-generating-decks.md)) cites the lines each fact came
from, and [`alix doctor`](17-command-reference.md) warns about a citation that no
longer resolves. A workspace built with `alix generate` goes one further and
**freezes** the cited excerpts into its `assets/`, so the workspace travels without
the original source and the quotes never shift. It also records where they came
from in an `<!-- origin: ... -->` directive, so the tutor can still reach the live
source, and `alix doctor` can flag a frozen card whose source has since **drifted**.
