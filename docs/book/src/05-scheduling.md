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

## Topological order *(experimental)*

By default your due cards come up in scheduler order — for Leitner, the ones you
know least first. That's right for *retention*, but it can feel random: a card
about parsing, then one about persistence, with no thread between them.

A **topology** gives the session a thread. `alix deck augment <deck> --target
topology` asks Claude to read the deck and lay out a *graph* of how the cards
relate — a suggested **walk** through them, plus a few coarse named **regions**
(stages or themes). It's cached beside your progress like distractors and notes;
a deck can hold several, one per `--with` principle:

```sh
alix deck augment internals.txt --target topology
alix deck augment capitals.txt --target topology --with "north to south"
alix deck augment capitals.txt --target topology --with "by continent"
```

Then review along it:

```sh
alix review internals.txt --topology auto    # or any cached principle's name
```

The key thing: this changes only the **order**, never the schedule. SRS still
decides *which* cards are due and how they advance — the topology just serves
that due set in walk order instead of shuffled, so each card is a natural
follow-up to the last. Not-due cards are skipped, so the session stays as short
as your due pile.

As you go, a thin **region breadcrumb** sits above each card — e.g.
`Ingestion · Review Engine · Persistence · Frontends`, the one you're in
emphasized — so you see *where you are* in the material, not just what's in front
of you. The names are deliberately coarse: they orient without giving away any
card's answer.

If a deck has exactly one cached topology, `--topology` with no name uses it.
This is experimental — both the terminal and the web show the breadcrumb and the
ordering; richer map views are still to come.
