---
id: "1ncnncxh9zz2c80ntnffh7jx03"
---

# Math rendering showcase

This deck is for manually checking every authored card surface that can carry LaTeX math.
One card near the end is intentionally malformed so the visible error fallback can be inspected.

## Inline math in a question: what theorem uses $a^2 + b^2 = c^2$?
The Pythagorean theorem.
<!-- id: 25s0jpnhaw8qfv5k8yp0jha25d -->

## Inline math in an answer: what is the mass-energy relation?
$E = mc^2$
<!-- id: 3hynx3kbyrt4chb2961hdac19s -->

## Does styled prose compose with $\alpha_i^2 + \beta_j$?
Yes. **Bold text**, *italic text*, `inline code`, and $\alpha_i^2 + \beta_j$ should remain distinct.
<!-- id: 06z5r3hv5k7nt2z57jwez747x3 -->

## What is the Gaussian integral?
$$\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}$$
<!-- id: 0ceyc641m6gzjweq7ftx9y7zts -->

## Name the operation represented by this display formula.
$$\begin{pmatrix} a & b \\ c & d \end{pmatrix}\begin{pmatrix} x \\ y \end{pmatrix}$$

---
Matrix-vector multiplication.
<!-- id: 3d0berjyra7j9bxswbt0p8cc4q -->

## State two trigonometric identities.
$\sin^2 x + \cos^2 x = 1$
$1 + \tan^2 x = \sec^2 x$
<!-- reveal: line -->
<!-- id: 65vs1bsxy8mv71p285qhceazfs -->

## Explain why $\lim_{x \to 0}\frac{\sin x}{x}=1$.
Geometrically, $\cos x \le \frac{\sin x}{x} \le 1$ near zero.
The squeeze theorem then gives $\lim_{x \to 0}\frac{\sin x}{x}=1$.
<!-- id: 2kk2avzmem577x00w3wrkmpbth -->

## Energy symbol
$E = mc^2$
<!-- direction: both -->
<!-- id: 2bc9dv0cs3n0f5whc01eeqjrfk -->

## Type the derivative of $x^2$.
$2x$
<!-- id: 77f0j9skjfa3wcd1aqtdb43sz1 -->

## Draw the unit circle relation.
$$x^2 + y^2 = 1$$
<!-- input: draw -->
<!-- id: 2kecxz9fwvwb4zce4q1xpn80z5 -->

## Complete the inline quadratic formula.
$x = \frac{-b \blank{\pm} \sqrt{b^2 - \blank{4ac}}}{2a}$
<!-- id: 3065zgmvgh9ne54yvshff7egsk -->

## Complete the displayed difference-of-squares identity.
$$a^2 - b^2 = \blank{(a-b)}\blank{(a+b)}$$
<!-- id: 44a77x675j7bwyjve4r5xhe9z8 -->

## Which derivative is correct for $f(x)=x^3$?
- [ ] $f'(x)=x^2$
- [x] $f'(x)=3x^2$
- [ ] $f'(x)=3x$
- [ ] $f'(x)=x^4$
<!-- id: 76xft3gym5tn20pmyh1y6qns6d -->

## Inspect these formulas as static front checkboxes.
- [x] $\sin^2 x + \cos^2 x = 1$
- [ ] $\sin x + \cos x = 1$

---
Only the first identity is true; these front checkboxes are not answer choices.
<!-- id: 0dvtp8xgj0nca3jye0b3r61sjy -->

## Where does inline math render inside a prose note?
After the answer is revealed.
> Euler's identity is $e^{i\pi}+1=0$, inside this sentence note.
<!-- id: 6bfwfcyswhv6wy5st7jpfyg8de -->

## Where does display math render inside a note?
After the answer is revealed.
> $$\sum_{n=1}^{\infty}\frac{1}{n^2}=\frac{\pi^2}{6}$$
<!-- id: 4t3qw2grtnar2655gt5hkz5af2 -->

## Can checklist notes contain formulas?
Yes.
> - [x] Maxwell-Faraday: $\nabla \times \mathbf{E}=-\frac{\partial \mathbf{B}}{\partial t}$
> - [ ] Coulomb force: $F=k\frac{q_1q_2}{r^2}$
<!-- id: 1fxk30e866bbdwv9m4eq52eq2g -->

## Do dollars inside a fenced note render as math?
No, fenced code remains verbatim.
> ```
> $x^2$ stays literal inside this note code block.
> ```
<!-- id: 1jfsk9ahqqd0yr5qpp8qzsza45 -->

## Does chemistry remain available through stock RaTeX?
$$\ce{2H2 + O2 -> 2H2O}$$
<!-- id: 2sq5kvya5evzvt2860eys9m0we -->

## Which dollar-like text deliberately stays literal?
Inline code: `$x^2$`
Currency: $5 and $10
Escaped: \$x
<!-- id: 6sxm8921x3cnt24ja7ef79cb15 -->

## What does a recognized but malformed formula show?
$\frac{1$
<!-- id: 6kgpc65bvw66m7aef94d5wdf4q -->
