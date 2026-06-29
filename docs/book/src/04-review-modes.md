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
stage), or **nailed** (up one stage). The same three grades the trace walk uses.
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
Recognition is easier than recall, so a correct pick grades **good** (never easy)
and a wrong pick fails. If a session has fewer than four distinct answers, the
card falls back to flip.

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
all → nailed, some → partly, none → failed — so the self-grade is a per-claim
check rather than a gut call. Atomic-answer cards get no key points and keep the
plain reveal.

---

To drop a card mid-session, press the **remove** key (`Ctrl-X` by default) instead
of grading it: it leaves the session and is deleted from the deck file when you
finish.
