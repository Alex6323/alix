# 5 · Scheduling, stages & completion

Spaced repetition is really just bookkeeping: each card remembers how well you
know it and when to show it next. This chapter is that bookkeeping — the
scheduler, the stages, and how a whole deck reaches "done."

Pick a scheduler with `--scheduler` or a deck's `% scheduler:` directive (it's
deck-level only).

## Leitner *(default)*

A box system. Each card sits at a **stage**, and each stage has a cooldown before
the card comes due again:

| Stage | 1 | 2 | 3 | 4 | 5 |
| --- | --- | --- | --- | --- | --- |
| Cooldown | now | 1 hour | 6 hours | 1 day | 1 week |

Grading moves the card between stages:

- **again** (fail) → back to stage 1
- **good** (pass) → up one stage
- **easy** → up two stages

So a card you keep getting right climbs to longer and longer intervals, and a miss
sends it back to the bottom of the ladder. It's predictable and needs no tuning —
a good default.

## SM-2

SuperMemo-2 spacing, with a per-card **ease factor**. Passing grows the interval
(roughly 1 day, then 6 days, then `interval × ease`); the ease nudges up or down
with each grade and never drops below 1.3; and a fail sends the card to a short
10-minute relearn. It adapts the spacing to each card's difficulty instead of
using fixed steps. Switching schedulers is safe: SM-2 seeds itself from your
existing Leitner progress and keeps the Leitner stage in sync, so you can move
between the two without losing your place.

## Retiring cards

A card doesn't climb forever. Once it reaches the **top stage** (5) by passing, it
**retires**: it rests and is no longer scheduled, *not even under `--cram`*, until
you `alix reset`. A deck is *finished* once all its cards have retired.

## Completion states

A deck's **state** is derived from its cards' stages, and shown in the picker and
`alix stats`:

- **not started** — you haven't reviewed any card yet
- **started** — somewhere in between
- **finished** (`done ✓`) — every card has retired at the top stage

A deck that declares a `% source:` adds one state in between — **exam due**. For
those decks, drilling the cards no longer finishes them: passing the **AI exam**
does, which marks the deck *mastered*. That's the subject of a later chapter.

## Unlocks, in one line

Completion also drives dependencies, with no extra syntax: a deck is **locked**
while any deck it `% requires:` isn't finished, so finishing a foundation unlocks
what builds on it. The lock is advisory and recomputed live — the dependencies
chapter covers it in full.

## Cramming

Need to review everything now, schedule be damned — the night before an exam?
`--cram` ignores cooldowns and shows every card that isn't retired:

```sh
alix --cram mydeck.txt
```

Retired cards stay out (that's what retirement is for); everything else is fair
game regardless of when it's next due.
