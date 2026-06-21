# 3 · The deck format

A deck is a plain-text file, and the format is deliberately small — you can write
one in any editor with no tooling, and read it back at a glance.

## Cards

A card starts with a `#` at **column 0** — the front (the question). The indented
lines beneath it are the answer (the back), and may span several lines:

```
# What is the capital of France?
    Paris.

# Name the three additive primary colors.
    Red
    Green
    Blue
```

A `#` only starts a new card *at column 0*. An **indented** `#` is ordinary
answer content — so shell comments, Rust attributes, or Dockerfile lines need no
escaping:

```
# What does this script print?
    echo hi
    # this # is just part of the answer
```

## Notes

A line beginning with `!` is a **note** — shown *after* you answer, never part of
what's tested:

```
# Why does TCP open with a three-way handshake?
    To agree on initial sequence numbers in both directions.
    ! SYN, SYN-ACK, ACK — each side learns the other's starting sequence.
```

Notes render as a quoted block, one sentence per line. A fenced ` ``` ` code
block inside a note is shown verbatim — indentation preserved, not reflowed — so
code stays readable. Use notes generously: keep the *answer* to the thing you
want to recall, and put the *why*, the example, or the mnemonic in the note.

## Comments

A line beginning with `%` is a **comment** — ignored, unless it's one of the
directives covered in later chapters. Use plain `%` lines to annotate a deck:

```
% Chapter 4 vocabulary. Reviewed weekly.
```

## Escaping

Lines are **trimmed**, so you never have to type indentation — it's purely for
readability. Because `#`, `!`, and `%` are markers, an answer line that must
*start* with one is escaped with a leading backslash:

```
# How do you start a comment in Python?
    \# like this
```

The backslash is consumed; the line displays as `# like this`.

## Why editing a deck is safe

A card's identity is derived from its **answer lines** (together with the deck's
name) — *not* from its question, its notes, or its comments. Two consequences are
worth knowing:

- You can freely edit comments, notes, and directives, reorder cards, and even
  **reword a question** without losing any review history — none of those feed
  the identity.
- Changing an **answer line** makes a new card with fresh progress. Two cards
  with identical answers in the same deck collide; `flash check` warns about
  those.

So a deck is safe to refactor: rephrase, annotate, reorder, add directives — your
progress rides on the answers.

## Deck directives, in one line

A deck can set its own defaults — the answer mode, the scheduler, and more — with
`% key: value` lines in the **header** (before the first card):

```
% mode: typing
% scheduler: sm2
```

Because they're just comments, directives don't affect card identity, and an
explicit command-line flag always overrides them. The full set gets its own
*Directives reference* chapter; the next two chapters cover the ones you'll reach
for first — the answer **modes** and **scheduling**.
