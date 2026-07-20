# 4 · Reveal & session depths

How a card is checked isn't one setting you pick per card. It falls out of two
independent things:

- **Reveal-method** (*how the answer is uncovered*): authored per card (or
  deck-wide) with `reveal:`, because only the author knows the answer's shape.
- **Session depth** (*how deeply you're asked to retrieve it*): chosen per
  session (Recognize, Recall, or Reconstruct), because only you know how well you
  want to know this material right now. It isn't a deck directive, and not
  personal config either: it's a property of the session you start.

alix derives the concrete check from the pair, so you never hand-write "type this
one" or "explain that one." Keeping them separate keeps *presentation* (the
author's job) apart from *how deep you're drilling* (your call, per session).

## The reveal-method axis: `reveal:`

Three ways to uncover an answer. Set it deck-wide with a `reveal:` line in the
frontmatter, or per card with a `<!-- reveal: ... -->` directive (default `flip`):

- **flip** *(default)*: the whole answer is revealed at once.
- **line**: the answer is revealed one line at a time, for ordered material
  (lyrics, a sequence of steps). Pair it with `order: sequential` in the
  frontmatter to walk the deck top to bottom.

A card becomes **cloze** (a gap to fill) automatically when its answer contains
`\blank{...}` markers; that is triggered by the markup, not set as a `reveal:`
value. See [cloze cards](06-cloze-direction-images.md).

```
## Stage every change in git, including deletions?
git add -A

## Recite the opening.
Now is the winter of our discontent
Made glorious summer by this sun of York
<!-- reveal: line -->
```

A per-card `<!-- reveal: -->` overrides the deck's; the deck's overrides the
default. It's a review property, not content, so it's not part of a card's
identity: adding or changing it never resets progress.

## Session depths: Recognize, Recall, Reconstruct

Every review session runs at one of three independent depths, picked when you
start it with the web picker's split **Learn** button, whose small ▾ opens a menu
of the three (on the keyboard: `v`, then `1`/`2`/`3`; `Esc` cancels; rebindable in
[`[keys.picker]`](16-configuration.md)). The menu also carries the **cram**
tick-box (`c`); see [Cramming](05-scheduling.md). Plain **Learn** reuses the
deck's own last-used depth, remembered per deck. The first time you ever open a
deck, that default is Recognize if it already has AI-generated distractors (`alix
deck augment --target choices`, or the web Augment screen), since a genuine
multiple-choice pick is ready to go; otherwise it's Recall.

- **Recognize**: unscheduled, boolean, and **pick-only**. There's no FSRS state
  for it at all, just a per-card *recognized* flag. It's a genuine multiple-choice
  pick, built from the deck's cached AI distractors (`alix deck augment --target
  choices`): a cloze card asks you to pick its gap, a line card to pick the whole
  sequence in the right order. Only recognizable cards (the ones with a buildable
  pick) are scheduled, so a Recognize session never falls back to a plain reveal,
  which would just be a Recall in disguise. Options are never sampled from other
  cards' answers, so a deck with no cached distractors has nothing to recognize:
  the picker greys the Recognize depth out until you run the augment. A correct
  pick marks the card recognized; a quiet **"I guessed"** link right after lets
  you undo that, re-queuing it. A wrong pick shows which option was right, then
  **Continue** re-queues it too.
- **Recall** *(the default)*: the classic flashcard. Bring the answer to mind,
  reveal it, and self-grade. Its own FSRS schedule.
- **Reconstruct**: produce the answer in full, on its **own independent FSRS
  schedule** per card. Recall and Reconstruct are two separate practices, so a
  card can be due for one and not the other; the one pass-only downward credit
  between them is covered in [Scheduling](05-scheduling.md).

Nothing climbs or descends between depths on its own: a card's Recall and
Reconstruct schedules just sit there side by side, and which one you exercise is
entirely your call each time you start a session.

## What you actually get: reveal + depth combined

The check derives from the reveal-method and the depth:

- **At Recall**, a `flip` or `cloze` card **reveals** and you self-grade; a `line`
  card reveals line by line, then you self-grade.
- **At Reconstruct**, you **produce** it: a `cloze` card has you **type** the gap;
  a card with a short, single-line answer has you **type** it; a `line`-reveal
  card has you **type each line in turn**; a card with a richer, multi-line answer
  becomes an **explain** prompt whose back lines are the **key points** you
  self-grade against.

A typed check normalizes both sides (case, whitespace, trailing punctuation) and
compares exactly, with no edit-distance tolerance, then shows the diff. The
automated comparison is evidence, not the verdict: grading is still yours, so a
mismatch you recognize as a typo (not a wrong answer) can still be graded Got it.

Grading is always the same three (**missed it** / **partly** / **got it**),
feeding FSRS *Again* / *Hard* / *Good*. See the [scheduling
chapter](05-scheduling.md) for how Recall and Reconstruct's independent schedules
work, and how badges summarize a deck's progress at each depth.

### explain: the self-graded Reconstruct check

The Reconstruct check for a rich (multi-line) answer is an open prompt: the back
lines are the **key points** a good answer should cover, not a string to
reproduce. You optionally type an explanation (never checked, just there to make
you commit before you peek), reveal the points, and grade whether you hit them.
It's for cards aimed at *understanding* rather than exact recall, and it's the
everyday, self-graded tier beneath the AI exam (a later chapter).

```
## Explain why spaced repetition beats massed review.
Retrieval just before forgetting strengthens memory the most.
Spacing forces effortful recall; cramming lets you coast on short-term memory.
```

Augment the deck with **key points** (`alix deck augment <deck> --target
keypoints`) and the reveal becomes a **checklist**: you tick each cached point you
covered and the grade is *derived* from the coverage (all covered → got it, some →
partly, none → missed it), a per-claim check rather than a gut call. Atomic-answer
cards get no key points and keep the plain reveal.

A different augment target, `alix deck augment <deck> --target format`, instead
*reshapes* a badly-shaped card (a list crammed into one prose answer, say) into
clean display lines, non-destructively: it changes how the card is shown, not the
deck file or how it's graded.

## The check badge

In the web frontend a small badge above the answer names the check you're doing
right now (`flip`, `line`, `typing`, `choice`, or `explain`), so how you'll
interact is clear before you commit. It badges the *present* interaction, not the
depth: a brand-new card, or a Recognize pick, shows `choice` even on a card whose
Recall/Reconstruct schedule will use something else once it's acquired.

## Draw instead of type: `input: draw` *(web only)*

`input:` is a third, separate axis: it changes how you *produce* an answer, not
how it's graded. `draw` swaps the usual typed/reveal input for a canvas: instead
of typing (or just reading) the answer, you draw or handwrite it, then self-grade
against the card's normal reveal.

Two ways to reach it:

- **Draw-only cards.** Set it deck-wide with `input: draw` in the frontmatter, or
  per card with `<!-- input: draw -->`, when the answer *can't* be typed (a
  diagram, a circuit, a piece of notation). The reveal is whatever the card
  already uses: a `![](...)` image on the answer side, or an explain card's
  key points. An authored draw card always uses the canvas; the per-device toggle
  below can't turn it off (you can't type a diagram).
- **The per-device toggle.** For a card that *can* be typed, the web ☰ menu's
  **Draw answers** switch lets you answer on the canvas anyway, for the retention
  of writing by hand, without changing the deck file. It's remembered per browser.

Grading a draw card is entirely **self-reported**: there's no OCR or vision model
reading the canvas, so it works like a self-graded flip/explain card. You judge
your own drawing against the reveal. In this version `input:` is honored on
**self-graded** checks only (a `flip` reveal or an explain); it's ignored
elsewhere.

---

To drop a card mid-session, press the **remove** key (`Ctrl-X` by default) instead
of grading it: it leaves the session and is deleted from the deck file when you
finish.
