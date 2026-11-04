# 12 · The AI exam

This is the feature the whole tool is built around. Drilling cards *loads* a
deck's material into memory; the **AI exam** checks that you actually *understood*
it, and passing the exam, not merely finishing the cards, is what marks a deck
done and unlocks what depends on it.

The reasoning: recall isn't understanding. You can drill every card and still not
see how the ideas connect. So a deck can name a ground-truth **source** and require
you to pass an exam *against that source* before it counts.

## Declaring the exam material: `exam:`

Name one or more sources in the deck's frontmatter, each a URL or a file. A
directory is not exam material: the exam refuses to start on one, and
`alix doctor` reports any `exam:` file it cannot read:

```
---
exam:
  - https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
  - notes/ownership.md
---
```

A URL `exam:` doubles as a [tutor](10-tutor.md) reference, so you
needn't repeat it as a `link:`. The reverse doesn't hold: a `link:` stays
tutor-only and never becomes exam ground truth: keep supplementary reading (a
blog post, an SO answer) as `link:` so the exam ignores it.

The exam reads only `exam:` entries (the deck's own and its workspace
manifest's). It never reads `cite:` roots or the text your cards cite: to
examine a cited file, list it under `exam:` as well.

Once every card in an exam-grounded deck has **graduated** (reached FSRS's
review phase, past the initial learning steps) the deck is **exam due** rather
than finished: drilled, but not yet counted, so it doesn't unlock its
dependents yet. A deck with no `exam:` at all (in the deck or its
workspace manifest) simply becomes *finished* when all its cards graduate,
unlocking its dependents directly, even when it declares `cite:` roots.

## Sitting the exam

The exam is a guided, one-question-at-a-time flow (answer, move Back/Next, then a
per-question breakdown). It is one of a deck's launches, next to the review
depths and Walk, and you can sit it at any time as practice, before drilling
too:

- **In the browser:** the deck drawer's **Take exam** chip (key `x`). It is
  filled when the deck is exam due and its prerequisites are passed, and shows
  a lock while a prerequisite is not passed. Choosing an exam-due deck itself
  drills; it never starts the exam.
- **On the phone:** the deck's launch sheet has an **Exam** row below Walk. The
  phone sits the exam through a [paired](19-pairing.md) desktop, so the row is
  disabled while no desktop is reachable.

Both offer **only what needs work**, on by default: the sitting then asks
only about the sections that have not passed (those that need work, have new
cards, or were never examined). The browser shows the tick only when some sections passed
and some did not, since otherwise the sitting covers the whole deck either
way; the phone always shows the switch and disables it when every section
has passed.

Starting the exam gives ids to cards added since the deck was last opened,
the way opening a review does, so their results can be recorded.

`alix` asks the model to read the source (URLs via `WebFetch`, local files embedded)
and write **fresh understanding questions** for each of the deck's `#`
sections (cards before the first heading form one section, named after the
deck). A first call lists each section's mental models from the source, the
distinct ideas its topic holds, whether or not a card states them; a second
call writes at least one question per listed model, with no upper limit, and
asks the student to apply a model a card states rather than recite the card.
A reply that skips a listed model, or names a model or section that was not
listed, is rejected and generated once more. Within a section the
questions cycle through three shapes: a concrete
**instance** no card shows, a **boundary** (when does the idea stop holding),
and a **fix** (a statement with one seeded mistake to find and correct). The
section's heading, prose and cards go into the prompt only as scope, so the
model knows what the deck already asks and avoids it. A question stands alone:
it shows any program or claim it asks about instead of pointing at the source,
and a program comes as a code block with numbered lines, so an answer can name
"line 3". Questions, feedback and key points render inline code and math. Each
question comes with the key points a correct answer must hit, shown only with
the results. You type a
prose answer per question, and an examiner grades each **Pass / Partial / Fail
against the source's rubric, never against your cards** (grading the cards would
be circular). The model calls run on a background thread, so the UI stays
responsive while it thinks.

## Results: a count, per section

A sitting reports points, not a verdict: each Pass is worth 1, each Partial
0.5, each Fail 0, shown as `17 / 18` or `17.5 / 18`, with the same count for
each examined section and the shape of each question. Every examined card
records its section's latest result:

- A **section passes** when its share of Pass answers reaches `pass_threshold`
  (a fraction; 1.0, every answer, by default; a Partial counts as no Pass
  here). Each examined card then stores its section's verdict, so a section
  reads as *needs work* when any of its cards carries a failed verdict, *has
  new cards* when some cards were added since its last sitting, *passed* when
  every card carries a passing one, and *never examined* when none carries
  one.
- The deck is **passed** the first time every section has passed, whether in
  one sitting or across several filtered ones. Passing, not mere drilling, is
  what unlocks decks that `requires:` this one, and a deck that has sat its
  exam once is never locked again by its own prerequisites. Source-less decks
  are unaffected: finishing them just means drilled (`done ✓`).
- The deck is **mastered** the first time a full sitting (every section)
  scores every answer Pass; a filtered sitting never sets it. The results
  screen says "mastered" only for such a sitting.

Neither passed nor mastered is taken back by a later, weaker sitting. The
deck row shows the highest of them with the last full sitting's count
(`15 / 18` when examined but not passed, `passed · 15 / 18`, `mastered 🎉 ·
3d · 17 / 18`, where `3d` is the time since it was mastered) and how many
sections need work or have new cards since the exam.

Whenever an answer missed key points, the results offer to turn the gaps into
**remediation cards**: a cloze card or a plain card for a missed fact, an open
understanding card (a prompt plus key points) for a missed concept, with
overlapping gaps merged. Re-drill those and re-sit. Once created, the screen
reports how many remediation cards it added.

Those remediation cards land in the deck's
[personal file](03-the-deck-format.md#your-personal-file) (`<deck>.local.md`),
never in the authored deck. They start as new cards: the drill introduces them,
then FSRS schedules them like any other card. Once scheduled they count toward
the deck's *due* total but not toward its card count. Regenerating the same gap won't
duplicate it; once a card's interval reaches the retirement cap it's archived,
and re-failing the gap brings it back. Because they are plain Markdown blocks
in a file you own, you can read, edit, or delete them yourself.

Resetting a whole deck (`alix reset <deck>`) also clears its exam state, so a
re-drilled deck must pass again; `alix reset <deck> --exam` clears only the
exam state (each card's result, passed, mastered and the last count) and
names the decks whose exams it will lock. Resetting only an individual card
(`--card`) leaves the deck's passed and mastered state intact.

## Strictness: match the rigor to the material

How hard each answer is judged fits the *material*: a checklist topic (a procedure,
exact syntax, a security drill) should fail you for omitting a step; a conceptual topic
shouldn't. It's a learner setting, not per deck: the `[exam]` `strictness` config default,
optionally overridden per workspace in `alix.toml`'s `[defaults]`. The levels:

- **strict**: completeness required: every rubric point must be present, so
  omitting one is a gap.
- **balanced** *(default)*: judges *understanding*, not phrasing: a point counts
  if your answer shows you grasp it, even briefly; only a wrong or genuinely-absent
  idea is a gap.
- **lenient**: benefit of the doubt: only clearly wrong or unanswered points are
  gaps.

This dial (how hard each answer is judged) is independent of `pass_threshold` (the
share of a section's answers that must pass). Both, plus `model`, `timeout_secs` (default 300),
an `extra` guidance field, live in
the `[exam]` config section.

## Why this is the centerpiece

Everything else serves this. The drilling loads the facts; the exam is the gate
that turns "I reviewed it" into "I understood it, and here's the check." It's also
why *passing* (not completion) drives [unlocks](09-dependencies.md): a curriculum
should open the next door only when you've genuinely passed through the last. The
everyday, self-graded rehearsal for it is `explain` mode
([chapter 4](04-review-modes.md)); the exam is the real thing.
