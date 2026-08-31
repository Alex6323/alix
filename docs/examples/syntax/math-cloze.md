---
title: "Cloze inside a formula"
description: >-
  A `blank: span` directive works on a formula line, inline or displayed.
  The hidden span is a piece of the formula, so what is asked is the
  step, not the sentence around it. Hide something that can be typed: the
  hidden text is the answer, and a control sequence like `\pm` would make
  the card a spelling test for LaTeX.
id: "deck-12x3e6yg9qkhxfjwgg9q4gwp7x"
---

## The quadratic formula
$$x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}$$
> [!NOTE]
> One blank, one sub-card. Blank what the learner would type: the hidden
> text is the expected answer, so hiding `\pm` would be asking for the
> string `\pm` rather than for any understanding.
<!-- blank: span hidden="4ac" b:q4acx7 -->
<!-- id: card-798bv2regxndcd0the8my59mb4 -->

## Euler's identity
$e^{i\pi} + 1 = 0$
<!-- blank: span hidden="1" b:3v9er8 -->
<!-- id: card-1ngfgr2bhw76mynyyappyt5m69 -->

## The derivative of a power
$$\frac{d}{dx}x^n = n x^{n-1}$$
> [!NOTE]
> Two blanks, two sub-cards, drilled separately. The exponent rule fails
> in two different places, so it is worth separating them.
<!-- blank: span hidden="n" occurrence=2 b:p2wrn6 -->
<!-- blank: span hidden="n-1" b:p8wn17 -->
<!-- id: card-6t775j2bw9ymbjky9g9bsy8zne -->
