---
name: Feature proposal
about: Propose a new capability — open this BEFORE writing code, so its fit is agreed first
title: "[proposal] "
labels: ["proposal"]
---

<!--
alix stays small on purpose. The fit gate (CONTRIBUTING.md) decides whether a
feature belongs, and it's cheapest to apply BEFORE any code exists. Fill this in
and let's reach a go/no-go together.
-->

## What's the idea?

<!-- The feature in a sentence or two. -->

## Which core-loop step does it deepen?

<!--
review → understand → verify → retain. Name the one it sharpens. If it sits
*beside* the loop rather than deepening a step, say so — that's "scope," and the
bar is higher (which is fine — let's talk it through).
-->

## Reach or scope?

- [ ] **Reach** — widens access to the *same* job (install, onboarding, a surface, a backend, perf, reliability, docs)
- [ ] **Scope** — a new job / card type / subsystem (this one has to earn its place)

## Fit gate self-check

- [ ] It does **not** move alix toward the NOT-list (accounts/SaaS, notes app, open-ended chat, gamification, marketplace, SR-migration layer).
- [ ] It **extends an existing concept** (a `%` directive, a mode) rather than adding a subsystem the user must learn — or, if it adds a new concept, here's why that beats merging into an existing one:

<!-- explain here if you ticked "adds a new concept" -->

## Subtraction test

<!-- Would removing this make alix meaningfully worse at its one job? Why? -->

## Anything else

<!-- Prior art, alternatives you considered, links. -->
