# 0040: A formula span blanks symbols and complete base-and-script pairs

- Status: Accepted (ruled by the maintainer 2026-08-31)
- Evidence: an_allowlisted_symbol_command_is_a_blankable_unit in src/parser/mathspan.rs
- Evidence: a_complete_base_and_script_is_a_blankable_unit in src/parser/mathspan.rs
- Evidence: a_cut_command_application_is_named_when_its_argument_continues in src/parser/mathspan.rs
- Recorded: 2026-08-31
- Retrospective: No

## Context

A `blank: span` over math source must be a complete structural unit of
its formula, so masking it can never leave dangling structure (span
masking design, 2026-08-19; regions per ADR 0034, spans as the only
cloze per ADR 0039). The first build of that law was maximally strict:
any contained control sequence and any contained `^ _ & % #` rejected
the span unless it covered the whole formula.

Strictness was safe but rejected units learners genuinely drill:

- `b^2` inside the quadratic formula's discriminant. The committed
  example deck had to drop exactly this blank under the strict rule.
- `\pm`, `\pi`, a Greek letter: standalone symbols with nothing to
  dangle. Since a math span sketches by default (ADR 0039, amended by
  ruling 2026-08-31), hiding a symbol no longer asks for its LaTeX
  spelling.

The retired `\blank{...}` marker never faced this: the author placed
its braces, so the grammar itself delimited the unit. A directive names
its target by text match, so the parser must judge unit-hood.

Two adversarial counterexamples (found by Codex) pin why "parses when
masked" is not a sufficient test:

- `z+\frac{a}{b}` with `\frac{a}` masked: the masked formula parses,
  but `{b}` is silently orphaned into a bare group.
- `x^2` with `^2` masked: the masked formula is a valid `x` followed
  by a box, and the exponent relationship is silently gone.

## Decision

The structural-unit law admits, beyond a whole-formula match:

**1. Blankable symbols.** A fixed allowlist of zero-argument,
learner-visible symbol commands (`BLANKABLE_SYMBOLS` in
`src/parser/mathspan.rs`: binary operators, relations, arrows, Greek
letters, big operators, log-like function names) may appear inside a
match. Any other contained command stays a loud error naming it. The
list is additive-only: removing an entry would reject decks it
accepted.

**2. Complete base-and-script pairs.** A contained `^` or `_` is legal
exactly when its base operand and its script operand (one token or one
balanced brace group each) both lie inside the match; otherwise the
error names the cut script. This admits `b^2`, `a_i`, `{ab}^{n+1}` and
rejects both `^2` alone and `x^` alone. (Amended 2026-08-31, after the
review's red constructions: a script is every spelling the pinned
renderer's atom parser converts into one, so Unicode
superscript/subscript characters and prime apostrophe runs follow the
same law, classified by the renderer's own public mapping rather than a
copied table; a base's stacked scripts all attach to the owning atom, so
`i^2` inside `x_i^2` is a cut, not a pair.)

**3. A cut command application is named as such.** When a contained
non-allowlisted command's match is followed by a further argument
group, the error says the application is cut (the `\frac{a}`
adversary), not merely that a command is present.

`& % #`, `\\`, split brace groups, comment interiors, phantom
arguments, and endpoints inside a control word stay rejected as
before. The whole-formula bypass and the mask-and-re-parse renderer
backstop stay as before.

## Consequences

Widening the allowlist later is loud-error-to-acceptance: a deck the
narrow list rejects fails at parse with a named violation, so adding
an entry breaks no existing deck. Narrowing is a format break and is
why the list is additive-only. The law is enforced at deck load, so an
over-wide entry cannot be caught later by doctor alone; entries are
reviewed against the two pinned adversaries (nothing an entry admits
may orphan an argument or a script).
