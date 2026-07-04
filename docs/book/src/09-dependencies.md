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
cycle is treated as non-blocking — a broken edge never hides a deck.

## Dependencies don't change what you review

`% requires:` is about *order and gating*, not session contents. When you review
(or browse) a deck, the session holds exactly that deck's cards — prerequisites
are never pulled in, so the `mode`/`order` you study under is always
the deck's own. What dependencies shape is the picker's **dependency tree**
(foundations shown first) and, for a deck with a `% source:`, the **exam gate**
below.

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

The same `% requires:` graph drives **unlocks**, with no extra syntax — and the
gate is the **exam**, not drilling. You can review any deck at any time, in any
order; what `% requires:` controls is **exam order**: a deck with a `% source:`
can't sit its exam until each of its *sourced* prerequisites has passed its own
exam, and passing a foundation's exam unlocks the exams that build on it. A
**source-less** prerequisite has no exam to pass, so it never gates — its edge is
just a suggested order in the tree. (`alix deck check` warns when a sourced deck
requires a source-less one, since that edge can't gate an exam; add a `% source:`
to the prerequisite to make it real.) (A **trace** masters by passing its exam —
retracing the path from memory — so it gates and unlocks like any sourced deck.)

In the picker a deck whose exam is locked shows a 🔒, but it stays **drillable** —
only the exam waits on the prerequisites.

This is what turns a folder of decks into a **curriculum**: order the material by
`% requires:`, and `alix` gates each step's exam on passing the last. It's the
backbone of the AI exam's notion of *mastery* (a later chapter) and of how `alix
explore` lays out a generated learning plan.
