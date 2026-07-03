# 12 · The AI exam — `alix exam`

This is the feature the whole tool is built around. Drilling cards *loads* a
deck's material into memory; the **AI exam** checks that you actually *understood*
it — and passing the exam, not merely finishing the cards, is what marks a deck
done and unlocks what depends on it.

The reasoning: recall isn't understanding. You can drill every card and still not
see how the ideas connect. So a deck can name a ground-truth **source** and require
you to pass an exam *against that source* before it counts.

## Declaring a source — `% source:`

Name one or more sources in the deck header — a URL or a local file path,
repeatable:

```
% source: https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
% source: notes/ownership.md
```

A URL `% source:` doubles as a [tutor](10-tutor.md) reference, so you
needn't repeat it as a `% link:`. The reverse doesn't hold: a `% link:` stays
tutor-only and never becomes exam ground truth — keep supplementary reading (a
blog post, an SO answer) as `% link:` so the exam ignores it.

Once every card in a sourced deck reaches the top stage, the deck is **exam due**
rather than finished — drilled, but not yet counted, so it doesn't unlock its
dependents yet. To open the exam earlier, while the cards keep drilling, set
`% unlock-stage: N`: the deck turns exam due once every card reaches stage `N`
(a source-less deck becomes *finished* at that stage instead, unlocking its
dependents directly).

## Sitting the exam

The exam is a guided, one-question-at-a-time flow — answer, move Back/Next, then a
per-question breakdown — identical in the terminal and the browser. You reach it
three ways:

- **Directly:** `alix exam ownership.txt` (with `--questions 8`, `--strictness …`
  if you like).
- **From the picker:** choosing an `exam due` deck starts the exam instead of an
  empty review.
- **From the summary:** when you drill a deck's last cards and it turns exam due,
  the session-end summary offers it — press `x` in the terminal (or `b` to browse
  the deck instead), a button in the browser.

`alix` asks the model to read the source (URLs via `WebFetch`, local files embedded)
and write **fresh understanding questions** — application and connections, *not*
the card facts — each with the key points a correct answer must hit. You type a
prose answer per question, and an examiner grades each **Pass / Partial / Fail
against the source's rubric, never against your cards** (grading the cards would
be circular). The model calls run on a background thread, so the UI stays
responsive while it thinks.

- **Pass** (every question by default — tune with `pass_threshold`) marks the deck
  **mastered** (`mastered ✓`). Mastery, not mere drilling, is what unlocks decks
  that `% requires:` this one. Source-less decks are unaffected: finishing them
  just means drilled (`done ✓`).
- **Fail** lists the gaps and offers to turn them into **remediation cards**
  appended to the deck — a cloze/plain card for a missed fact, a `% mode: explain`
  card for a missed concept, with overlapping gaps merged. Re-drill those and
  re-sit.

A **trace** deck is examined differently: instead of generated questions, its exam
asks you to *retrace the whole path from memory* in a sentence or two — **the
compression** — graded holistically against the checkpoints (no question
generation, no source read). Passing masters the trace; a fail sends you back to
**re-walk** it. See [trace decks](13-trace-decks.md) for the full flow.

Resetting a whole deck (`alix reset <deck>`) also clears its mastered state, so a
re-drilled deck must pass again; resetting only individual cards (`--card` /
`--cards`) leaves mastery intact.

## Strictness — match the rigor to the material

How hard each answer is judged is a property of the *material*, so it's per deck. A
checklist topic — a procedure, exact syntax, a security drill — should fail you for
omitting a step; a conceptual topic shouldn't. Set it with a `% strictness:`
header directive (or `alix exam --strictness …`, or the `[exam]` default):

- **strict** — completeness required: every rubric point must be present, so
  omitting one is a gap.
- **balanced** *(default)* — judges *understanding*, not phrasing: a point counts
  if your answer shows you grasp it, even briefly; only a wrong or genuinely-absent
  idea is a gap.
- **lenient** — benefit of the doubt: only clearly wrong or unanswered points are
  gaps.

This dial (how hard each answer is judged) is independent of `pass_threshold` (how
*many* answers must pass). Both, plus `model`, `timeout_secs` (default 300),
`num_questions` (default 5), and an `extra` guidance field, live in the `[exam]`
config section.

## Why this is the centerpiece

Everything else serves this. The drilling loads the facts; the exam is the gate
that turns "I reviewed it" into "I understood it, and here's the check." It's also
why *mastery* — not completion — drives [unlocks](09-dependencies.md): a curriculum
should open the next door only when you've genuinely passed through the last. The
everyday, self-graded rehearsal for it is `explain` mode
([chapter 4](04-review-modes.md)); the exam is the real thing.
