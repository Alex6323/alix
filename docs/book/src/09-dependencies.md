# 9 · Dependencies & unlocks

Real subjects have an order: you can't grasp borrowing before ownership, or a
derived rule before its axioms. `alix` lets a deck declare what it builds on, and
uses that both to sequence your study and to gate decks until you're ready.

## Declaring prerequisites: `requires:`

A deck names its prerequisites with a `requires:` list in its frontmatter
(repeatable):

```
---
requires:
  - rust-ownership
  - rust-references
---

## What does the borrow checker prevent?
Aliasing a value while it's mutably borrowed.
```

A name resolves next to the requiring deck or in your decks directory, with or
without the `.md`. Directives aren't card content, so adding or
changing them never touches card progress. A missing prerequisite or a dependency
cycle is treated as non-blocking. A broken edge never hides a deck.

## Dependencies don't change what you review

`requires:` is about *order and gating*, not session contents. When you review
(or browse) a deck, the session holds exactly that deck's cards; prerequisites
are never pulled in, so the `reveal`/`order` you study under is always
the deck's own. What dependencies shape is the picker's **dependency tree**
(foundations shown first) and, for a deck with a `source:`, the **exam gate**
below.

## Unlocks

The same `requires:` graph drives **unlocks**, with no extra syntax, and the
gate is the **exam**, not drilling. You can review any deck at any time, in any
order; what `requires:` controls is **exam order**: a deck with a `source:`
can't sit its exam until each of its *sourced* prerequisites has passed its own
exam, and passing a foundation's exam unlocks the exams that build on it. A
prerequisite with neither `source:` evidence nor a public URL `origin:` has no
exam to pass, so it never gates: its edge is just a suggested order in the
tree. (`alix doctor` warns when an exam-grounded deck requires one without exam
grounding, since that edge can't gate an exam; add evidence or a URL origin to
the prerequisite to make it real. It also flags a **dangling** `requires:`, one
naming a deck that does not exist, so a renamed or deleted prerequisite is caught
rather than silently dropping the edge.) (A **trace** masters by passing its exam
(retracing the path from memory) so it gates and unlocks like any exam-grounded
deck.)

In the picker a deck whose exam is locked shows a 🔒, but it stays **drillable**:
only the exam waits on the prerequisites.

This is what turns a folder of decks into a **curriculum**: order the material by
`requires:`, and `alix` gates each step's exam on passing the last. It's the
backbone of the AI exam's notion of *mastery* (a later chapter) and of how
[`alix generate`](14-explore.md) lays out a generated learning plan.
