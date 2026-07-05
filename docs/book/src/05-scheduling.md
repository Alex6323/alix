# 5 · Scheduling, retirement & completion

Spaced repetition is really just bookkeeping: each card remembers how well you
know it and when to show it next. This chapter is that bookkeeping — the
scheduler, retirement, and how a whole deck reaches "done."

## FSRS

alix schedules with **FSRS** — the Free Spaced Repetition Scheduler (FSRS-5, via
the `rs-fsrs` crate). There's one scheduler and nothing to choose: FSRS keeps a
small memory model per card (its *stability* and *difficulty*) and, from your
grade, works out when the card is next due.

Grading feeds FSRS a rating:

- **failed** → *Again* — a lapse; the card comes back soon and its interval shrinks
- **partly** → *Hard* — a weak success; a shorter next interval than a clean pass
- **passed** → *Good* — the interval grows

So a card you keep getting right stretches to longer and longer intervals, a miss
pulls it back in, and a **partly** — you got the gist but stumbled — lands in
between. Early on the first successful reviews are minutes-to-hours apart (FSRS's
short-term *learning* steps); a card **graduates** into the review phase — where
intervals grow to days, then weeks — only after **two** spaced correct recalls,
and missing it resets that progress, so a slip doesn't shortcut it.

A session shows each due card once. Miss one and it returns **spaced** — after its
short step, interleaved behind other cards — not drilled again the instant you saw
the answer (which would test your working memory, not your recall). When nothing is
due right now the session ends; a card still cooling is picked up the next session,
or slots back in on its own if you leave the window open.

One knob shapes the whole schedule: **`retention`** — the recall probability FSRS
aims for (0.70–0.99, default 0.9). Raise it to see cards more often, lower it to
stretch the gaps. Set it in the `[review]` config section, or per workspace in an
`alix.local.toml` (see [Configuration](16-configuration.md)).

### New cards: an attempt before they're tested

A card you've never seen isn't quizzed cold — you can't reconstruct what you've
never read — but it isn't simply handed to you either. The first encounter is a
**low-stakes attempt**, then the answer, then one key (**Seen**) records it
*without a grade*. By default it's **recall**: the front shows first, you
try, then reveal. If the deck has AI distractors (`alix deck augment --target
choices`), an **atomic** card instead greets you as a **multiple-choice** question —
pick one, see which was right. Either way a guess never promotes or punishes, and
the first *graded* quiz then comes back **later in the same session** — once a
short (~1-minute) settle passes it resurfaces, interleaved behind the other cards
you're seeing, so seeing a deck flows straight into drilling it. Each session introduces up to `--new N` new cards (default 10); start
another session for more. This is the first step of a card's life — *acquire*,
then let FSRS space it.

## Retiring cards

A card doesn't stay in rotation forever. Once its interval grows past
**`retire_after`** (default one year), the card **retires**: it rests and is no
longer scheduled, *not even under `--cram`*, until you `alix reset` it. Set
`retire_after = "never"` to keep drilling a deck forever — facts you never want to
risk forgetting; a workspace can override it in its `alix.local.toml`.

## Completion states

A deck's **state** is derived from how far its cards have progressed, and shown in
the picker and `alix stats`:

- **not started** — you haven't reviewed any card yet
- **started** — somewhere in between
- **finished** (`done ✓`) — every card has **graduated** (reached FSRS's review
  phase, past the initial learning steps)

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
`--cram` ignores due times and shows every card that isn't retired:

```sh
alix --cram mydeck.txt
```

Cramming is a **refresh, not a reward**: a correct answer re-anchors the card by
its current interval — it doesn't grow the schedule or count as a real review — so
a heavy cram session won't distort your long-term spacing. A card you *miss* under
cram still lapses normally. Retired cards stay out (that's what retirement is for).

## Topological order *(experimental)*

By default your due cards come up in scheduler order — soonest-due first. That's
right for *retention*, but it can feel random: a card
about parsing, then one about persistence, with no thread between them.

A **topology** gives the session a thread. `alix deck augment <deck> --target
topology` asks the model to read the deck and lay out a *graph* of how the cards
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
