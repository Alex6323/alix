# 6 · Cloze, dual-direction & image cards

Three extensions to the basic card, each just a small directive on top of the
format from chapter 3.

## Cloze cards — fill in the blank

Mark a front with `#?` (no space) and the card becomes a **cloze**: every `{{...}}`
span in its answer lines is a blank, and the card expands into one sub-card per
blank.

```
#? Complete the Rust declaration
    let {{mut}} x: {{u64}} = 0;
```

This makes two cards. One blanks `mut` and shows the rest; the other blanks `u64`.
The asked blank shows as `____`; the *other* blanks are hidden as `[…]`, so no
card gives away its siblings' answers — you only produce the hidden text.

Only the doubled braces are special: a lone `{` or `}` is literal, so an answer
like `let p = Foo {};` is fine inside a cloze (and if you ever need a literal
`{{`, write `\{\{`).

flash keeps a card's cloze siblings apart in the queue when other cards are
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

Like `mode`, it works per card or deck-wide (a `% direction: both` header with
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

One catch worth knowing: **images render in the web frontend only** — a terminal
can't draw them. So an image card is automatically *web-only* (as if it declared
`% frontend: web`): `flash review` in the terminal skips it with a note, and if a
whole deck is images it points you at `--serve` to open it in the browser. `flash
check` warns about an image file it can't find, but doesn't fail on it.
