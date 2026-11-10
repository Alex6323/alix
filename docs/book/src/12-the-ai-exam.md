# 12 · The AI exam

This is the feature the whole tool is built around. Drilling cards *loads* a
deck's material into memory; the **AI exam** checks that you actually *understood*
it, and passing the exam, not merely finishing the cards, is what marks a deck
done and unlocks what depends on it.

The reasoning: recall isn't understanding. You can drill every card and still not
see how the ideas connect. So a deck can name a ground-truth **source** and require
you to pass an exam *against that source* before it counts.

## What the exam reads

The exam reads a [workspace](08-workspaces.md) deck's **sources** (below):
frozen copies of local text files, each bound to the topics it grounds (a
**topic** is a `#` heading with the prose and cards under it; see
[Topics](03-the-deck-format.md#topics-and-sub-cards)). Each topic with a
source is examined against its own sources alone; a topic with no source is
never sent to the model and is not examined. A deck whose sources manifest
lists no source, and any loose deck outside a workspace, has no exam.

The exam never reads the files your cards cite through `<!-- at: -->`
locators and `cite:` roots (see [Source
citations](06-cloze-direction-images.md#source-citations)): to examine a
file, add it as a source.

Once every card in a deck with sources has been answered right at least
once (a passing review at any [session
depth](04-review-modes.md#session-depths-recognize-recall-reconstruct),
Recognize included, or a [walk](15-the-web-app.md#walking-a-deck) graded
**Got it** or **Partly**) the deck is **exam due**. It stays exam due, also
after passing, until a full sitting masters it, so retaking stays one tap
away; passing alone already unlocks its dependents. A deck without sources
simply becomes *finished* when all its cards
[graduate](05-scheduling.md#fsrs), unlocking its dependents directly, even
when it declares `cite:` roots.

## Sources

A workspace deck can also hold **sources**: frozen copies of local text
files, each bound to the topics (the deck's `#` heading blocks) it grounds.
They live in `sources/<deck-id>/` at the workspace root, where `<deck-id>`
is the deck's `id: deck-<token>` frontmatter value, beside a
`manifest.toml` that lists every copy by its id (`source-` plus the file's
SHA-256), its display path (`<key>:<path>`), and its bindings. Each topic
has a permanent id, a `<!-- id: topic-<token> -->` line in the deck file
(see [Topic IDs](03-the-deck-format.md#topic-ids)), and
`alix deck source list` prints every topic's id with its heading. A
command takes any unique prefix of a deck, topic or source id, as
`topic-3whz` does below.

```
alix deck source add decks/rust.md ../sdk/src/lib.rs --root sdk=../sdk --topic topic-3whz
alix deck source list decks/rust.md
```

- A **key** names a folder of live files on this machine. `--root
  <key>=<dir>` saves it on first use, `alix deck source root set <deck>
  <key> <dir>` and `alix deck source root unset <deck> <key>` change it
  later, and they live in `manifest.local.toml`, which never leaves the
  machine. A file is shown as its path under the root, after the key.
- A **binding** is a topic id, or `<ranges>|<topic>` for some lines only,
  for example `10-80,120|topic-3whz`: 1-based inclusive lines, ascending and
  disjoint. A source is always bound to at least one topic; a deck without
  `#` topics binds its deck id.
- `alix deck source bind <deck> <source> <topic>...` binds a source to more
  topics or replaces a topic's ranges, `unbind` removes topics from it, and
  `remove <deck> <source>...` drops sources and deletes their copies.
- `update` refreezes a source from its file and keeps every binding,
  refusing when a range would lie beyond the new copy's last line;
  `list` shows whether each live file still matches its copy.
- `--keep-results`, on `update` and on `alix doctor --repair-sources`,
  declares that the new bytes keep the meaning. Each affected topic's
  result then moves to the topic's new material, so the topic stays passed
  and the deck's passed or mastered standing is not marked *sources
  changed* (see [What a standing means](#what-a-standing-means)) because of
  it. It never revives a result that was already stale (the topic's
  material had changed since that result was earned), and a standing
  already marked *sources changed* stays marked; a change still pending
  elsewhere, such as a card added to a topic without sources, also keeps
  the standing marked. Without the
  flag, every topic the source grounds reads unexamined until it is sat
  again. The manifest is written first; if the progress store then cannot
  be written, the command fails naming the topics whose results were not
  kept, and those results stay stale.

A source is refused when it is a URL (save the page as a file), empty, not
UTF-8, outside its root (a symbolic link out of the root included), already
in the deck under another path with the same bytes, or at a path the deck
already holds (refreeze that one with `update` or `alix doctor
--repair-sources`). Every command checks the deck's whole sources folder
first and refuses a damaged one. A change writes new copies first and the
manifest last, so an interruption leaves the old or the new state, and the
next change deletes any copy no row names. Every subcommand and flag is
listed in the [command reference](17-command-reference.md#the-ai-features).

`alix doctor` checks every member deck's sources folder: a damaged folder
(a copy missing, renamed, linked or not matching its id, a malformed
manifest) and a binding to a topic the deck lacks are errors, a copy no
row names is a note, and a `sources/<deck-id>/` folder no member deck or
member's `.bak` backup owns (left, for example, when `alix workspace generate` into an existing
workspace kept your deck of the same file name instead of the generated
one, or by hand edits) is a warning. A `sources/` folder that is a symbolic
link is one error, whether doctor checks the workspace, its `decks/` folder, or one
member deck. A live file changing is normal, so drift is opt-in:
`alix doctor --warn-on-source-drift` warns for each source whose live file
changed, is gone, or cannot be read, and names each key with no usable root
on this machine together with the `root set` command that saves one.
`alix doctor --repair-sources` refreezes every source whose live file
changed, keeping its bindings and printing each kept range for review; with
`--keep-results` it also keeps each refrozen topic's result and the
standing, under the rules above. It
refuses the whole repair when two sources would end up with the same bytes,
naming both, or when a range would lie beyond a new copy's last line. A live
file that is gone is reported and its source kept; a source under a key
with no usable root is never refrozen, and the warning names why and the
`root set` command that fixes it. The repair also deletes copies no row
names.

## How big a topic may be

Each topic's calls carry that topic's sources whole, and every backend CLI
refuses a prompt past a limit of its own. alix measures each sourced topic's
prompt ahead of the call at the largest it can get: the question-writing
prompt with `[exam]` `extra`, the longest idea lines the model may write
(an idea is cut at 300 characters), and the longest note alix adds when it
retries a rejected answer. It compares that with the configured backend's
limit, so no prompt the exam then sends can be larger than what was
measured. The limits were measured against each CLI:

| Backend | Limit | How alix measures |
| --- | --- | --- |
| Codex | 1,048,576 characters | an exact character count |
| Copilot | 128,000 estimated tokens | bytes divided by 2.50, rounded up, plus 2,200 |
| Claude | 1,000,000 estimated tokens | bytes divided by 2.51, rounded up |
| Gemini | 128,000 estimated tokens | Copilot's values stand in: Gemini's own limit could not be measured |

The token counts are estimates from the worst bytes-per-token ratio
observed, not the backend's own count, so a prompt the estimate admits can
still be refused by the backend.

- **Generate exam**, in the browser or on the phone, refuses to start when
  a topic's prompt exceeds the configured backend's limit, naming all four
  facts:
  ``topic `Moves` exceeds codex's limit: 1048577 of 1048576 characters``.
  A topic whose prepared questions are kept is not measured again.
- `alix deck source list` shows each sourced topic's measure for every
  backend and marks the ones it exceeds. `add`, `bind`, `update` and
  `alix doctor` warn with the same line and never refuse: the same measure
  everywhere, so what the list shows is what the exam will say.
- The fix is to split the topic into two `#` headings, or to bind only the
  lines the topic needs (`10-80|<topic>`).

## Sitting the exam

An exam is written ahead, then taken. Writing the questions is a long model
call (several minutes at a high effort), so it runs in the background while
you do something else, and the finished exam waits until you sit it. The
sitting itself is a guided, one-question-at-a-time flow (answer, move
Back/Next, then a per-question breakdown). It is one of a deck's launches
(the ways to start working on a deck), next to the review depths and Walk,
and you can sit it at any time as practice, before drilling too:

- **In the browser:** the **Generate exam** chip (key `x`) in the deck's
  focus drawer, the panel that opens under a focused deck in the picker (see
  [Choosing decks in the browser](15-the-web-app.md#choosing-decks-in-the-browser)),
  starts writing the exam on the server. While it runs, a **Writing exam**
  chip shows the minutes so far and a rough wait, beside a **Stop** chip; a
  short notice says when the exam is ready, and the chip then reads **Take
  exam** (key `x`). The chip is filled when the deck is exam due and its
  prerequisites are passed, and shows a lock while a prerequisite is not
  passed. Choosing an exam-due deck itself drills; it never starts the exam.
- **On the phone:** the deck's launch sheet, which opens when you tap a deck
  (see [The deck list](18-the-mobile-app.md#the-deck-list)), has an exam row
  below Walk that
  reads **Generate exam**, then **Preparing** with the rough wait, then
  **Take exam**; the picker says when the exam is ready. The phone sits the
  exam through a [paired](19-pairing.md) desktop, so the row is disabled
  while no desktop is reachable.

**Generate exam** offers **only what needs work**, on by default: the exam
then asks only about the topics that have not passed (those that need
work or were never examined). The control is always shown and disabled
when every topic has passed: with nothing left to filter, the exam covers
every topic whether the control is on or off.

A prepared exam is kept until it is graded. In the browser it lives in the
deck's store (`.alix/exam/<deck-id>.prepared.json`), so it survives a closed
page or a restarted server, and your answers are saved into it as you give
them. On the phone, the desktop holds the prepared exam in memory and the
phone saves each answer as you move on from it; a restarted desktop loses
the phone's prepared exam, and the **End exam** button under each question
discards it. The phone records a result against the material as it stood
when the phone started that sitting, and it remembers that starting point
only while the app runs: a sitting reopened after the app restarted, or
one this phone did not start (another client's, even on the same deck),
shows its result but records nothing. A result the phone already recorded
shows again, if the desktop still holds it, without being recorded twice,
and its remediation cards are not applied a second time. Either
way, an
answer locks once you move on from it, and leaving returns you to the first
unanswered question next time. Grading uses the exam up. In the browser,
editing cards, topic prose or a topic's heading leaves a prepared exam
valid. Changing a topic's sources (adding, updating, removing or rebinding
one, or changing its ranges), or removing a topic, makes it stale: Take
exam is refused, the prepared exam is kept, and the next Generate exam
rewrites only the changed or new topics, keeping the other topics'
questions and your answers to them.

Writing the exam gives [card IDs](03-the-deck-format.md#why-editing-a-deck-is-safe)
to cards added since the deck was last opened, the way opening a review
does.

`alix` writes **fresh understanding questions** one topic at a time (cards
before the first `#` heading form one topic, named after the deck; two
headings with the same text are two topics; a deck where two topics share an
id is refused until `alix doctor`'s finding is fixed). Each topic's calls carry that
topic's frozen copies whole, in display-path order, each under its display
path; a ranged binding carries only its lines, under a `path:a-b` header.
Nothing is fetched or cut short, so the same deck and sources give the same
prompt on any machine. A copy that is not UTF-8 text fails the exam, naming
the source. Each topic keeps a list of its **ideas**, the distinct mental
models its topic holds in its sources, whether or not a card states them. A
first call writes that list for a topic that has none yet, or whose sources
changed. Each exam then picks **three ideas per topic**: ideas never asked first, then ideas last failed, then ideas
asked longest ago, so repeated exams work through the whole list. A second
call writes exactly one question per picked idea and asks the student to
apply an idea a card states rather than recite the card. A reply that skips
a picked idea, asks one twice, or names an idea that was not listed is
rejected and generated once more. Grading records each asked idea
as passed or failed, which steers the next pick. The browser keeps the idea
lists in `.alix/exam/<deck-id>.json`, one entry per topic id; the phone
keeps its own and sends them with each exam.

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
against that question's key points, written from the sources along with the
question, never against your cards** (grading the cards would be circular). The model calls run on a background thread, so the UI
stays responsive while it thinks.

## Results: a count, per topic

A sitting reports points, not a verdict: each Pass is worth 1, each Partial
0.5, each Fail 0, shown as `17 / 18` or `17.5 / 18`, with the same count for
each examined topic and the shape of each question. The deck's progress
records each examined topic's latest result, together with the fingerprint
of the sources it was examined over. If an examined topic's sources change
(or stop loading), or a card is added to an unsourced topic, while the exam
is open, the sitting records nothing at all, no result and no standing, and
says the exam must be taken again:

- A **topic passes** when its share of Pass answers reaches `pass_threshold`
  (a fraction; 1.0, every answer, by default; a Partial counts as no Pass
  here). A topic reads as *passed* while its latest result passed, *needs
  work* while it failed, and *never examined* when it has no result or its
  sources changed since the result was recorded: a result earned on older
  material does not count, so "only topics needing work" always includes a
  topic whose sources changed. The topic's cards play no part in this:
  adding a card to a passed topic keeps it passed, and a topic with no cards
  can be examined.
- A sitting can set a standing only when it meets the **review condition**
  at its start: every card of every topic *without* a source,
  [retired](05-scheduling.md#retiring-cards) cards included, has graduated
  at Recall or deeper, its latest grade at that depth was a pass, and it is
  not yet due (scheduled for review now). Those cards are the only evidence
  that an unsourced topic was learned, so they must be learned first. A
  sitting that starts without the condition still runs and records every
  topic's result, but sets neither passed nor mastered.
- The deck is **passed** the first time every topic has passed, whether in
  one sitting or across several filtered ones, by a sitting that met the
  review condition. Passing, not mere drilling, is
  what unlocks decks that `requires:` this one, and a deck that has sat its
  exam once is never locked again by its own prerequisites. Source-less decks
  are unaffected: finishing them just means drilled (`done ✓`).
- The deck is **mastered** the first time a full sitting (every topic) that
  met the review condition scores every answer Pass; a filtered sitting
  never sets it.

## What a standing means

Neither passed nor mastered is taken back by a later, weaker sitting. The
results screen and the deck row both name the highest of them with the day
it was reached, and the row adds the last full sitting's day and points:
`passed (2 Nov) · last (4 Nov) 12 / 18`, `mastered (4 Nov) · last (4 Nov)
18 / 18`, or `last (4 Nov) 15 / 18` when examined but not passed. A date
outside the current year names the year (`31 Dec 2025`).

A standing names the material it was earned on: the sitting that sets it
records the sourced topics' fingerprints and the id of every card in the
unsourced topics, cards in your [personal
file](03-the-deck-format.md#your-personal-file) included. When that
material changes later (a source refrozen or rebound, a topic given or
stripped of its source, a card or a cloze blank added to an unsourced topic, a remediation card among them),
the standing reads as **sources changed**: `alix stats` prints
`passed (sources changed)` or `mastered (sources changed)`, the web deck row's
exam badge and the phone picker's badge turn dashed, and the exam result's
standing line does too. It still stands:
a dependent it unlocked stays unlocked, and mastered stays mastered. Re-sit
(the changed topics are the ones needing work) and pass in a sitting that
meets the review condition, and the standing is re-earned on the current
material. Removing a card or a whole unsourced topic, editing card text or
headings, and a card becoming due or lapsing never change a standing.

The web and phone exam results list, in one line under the topics, the
names of the topics they left out because no source grounds them: every
such topic that holds a card, counting the cards in your personal file (the
leading topic, the cards above the first `#`, is named by the deck's
title).

Whenever an answer missed key points, the results offer to turn the gaps into
**remediation cards**: a cloze card or a plain card for a missed fact, an open
understanding card (a prompt plus key points) for a missed concept, with
overlapping gaps merged. Re-drill those and re-sit. Once created, the screen
reports how many remediation cards it added. The results of a sitting that
recorded nothing offer no remediation, on the web and on the phone.

Remediation runs once per failed topic, and each card it writes is bound to
that topic. Those remediation cards land in the deck's
[personal file](03-the-deck-format.md#your-personal-file) (`<deck>.local.md`),
never in the authored deck. They start as new cards: the drill introduces them,
then [FSRS](05-scheduling.md#fsrs), alix's scheduler, schedules them like
any other card. Once scheduled they count toward
the deck's *due* total but not toward its card count. Regenerating the same gap won't
duplicate it; once a card's interval reaches `retire_after` it
[retires](05-scheduling.md#retiring-cards),
and re-failing the gap brings it back. Because they are plain Markdown blocks
in a file you own, you can read, edit, or delete them yourself.

Resetting a whole deck (`alix reset <deck>`) also clears its exam state, so a
re-drilled deck must pass again; `alix reset <deck> --exam` clears only the
exam state (each topic's result, the record of the material the standing
was earned on, passed, mastered and the last count) and names the decks whose exams it will lock. Resetting only an individual card
(`--card`) leaves the deck's passed and mastered state intact.

## Strictness: match the rigor to the material

How hard each answer is judged fits the *material*: a checklist topic (a procedure,
exact syntax, a security drill) should fail you for omitting a step; a conceptual topic
shouldn't. It's a learner setting, not per deck: the `[exam]` `strictness` config default,
optionally overridden per workspace in the `[defaults]` table of its
`alix.toml`, the shared settings file that travels with a workspace (see
[Workspaces](08-workspaces.md)). The levels:

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
`[exam]` config section (see [Configuration](16-configuration.md)).
`effort` sets how hard the backend thinks while writing and grading the
exam; when it is unset, the exam uses the `effort` of the `[ask]` section,
which configures the AI backend's calls. A workspace may set its own exam
effort in the `[exam]` section of its `alix.local.toml`, the personal
settings file that stays on your machine (see [Personal
pacing](08-workspaces.md#personal-pacing-alixlocaltoml)).

## Why this is the centerpiece

Everything else serves this. The drilling loads the facts; the exam is the gate
that turns "I reviewed it" into "I understood it, and here's the check." It's also
why *passing* (not completion) drives [unlocks](09-dependencies.md): a curriculum
should open the next door only when you've genuinely passed through the last. The
everyday, self-graded rehearsal for it is `explain` mode
([chapter 4](04-review-modes.md)); the exam is the real thing.
