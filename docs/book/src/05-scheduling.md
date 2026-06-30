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
| Cooldown | ~5 min | 1 hour | 6 hours | 1 day | 1 week |

Grading moves the card between stages:

- **failed** → back to stage 1
- **partly** → down one stage (floored at 1)
- **passed** → up one stage

So a card you keep getting right climbs to longer and longer intervals, a miss
sends it back to the bottom of the ladder, and a **partly** — you got the gist but
stumbled — only steps it back one rung, so you keep most of your progress but see
it sooner. It's predictable and needs no tuning — a good default.

Stage 1's cooldown is a short **relearn/settle gap** (~5 minutes): a card that just
failed — or a brand-new one you've just met (see below) — comes due a few minutes
later rather than instantly, so a new session started right away won't re-test
something you only just saw. Within a session a failed card still comes back the
same run, as soon as the queue cycles round to it.

### New cards: an attempt before they're tested

A card you've never seen isn't quizzed cold — you can't reconstruct what you've
never read — but it isn't simply handed to you either. The first encounter is a
**low-stakes attempt**, then the answer, then one key (**Seen**) records it at
stage 1 *without a grade*. By default it's **recall**: the front shows first, you
try, then reveal. If the deck has AI distractors (`alix deck augment --target
choices`), an **atomic** card instead greets you as a **multiple-choice** question —
pick one, see which was right. Either way a guess never promotes or punishes
(stage 1 regardless), and the first *graded* quiz comes a **later session**, once
the ~5-minute settle has passed. Each session introduces up to `--new N` new cards
(default 10); start another session for more. This is the first step of a card's
life — *acquire*, then drill it up the ladder.

## SM-2

SuperMemo-2 spacing, with a per-card **ease factor**. Passing grows the interval
(roughly 1 day, then 6 days, then `interval × ease`); the ease nudges up or down
with each grade and never drops below 1.3; a **partly** keeps the card's
repetition count but halves its next interval (the SM-2 twin of Leitner's
one-stage demotion); and a **failed** sends the card to a short 10-minute relearn.
It adapts the spacing to each card's difficulty instead of
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

Completion also drives dependencies, with no extra syntax: a deck's **exam** is
locked while any of its *sourced* prerequisites hasn't passed its own exam — the
deck itself stays drillable throughout. Passing a foundation's exam unlocks the
exams that build on it. The lock is advisory and recomputed live — the
dependencies chapter covers it in full.

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
card's answer. Under each region is a **strength heatmap**: one small bar per
card, red (weak) → green (learned), so a region visibly greens up as you master
it — the breadcrumb doubles as a progress map.

To **drill one weak region** on its own, name it:

```sh
alix review internals.txt --topology auto --region Persistence
```

SRS still chooses what's due *within* that region — you've just narrowed the
session to it.

If a deck has exactly one cached topology, `--topology` with no name uses it. In
the **web picker** you don't type any of this: select a deck that has a topology
and an inline **focus drawer** opens beneath it — choose which topology orders
the session and tap a region's heatmap to scope the launch to it, then start.
The choice is made *before* the session; the in-card breadcrumb itself stays
read-only. This is experimental — both surfaces show the breadcrumb, the heatmap,
and the ordering; richer map views are still to come.
