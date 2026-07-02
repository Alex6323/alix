# 4 · Review modes

A *mode* decides how a card is tested — from "reveal it and be honest with
yourself" to "type it exactly." `alix` has six. Set one with `--mode` on the
command line, or per deck or card with a `% mode:` directive; the effective mode
resolves **`--mode` flag > card's `% mode:` > deck's `% mode:` > the default
(`flip`)**. A small badge above the answer always shows which mode is in play.

The point of having several is to match the test to what you're training —
recognition, exact recall, or understanding.

## flip — reveal and self-grade *(default)*

You read the question, recall the answer, reveal it, and grade yourself: **failed**
(missed — reset to stage 1), **partly** (got the gist but stumbled — down one
stage), or **passed** (up one stage). The same three grades the trace walk uses.
It's the Anki-style default, and the right choice whenever you can fairly
judge your own answer — conceptual questions, explanations, anything open-ended.

```
# Why is UDP described as connectionless?
    It sends datagrams with no handshake and no delivery guarantee.
```

## typing — type it exactly

You type the back of the card character by character, with instant green/red
feedback. `Tab` reveals the next two characters as a hint (press again for two
more), but a card you needed a hint on counts as failed. Use it where the answer
must be *exact* — syntax, spellings, command flags, a formula.

```
# Stage every change in git, including deletions?
    git add -A
    % mode: typing
```

## fuzzy — type it, typos forgiven

Like typing, but you submit a whole line with `Enter` and small typos are
tolerated (the tolerance is configurable; `--max-typos` defaults to 2). When the
answer is several lines, each is checked and their order doesn't matter. Reach for
it when you want the effort of *producing* the answer without being failed for a
slipped key.

A wrong answer in `typing` or `fuzzy` drops the card to stage 1 and brings it back
later in the same session until you get it.

## choice — pick from four

You choose the answer from four options with `1`–`4`. The three wrong options are
sampled automatically from the *other* cards' answers (preferring similar-looking
ones — years compete with years), so you never have to write distractors.
A correct pick grades **passed** and a wrong pick **fails** (recognition is easier
than recall, so there's no bigger reward than a normal pass). If a session has
fewer than four distinct answers, the card falls back to flip.

**AI distractors.** For wrong options written by Claude instead — plausible,
tempting answers the kind a half-learned mind would fall for — augment the deck
ahead of time:

```sh
alix deck augment mydeck.txt --target choices --with "use common misconceptions"
```

This is a deliberate, one-off command: it generates the distractors once and
caches them by card id (in `augment.json` beside your progress). Review reads the
cache automatically, so study stays instant and fully offline — Claude is never
called while you study. A card without cached distractors falls back to the
sampled ones above, so it never loses its options; and because the AI brings its
own wrong answers, choice mode works even on a deck too small to sample from. See
the augmentation chapter for `--target notes` and the rest.

## line — one line at a time

The back is revealed one line at a time: press the reveal key (`Space`), recalling
each line before you uncover it; once the whole card is shown you grade yourself
like flip. It's for ordered material — lyrics, poems, a sequence of steps. Pair it
with `% order: sequential` so the deck walks top to bottom (one card per verse,
say).

## explain — open prompt, key points

The back lines aren't a string to reproduce — they're the **key points** a good
answer should cover. You optionally type an explanation, reveal the points, and
grade yourself on whether you hit them. It's for cards aimed at *understanding*
rather than recall. The typing is never checked — a self-graded mode can't verify
your answer, so it doesn't pretend to; it's there to make you commit before you
peek. `explain` pairs with the ask-Claude tutor and is the everyday, self-graded
tier beneath the AI exam (a later chapter).

```
# Explain why spaced repetition beats massed review.
    Retrieval just before forgetting strengthens memory the most.
    Spacing forces effortful recall; cramming lets you coast on short-term memory.
    % mode: explain
```

Augment the deck with **key points** (`alix deck augment <deck> --target
keypoints`) and the reveal becomes a **checklist**: you tick each cached point you
covered and the grade is *derived* from the coverage —
all → got it, some → partly, none → missed it — so the self-grade is a per-claim
check rather than a gut call. Atomic-answer cards get no key points and keep the
plain reveal.

A different augment target, `alix deck augment <deck> --target format`, instead
*reshapes* a badly-shaped card — a list crammed into one prose answer, say — into
clean display lines, non-destructively: it changes how the card is shown, not the
deck file or how it's graded.

## input: draw — draw instead of type *(web only)*

`% input:` is a separate axis from `% mode:` — it changes how you *produce* an
answer, not how it's graded. `draw` swaps the usual typed/reveal input for a
canvas: instead of typing (or just reading) the answer, you draw or handwrite
it, then self-grade against the card's normal reveal. The mode's grading is
untouched — a `draw` `flip` card still reveals and asks Missed it / Partly / Got
it, a `draw` `explain` card still reveals its key points.

Two ways to reach it:

- **Draw-only cards.** Set `% input: draw` on a card (or deck-wide) when the
  answer *can't* be typed in the first place — a diagram, a circuit, a piece of
  notation. The reveal is whatever the card already uses for that: an
  `% img-back:` image, or an explain card's key points. An authored `% input:
  draw` card always uses the canvas — the per-device toggle below can't turn it
  off (you can't type a diagram).
- **The per-device toggle.** For a card that *can* be typed, the web ☰ menu's
  **Draw answers** switch lets you answer on the canvas anyway — for the
  retention of writing by hand — without changing the deck file. It's
  remembered per browser, not written to the deck, and it only *adds* drawing
  to an otherwise-typed `flip`/`explain` card.

Grading a draw card is entirely **self-reported**: there is no OCR or vision
model reading the canvas, so it works exactly like a self-graded flip/explain
card — you judge your own drawing against the reveal. In this version `%
input:` is honored on **`flip` and `explain`** cards only, and is web-only (the
terminal ignores it); it's an input axis meant to extend to other self-graded
modes later, but `line`, `typing`, `fuzzy`, `cloze`, and `choice` don't draw
yet.

---

To drop a card mid-session, press the **remove** key (`Ctrl-X` by default) instead
of grading it: it leaves the session and is deleted from the deck file when you
finish.
