---
format-version: 1
id: "deck-12x3e6yg9qkhxfjwgg9q4gwp7x"
---

# Cloze inside a formula

`\blank{...}` works inside `$...$` and `$$...$$`. The hidden span is a
piece of the formula, so what is asked is the step, not the sentence
around it. Hide something that can be typed: the hole's content is the
answer, and a control sequence like `\pm` would make the card a spelling
test for LaTeX.

## The quadratic formula
$$x = \frac{-b \pm \sqrt{b^2 - \blank{4ac}}}{2a}$$
> One blank, one sub-card. Blank what the learner would type: the hole's
> content is the expected answer, so `\blank{\pm}` would be asking for
> the string `\pm` rather than for any understanding.
<!-- id: card-798bv2regxndcd0the8my59mb4 -->

## Euler's identity
$e^{i\pi} + \blank{1} = 0$
<!-- id: card-1ngfgr2bhw76mynyyappyt5m69 -->

## The derivative of a power
$$\frac{d}{dx}x^n = \blank{n} x^{\blank{n-1}}$$
> Two blanks, two sub-cards, drilled separately. The exponent rule fails
> in two different places, so it is worth separating them.
<!-- id: card-6t775j2bw9ymbjky9g9bsy8zne -->
