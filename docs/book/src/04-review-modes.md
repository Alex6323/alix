# 4 · Reveal & depth

How a card is checked isn't one setting you pick per card. It falls out of two
independent axes:

- **Reveal-method** — *how the answer is uncovered.* Authored per card (or
  deck-wide) with `% reveal:`, because only the author knows the answer's shape.
- **Depth** — *how deeply you're asked to retrieve it.* Your personal
  `[review] target`, because only you know how well you want to know this
  material. It's not a deck directive — depth is the learner's call, not the
  author's.

alix derives the concrete check from the pair, so you never hand-write "type this
one" or "explain that one." The point of two axes is to separate *presentation*
(the author's job) from *difficulty* (yours).

## The reveal-method axis — `% reveal:`

Three ways to uncover an answer, set with a `% reveal:` directive (per card, or in
the deck header for all cards; default `flip`):

- **flip** *(default)* — the whole answer is revealed at once.
- **cloze** — the answer is shown with a gap to fill; `{{...}}` spans in the answer
  mark the gaps, and the card expands into one sub-card per gap. See
  [cloze cards](06-cloze-direction-images.md).
- **line** — the answer is revealed one line at a time, for ordered material
  (lyrics, a sequence of steps). Pair it with `% order: sequential` to walk the
  deck top to bottom.

```
# Stage every change in git, including deletions?
    git add -A

# Recite the opening.
    % reveal: line
    Now is the winter of our discontent
    Made glorious summer by this sun of York
```

A per-card `% reveal:` overrides the deck's; the deck's overrides the default. It's
a review property, not content, so it's invisible to a card's identity hash —
adding or changing it never resets progress.

## The depth axis — `[review] target`

Depth is a small nested ladder, `recognize` ⊂ `recall` ⊂ `reconstruct`:

- **recognize** — pick the answer out of options. In this version it's only the
  ungraded *acquire* on-ramp for a brand-new card (see
  [scheduling](05-scheduling.md)), never a scheduled target.
- **recall** *(default)* — bring the answer to mind, then reveal and self-grade.
- **reconstruct** — produce the answer in full.

You set the target you're aiming for once, in your config (or per workspace in an
`alix.local.toml`):

```toml
[review]
target = "reconstruct"
```

It's deliberately *not* on the deck: the same deck can be drilled shallowly by one
person and deeply by another.

## What you actually get — the two axes combined

The check derives from the reveal-method, the card's current rung on the ladder,
and the answer's shape:

- **At recall**, a `flip` or `cloze` card **reveals** and you self-grade; a `line`
  card reveals line by line, then you self-grade.
- **At reconstruct**, you **produce** it: a `cloze` card has you **type** the gap;
  a card with a short, single-line answer has you **type** it exactly (`Tab`
  reveals two more characters as a hint, but a hinted card counts as missed); a
  card with a richer, multi-line answer becomes an **explain** prompt whose back
  lines are the **key points** you self-grade against.

Grading is always the same three — **missed it / partly / got it** — feeding FSRS
*Again* / *Hard* / *Good*. The [scheduling chapter](05-scheduling.md) covers how a
card climbs the ladder toward your target and drops back on a miss.

> **A default-target deck reviews as recall** — reveal-and-self-grade — even for
> cards once authored to be typed or explained (the retired `% mode:` directive).
> Reconstruction checks only appear once you raise `[review] target` to
> `reconstruct`.

### explain — the self-graded reconstruct check

The reconstruct check for a rich (multi-line) answer is an open prompt: the back
lines are the **key points** a good answer should cover, not a string to
reproduce. You optionally type an explanation — never checked, just there to make
you commit before you peek — reveal the points, and grade whether you hit them.
It's for cards aimed at *understanding* rather than exact recall, and it's the
everyday, self-graded tier beneath the AI exam (a later chapter).

```
# Explain why spaced repetition beats massed review.
    Retrieval just before forgetting strengthens memory the most.
    Spacing forces effortful recall; cramming lets you coast on short-term memory.
```

Augment the deck with **key points** (`alix deck augment <deck> --target
keypoints`) and the reveal becomes a **checklist**: you tick each cached point you
covered and the grade is *derived* from the coverage — all → got it, some →
partly, none → missed it — a per-claim check rather than a gut call.
Atomic-answer cards get no key points and keep the plain reveal.

A different augment target, `alix deck augment <deck> --target format`, instead
*reshapes* a badly-shaped card — a list crammed into one prose answer, say — into
clean display lines, non-destructively: it changes how the card is shown, not the
deck file or how it's graded.

## The rung badge

In the web frontend a small badge above the answer shows the card's current depth
(`recognize` / `recall` / `reconstruct`), and its **opacity tracks FSRS
retrievability** — bright when the memory is fresh, dimming as the card comes due.
So the badge tells you both where a card sits on the ladder and how well you're
holding it. The terminal shows the concrete check instead (`flip`, `typing exact`,
`line by line`, `explain`).

## input: draw — draw instead of type *(web only)*

`% input:` is a third, separate axis: it changes how you *produce* an answer, not
how it's graded. `draw` swaps the usual typed/reveal input for a canvas — instead
of typing (or just reading) the answer, you draw or handwrite it, then self-grade
against the card's normal reveal.

Two ways to reach it:

- **Draw-only cards.** Set `% input: draw` on a card (or deck-wide) when the
  answer *can't* be typed — a diagram, a circuit, a piece of notation. The reveal
  is whatever the card already uses: an `% img-back:` image, or an explain card's
  key points. An authored `% input: draw` card always uses the canvas — the
  per-device toggle below can't turn it off (you can't type a diagram).
- **The per-device toggle.** For a card that *can* be typed, the web ☰ menu's
  **Draw answers** switch lets you answer on the canvas anyway — for the retention
  of writing by hand — without changing the deck file. It's remembered per browser.

Grading a draw card is entirely **self-reported**: there's no OCR or vision model
reading the canvas, so it works like a self-graded flip/explain card — you judge
your own drawing against the reveal. In this version `% input:` is honored on
**self-graded** checks only (a `flip` reveal or an explain), and is web-only (the
terminal ignores it).

---

To drop a card mid-session, press the **remove** key (`Ctrl-X` by default) instead
of grading it: it leaves the session and is deleted from the deck file when you
finish.
