# 6 · Cloze, dual-direction & image cards

Three extensions to the basic card, each just a small directive on top of the
format from chapter 3.

## Cloze cards — fill in the blank

Add `% reveal: cloze` to a card and it becomes a **cloze**: every `{{...}}`
span in its answer lines is a blank, and the card expands into one sub-card per
blank.

```
# Complete the Rust declaration
    % reveal: cloze
    let {{mut}} x: {{u64}} = 0;
```

This makes two cards. One blanks `mut` and shows the rest; the other blanks `u64`.
The asked blank shows as `____`; the *other* blanks are hidden as `[…]`, so no
card gives away its siblings' answers — you only produce the hidden text.

Only the doubled braces are special: a lone `{` or `}` is literal, so an answer
like `let p = Foo {};` is fine inside a cloze (and if you ever need a literal
`{{`, write `\{\{`).

`alix` keeps a card's cloze siblings apart in the queue when other cards are
available, so you don't see `mut` right after `u64`. Cloze progress is forgiving:
rewording the front — or even a later change to the blank markup — keeps your
history, while editing the answer text or what's *inside* a blank resets the
affected blanks.

Reach for cloze when the *context* is the cue: a definition with its key term
removed, a line of code with the operative token blanked.

## Dual-direction cards — `% direction:`

A `% direction:` directive reviews a card *both ways* — exactly what you want for
vocabulary and other reversible facts:

```
# purported
    angeblich
    % direction: both
```

- `both` makes two cards — `purported → angeblich` and the swap `angeblich → purported`.
- `reverse` keeps only the swapped one.
- `forward` (the default) is the card as written.

Like `% reveal:`, it works per card or deck-wide (a `% direction: both` header with
per-card overrides). The two directions get **distinct progress**, are kept apart
in the queue (you won't be shown one right after the other), and are removed
together; the reversed card keeps the note. It's best for single-line cards, and
it doesn't apply to cloze cards.

## Image cards — `% img:`, `% img-back:`

A card can carry an image on the question side, the answer side, or both:

```
% img-dir: ~/decks/img

# What phase is the moon in?
    % img: moon-waxing.png
    Waxing gibbous

# Play this chord.
    G major
    % img-back: g-major-tab.png
```

`% img:` shows on the front, `% img-back:` on the back (revealed with the answer)
— one image per side. Filenames resolve against the deck's `% img-dir:` (a
header-only directive, absolute or relative to the deck file); without one they
resolve next to the deck file, and an absolute path written on the card is used
as-is.

One catch worth knowing: images render in the web app. `alix deck check` warns
about an image file it can't find, but doesn't fail on it.

## Source citations

A plain fact card can show *where its answer comes from*. Give the card a `% at:`
locator into the deck's `% source:`, and on reveal it offers to swap the worded
answer for the exact source lines:

```text
% source: src/string.rs

# What does the `String` struct hold?
    A `Vec<u8>` (its bytes).
    % at: src/string.rs:1-3
```

The locator is the same shape a [trace checkpoint](13-trace-decks.md) uses:
`file:lines` (e.g. `src/string.rs:1-3`), or just `lines` when the `% source:` is
a single file. On reveal a `</>` marker appears on the answer —
**click the answer** (or press `s`) to flip it to the line-numbered excerpt and
back. The lines are read *live* from the source,
so a moved or deleted file shows "source unavailable" rather than a stale quote.

This is the same machinery trace walks use to reveal source, brought to ordinary
fact cards — so a card that asks *what* a thing is can also show you *where* it
lives. Like all `%` directives, `% at:` is invisible to the identity hash: adding
a citation to an existing card never resets its progress.

You rarely have to write these by hand. Generating a deck from a local source —
[`alix deck generate <path>`](11-generating-decks.md) or
[`alix explore --build`](14-explore.md) — cites the lines each fact came from,
and [`alix deck check`](17-command-reference.md) warns about a citation that no
longer resolves, so a moved or shrunk file is caught before you next review the
card. A workspace built with `alix explore --into --build` goes one further and
**freezes** the cited excerpts into its `assets/` (just like trace excerpts), so
the workspace travels without the original source and the quotes never shift under
you. It also records where they came from in an `% origin:` directive — so the
tutor can still reach the live source for context, and
[`alix deck check`](17-command-reference.md) can flag a frozen card whose source has
since **drifted**.
