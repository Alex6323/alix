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

Once every card in an exam-grounded deck has been answered right at least
once (a passing review at any depth, Recognize included, or a walk graded
**Got it** or **Partly**) the deck is **exam due**. It stays exam due, also
after passing, until a full sitting masters it, so retaking stays one tap
away; passing alone already unlocks its dependents. A deck with no `exam:` at all (in the deck or its
workspace manifest) simply becomes *finished* when all its cards graduate,
unlocking its dependents directly, even when it declares `cite:` roots.

## Sources

A workspace deck can also hold **sources**: frozen copies of local text
files, each bound to the topics (the deck's `#` heading blocks) it grounds.
They live in `sources/<deck-id>/` at the workspace root, beside a
`manifest.toml` that lists every copy by its id (`source-` plus the file's
SHA-256), its display path (`<key>:<path>`), and its bindings. The exam does
not read sources yet: until it does, `exam:` above names its material.

```
alix deck source add decks/rust.md ../sdk/src/lib.rs --root sdk=../sdk --topic topic-3whz
alix deck source list decks/rust.md
```

- A **key** names a folder of live files on this machine. `--root
  <key>=<dir>` saves it on first use, `root set` and `root unset` change it
  later, and they live in `manifest.local.toml`, which never leaves the
  machine. A file is shown as its path under the root, after the key.
- A **binding** is a topic id, or `<ranges>|<topic>` for some lines only,
  for example `10-80,120|topic-3whz`: 1-based inclusive lines, ascending and
  disjoint. A source is always bound to at least one topic; a deck without
  `#` topics binds its deck id.
- `update` refreezes a source from its file and keeps every binding,
  refusing when a range would lie beyond the new copy's last line;
  `list` shows whether each live file still matches its copy.

A source is refused when it is a URL (save the page as a file), empty, not
UTF-8, outside its root (a symbolic link out of the root included), already
in the deck under another path with the same bytes, or at a path the deck
already holds (refreeze that one with `update` or `alix doctor
--repair-sources`). Every command checks the deck's whole sources folder
first and refuses a damaged one. A change writes new copies first and the
manifest last, so an interruption leaves the old or the new state, and the
next change deletes any copy no row names. Every subcommand and flag is
listed in the [command reference](17-command-reference.md#the-ai-features).

## Sitting the exam

An exam is written ahead, then taken. Writing the questions is a long model
call (several minutes at a high effort), so it runs in the background while
you do something else, and the finished exam waits until you sit it. The
sitting itself is a guided, one-question-at-a-time flow (answer, move
Back/Next, then a per-question breakdown). It is one of a deck's launches,
next to the review depths and Walk, and you can sit it at any time as
practice, before drilling too:

- **In the browser:** the deck drawer's **Generate exam** chip (key `x`)
  starts writing the exam on the server. While it runs, a **Writing exam**
  chip shows the minutes so far and a rough wait, beside a **Stop** chip; a
  short notice says when the exam is ready, and the chip then reads **Take
  exam** (key `x`). The chip is filled when the deck is exam due and its
  prerequisites are passed, and shows a lock while a prerequisite is not
  passed. Choosing an exam-due deck itself drills; it never starts the exam.
- **On the phone:** the deck's launch sheet has an exam row below Walk that
  reads **Generate exam**, then **Preparing** with the rough wait, then
  **Take exam**; the picker says when the exam is ready. The phone sits the
  exam through a [paired](19-pairing.md) desktop, so the row is disabled
  while no desktop is reachable.

**Generate exam** offers **only what needs work**, on by default: the exam
then asks only about the topics that have not passed (those that need
work, have new cards, or were never examined). The control is always shown
and disabled when every topic has passed, since the exam then covers the
whole deck either way.

A prepared exam is kept until it is graded. In the browser it lives in the
deck's store (`.alix/exam/<deck-id>.prepared.json`), so it survives a closed
page or a restarted server, and your answers are saved into it as you give
them. On the phone, the desktop holds the prepared exam in memory and the
phone saves each answer as you move on from it; a restarted desktop loses
the phone's prepared exam, and **End exam** discards it. Either way, an
answer locks once you move on from it, and leaving returns you to the first
unanswered question next time. Grading uses the exam up. In the browser,
editing cards or topic prose leaves a prepared exam valid, a topic whose
heading was renamed or removed since is dropped from it, and changing the
exam material discards it, so the next Generate exam writes a new one.

Writing the exam gives ids to cards added since the deck was last opened,
the way opening a review does, so their results can be recorded.

`alix` asks the model to read the source (URLs via `WebFetch`, local files
embedded) and write **fresh understanding questions** for each of the deck's
`#` topics (cards before the first heading form one topic, named after
the deck). Each topic keeps a list of its **ideas**, the distinct mental
models its topic holds in the source, whether or not a card states them. A
first call writes that list for every topic that has none yet, or for all
of them when the exam material changed. Each exam then picks **three ideas
per topic**: ideas never asked first, then ideas last failed, then ideas
asked longest ago, so repeated exams work through the whole list. A second
call writes exactly one question per picked idea and asks the student to
apply an idea a card states rather than recite the card. A reply that skips
a picked idea, asks one twice, or names an idea or topic that was not
listed is rejected and generated once more. Grading records each asked idea
as passed or failed, which steers the next pick. The browser keeps the idea
lists in `.alix/exam/<deck-id>.json`; the phone keeps its own and sends them
with each exam.

Within a topic the questions cycle through three shapes: a concrete
**instance** no card shows, a **boundary** (when does the idea stop
holding), and a **fix** (a statement with one seeded mistake to find and
correct). The topic's heading, prose and cards go into the prompt only as
scope, so the model knows what the deck already asks and avoids it. A
question stands alone: it shows any program or claim it asks about instead
of pointing at the source, and a program comes as a code block with
numbered lines, so an answer can name "line 3". Questions, feedback and key
points render inline code and math. Each question comes with the key points
a correct answer must hit, shown only with the results. You type a prose
answer per question, and an examiner grades each **Pass / Partial / Fail
against the source's rubric, never against your cards** (grading the cards
would be circular). The model calls run on a background thread, so the UI
stays responsive while it thinks.

## Results: a count, per topic

A sitting reports points, not a verdict: each Pass is worth 1, each Partial
0.5, each Fail 0, shown as `17 / 18` or `17.5 / 18`, with the same count for
each examined topic and the shape of each question. Every examined card
records its topic's latest result:

- A **topic passes** when its share of Pass answers reaches `pass_threshold`
  (a fraction; 1.0, every answer, by default; a Partial counts as no Pass
  here). Each examined card then stores its topic's verdict, so a topic
  reads as *needs work* when any of its cards carries a failed verdict, *has
  new cards* when some cards were added since its last sitting, *passed* when
  every card carries a passing one, and *never examined* when none carries
  one.
- The deck is **passed** the first time every topic has passed, whether in
  one sitting or across several filtered ones. Passing, not mere drilling, is
  what unlocks decks that `requires:` this one, and a deck that has sat its
  exam once is never locked again by its own prerequisites. Source-less decks
  are unaffected: finishing them just means drilled (`done ✓`).
- The deck is **mastered** the first time a full sitting (every topic)
  scores every answer Pass; a filtered sitting never sets it.

Neither passed nor mastered is taken back by a later, weaker sitting. The
results screen and the deck row both name the highest of them with the day
it was reached, and the row adds the last full sitting's day and points:
`passed (2 Nov) · last (4 Nov) 12 / 18`, `mastered (4 Nov) · last (4 Nov)
18 / 18`, or `last (4 Nov) 15 / 18` when examined but not passed. A date
outside the current year names the year (`31 Dec 2025`).

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
share of a topic's answers that must pass). Both, plus `model`, `effort`,
`timeout_secs` (default 3600) and an `extra` guidance field, live in the
`[exam]` config section. `effort` sets how hard the backend thinks while
writing and grading the exam, over `[ask] effort`; a workspace may set its
own in `alix.local.toml`'s `[exam]` section.

## Why this is the centerpiece

Everything else serves this. The drilling loads the facts; the exam is the gate
that turns "I reviewed it" into "I understood it, and here's the check." It's also
why *passing* (not completion) drives [unlocks](09-dependencies.md): a curriculum
should open the next door only when you've genuinely passed through the last. The
everyday, self-graded rehearsal for it is `explain` mode
([chapter 4](04-review-modes.md)); the exam is the real thing.
