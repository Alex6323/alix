# 9 · Dependencies & unlocks

Real subjects have an order: you can't grasp borrowing before ownership, or a
derived rule before its axioms. `alix` lets a deck declare what it builds on, and
uses that both to sequence your study and to gate decks until you're ready.

## Declaring prerequisites — `% requires:`

A deck names its prerequisites with `% requires:` lines in the header
(repeatable):

```
% requires: rust-ownership
% requires: rust-references

# What does the borrow checker prevent?
    Aliasing a value while it's mutably borrowed.
```

A name resolves next to the requiring deck or in your decks directory, with or
without the `.txt`. Like all directives these are plain comments, so adding or
changing them never touches card progress. A missing prerequisite or a dependency
cycle is reported as an error.

## Foundations first

When you review a deck, its prerequisites are pulled into the session
**automatically** — transitively and de-duplicated — and ordered **foundations
first**: a prerequisite's cards (both due reviews and freshly introduced ones)
come before the cards of the deck that depends on it, while each deck keeps its
normal scheduler order internally. So reviewing `borrowing` quietly refreshes
`ownership` first.

A prerequisite contributes only its **cards**, not its settings — the `mode`,
`order`, and `scheduler` come from the deck you actually asked to review, so
requiring a `% mode: line` lyrics deck won't switch your session into line mode.
(Dependencies apply to `review` only; `browse` and `stats` work on exactly the
decks you name.)

## Editing without typos — `alix deps`

To change a deck's prerequisites without hand-editing, use `alix deps <deck>`
(alias `alix require`):

```sh
alix deps borrowing.txt
```

It opens the deck picker over your decks directory, pre-ticked to the current
prerequisites: `Space` toggles, `Enter` saves (rewriting the `% requires:` lines),
`Esc` cancels, and unticking everything clears them. Because the lines are
comments, editing dependencies never disturbs card progress.

## Unlocks

The same `% requires:` graph drives **unlocks**, with no extra syntax. A deck is
**locked** while any of its prerequisites isn't *finished* — every card retired,
the completion state from [chapter 5](05-scheduling.md). Finish a foundation and
the decks that build on it open up; locked decks show dimmed with a 🔒 in the
picker.

The lock is **advisory**, not a wall: you can still pick a locked deck, and its
prerequisite cards are pulled in foundations-first anyway, so you're never truly
stuck. And it's recomputed **live** — if a finished deck later lapses below the
top stage, its dependents lock again, nudging you to shore up the foundation
before moving on.

This is what turns a folder of decks into a **curriculum**: order the material by
`% requires:`, and `alix` walks you through it foundations-first, gating each step
on the last. It's also the backbone of the AI exam's notion of *mastery* (a later
chapter) and of how `alix explore` lays out a generated learning plan.
