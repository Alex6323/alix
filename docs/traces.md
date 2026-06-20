# Traces — design notes

> Status: **design, not built.** This captures the full model worked out in
> conversation so it survives across sessions. Workspaces (the foundation) and
> the `flash.toml` manifest are already shipped on `main`. Traces are the next,
> separate experiment — intended to live on its own branch on top of workspaces.
> This is the concrete realization of ROADMAP L8 ("understand and replicate how
> AI/Claude learns — cards == filling the context in the brain").

## Why traces exist

Flashcards (and Anki) are good at one thing: getting **atomic facts** retrievable
to near-automaticity via SRS. That's necessary but it's a ceiling. Real
understanding lives in the **connections between facts** — the *edges* of a
knowledge graph, not the *nodes*. Anki and the existing `flash exam` both test a
**set** of independent items. Nothing makes you trace a **path** and predict each
hop. That unoccupied territory is what traces target.

The model came from asking: *how does Claude actually build understanding of a
repo/paper to solve a problem?* The honest answer — the loop we want to
productize:

1. **Goal first.** Never read for coverage; read to answer a specific question.
2. **Find the load-bearing nodes** (the spine: the central type + the main path).
3. **Traverse edges, not lines** — follow references / data flow, building a graph.
4. **Predict, then verify.** Before reading the body, predict what it does; the
   *gap* between prediction and reality is where learning happens.
5. **Compress to a re-derivable model.** Throw away the text, keep a model that
   *regenerates* the details. If you can re-derive it, you understand it.
6. **Change it and watch it break** (tests/compiler as the feedback signal).

The context window is the *substrate*, not the understanding. Understanding is
the active predict-and-verify loop over a relationship graph, compressed into a
re-derivable model. Traces automate that loop.

Two honest disanalogies that shape the design:
- **My understanding is ephemeral** — re-derived each session. A human's
  persists. That persistence (the SRS layer under the loop) is exactly the value
  the tool adds that I lack.
- **I have absurd reading bandwidth; a human doesn't.** So the *selectivity*
  (goal-first, hub-first, one-path-at-a-time) matters far more for a human —
  tracing one path *well* is the whole game.

## The core mechanic

A trace is a **single path** you walk hop by hop. At each checkpoint:

1. Show the **anchor + ask** (the prediction prompt).
2. **You commit a prediction** (type it) — committing is the point.
3. **Reveal** the ground truth: a *real excerpt from the source* (with a locator),
   never an invented answer.
4. **Delta judgment**: Got it / Partial / Missed — *what* you missed.
5. **Advance from the revealed truth**, not your guess — so a wrong prediction
   doesn't derail the chain. The miss is recorded (for SRS), not punished.
6. **End: compress.** "Restate the whole path in two sentences." Checks whether
   your compressed model can re-derive the steps. Following along ≠ understanding.

Three properties do the work: it's a **path not a set**; **predict-before-reveal**;
and **the ground truth is checkable** (it's *in* the source).

## Two tiers: orient → goals → trace

You often don't even know the goal. The first goal is always **orient**, and
orientation is the cheap step that *manufactures* the goals — it bottoms out the
regress because orienting needs no prior understanding (just skim structure).

How I orient cold: manifest first (what kind of thing + its tech), then module
names (the nouns / domain model), then the entry point, then README (treated as
*intent*, a claim to verify, not truth). With no task, the default goal is "find
the **spine**" (central noun + main path).

**Decided model:** orientation is itself the **top-level "orient me" trace** —
you predict the shape at each hop ("given these deps, what kind of app is this?"),
and its **compression output is the map + a menu of deeper trace-goals**. So it's
traces all the way down, and you *learn* the orientation too. Escape hatch:
**`flash trace <source> --map`** skips the quiz and just prints the map + goals.

## Authoring (what you write) — minimal

You declare only what you actually know: the **goal** — written as a `% trace:`
line, a path description, *what* you want to understand — and the **scope**
(`% source:`). You do **not** pre-specify the checkpoint count or which files the
path crosses — those are *outputs* of the build (you can't know them before
tracing).

```
% trace: how pressing the Good key becomes a saved grade
% source: .
```

`% source:` is the **scope** (reuses the exam directive): a repo (`.`), a
directory, a single entry file, or a URL/doc. Optional soft cap:
`% checkpoints: ~8` (never required; defaults to "as many as the path needs").

## Build (`flash trace --build <deck>`)

Claude **explores** the scope, traces the path, and writes the discovered
checkpoints back into the deck file (cached, version-controlled, editable). The
specific files surface per-checkpoint; you never told it the file set or hop
count. Re-build explicitly (`--build`) when the source changes (stale → flag).

### Generation-prompt spec — a chain, not a set

This is the rubric the `--build` prompt must encode. It is **not** optional
polish: it is the difference between a trace and a themed quiz, and it was
learned by hand-authoring (then dogfooding) the first examples — a deck that was
a *set* of facts about one function felt hollow, and a path with a broken
*return* felt disconnected. Bake every rule below into the generation prompt so
each generated trace is a chain *by construction*, not by luck:

1. **One path, not a set.** Trace a real **sequence** — a data flow, a control
   flow, a causal chain, or a derivation — that runs from a *trigger* to the
   *outcome named in the `% trace:`*. If the candidate checkpoints are independent
   facts hanging off one node (a hub with facets), that is a card deck, not a
   trace: pick a different spine, or refuse and say so. The litmus test: if you
   can **reorder two checkpoints** without anything breaking, it's a set.
2. **Every prompt opens on the previous reveal.** Checkpoint *N+1*'s prompt must
   restate the **conclusion just revealed at *N***, then ask the next question
   (e.g. "the request carries no card id — how does the server know which
   card?"). If a prompt is answerable without having seen the prior reveal, the
   link is broken — rewrite it so the prior reveal is its premise.
3. **Carry the state, not the bookkeeping.** When a prompt restates what came
   before, phrase it as **standalone fact about the system** — "the grade has been
   applied and the new stage is now recorded in the store" — *never* as a reference
   to the trace's own structure ("as hops 2–3 showed", "the call from checkpoint 2",
   "those last two hops"). Each checkpoint is an **individual SRS card that can
   resurface alone, out of order, weeks later**, so an index reference is a
   dangling pointer — "checkpoint 2" means nothing when this card comes up by
   itself. The opening should read like a **status line of where the system is
   now**, not a recap of the lesson. That is what keeps a checkpoint *atomic yet
   connected*: connected through accumulated state, not through position.
4. **Ask forward — and just ask.** The front poses a question answered by
   reasoning *forward* from the prior reveal ("how does the server know which
   card?"), not one answered by outside recall of a fact. Phrase it as a **plain
   question**; don't prefix fronts with "Predict what…" — predicting is the whole
   mechanic and the UI already prompts for it, so the word is noise. A
   forward-looking question is already a prediction. The nudge to *commit a guess
   even when unsure* ("a hunch beats 'I don't know'") is a constant, so it lives
   **once** in the walk's framing and the `predict >` prompt — not repeated into
   every front (same reason as rule 3: constants belong in the frame, not the
   content).
5. **Don't lead the witness — keep the answer out of the prompt.** The setup
   states the established state (rule 3) and poses the prediction, but must not
   *contain* that prediction's answer, or a tell that hands it over. Beware
   evaluative/loaded framing — "the stage lives **only** in memory", "it isn't
   saved **yet**", "the id is **still** unknown" — which names the very gap the
   next hop fills; that gap is exactly what the learner should predict. State the
   carried-forward fact *neutrally* ("the new stage is now recorded in the store")
   and let the learner draw the consequence themselves; the insight belongs in the
   reveal and key points, **never the front**. Litmus: if the learner could
   produce the prediction just by paraphrasing the prompt, the answer leaked —
   strip the tell.
6. **Dives must return.** When a hop calls into another function or file, the
   next hop may **dive into** the callee — but you must then **climb back to the
   call site** before continuing past it, or the chain snaps (a reveal deep in a
   callee followed by a hop about the caller's *next* line, with nothing
   bridging the return). Reuse the call-site line in **both** the caller hop and
   the return hop (overlapping `% at:` ranges) to stitch the seam — but bridge it
   with *state* ("the call has returned; the store now holds the new stage"), per
   rule 3, not with "the call from checkpoint 2". A clean shape is symmetric:
   *caller calls X → dive into X → return to caller → caller calls Y → dive into
   Y …*.
7. **The reveal is the real source.** Ground truth is a **live excerpt** via
   `% at:`, never invented. The key points must paraphrase *exactly those lines*,
   and the prompt must be answerable *from* them — the model selects the path and
   the locator; the source is the oracle.
8. **The last hop lands the goal.** The final reveal is the **payoff that
   completes the `% trace:`** — it reaches the outcome the path was tracing
   toward; the compression step then retraces the whole path. If the last
   checkpoint doesn't reach that outcome, the path stopped short.

Two self-checks for the builder to run before emitting the deck. **Order:** read
the checkpoints top to bottom using **only each prompt + the prior reveal**; if a
hop needs information no earlier hop (or its own excerpt) established, it's out of
order or off the path. **Atomicity:** read each checkpoint **in isolation, as if
it were the only card you saw today**; its prompt must still make sense (state
carried as fact, no "checkpoint N" / "the last two hops" references) — because in
review it often *will* surface alone.

## The trace deck format — it's a normal deck

A trace deck reuses the existing plain-text format: a sequence of **`explain`-style
cards** (open prompt + key points), one per checkpoint, plus a per-card `% at:`
pointer to the real source. The only new directive is `% at:`.

```
% trace: how pressing the Good key becomes a saved grade
% source: .

# You press Good. What fires, and what does it send where?
	The keydown handler calls grade("good")
	It POSTs to /api/grade
	The body carries only { grade } — not which card
	% at: assets/serve/review.html:980,314
	! The page is a thin view; it never tracks card identity.

# The body has no card id. How does the server know which card?
	The server holds the session (reviewing.session)
	current() is the card being graded — state lives server-side
	So the server, not the page, is the source of truth
	% at: src/serve.rs:544-550
```

| Part | Role in the walk |
|---|---|
| `#` front | the **predict** prompt — you type a guess before anything reveals |
| indented lines | the **key points** a good prediction should hit (the rubric) — shown on reveal; what self-grade / `--grade` checks against |
| `% at: <locator>` | the **reveal** — flash reads those lines from the real file and shows them. Ground truth is the source, not the model |
| `! note` | the connective **insight**, shown after the reveal |
| file order | the **path** — checkpoints walked top to bottom |

It degrades gracefully: even without `flash trace`, it's a valid deck of explain
cards. Each checkpoint gets the normal card identity (hash of its key-point
lines), so **SRS attaches per checkpoint for free**.

### `% at:` locator
Generalize beyond code: a **quoted span** is the universal, verifiable form ("the
passage that says X") — works for code snippets, paper sentences, doc sections;
robust to reformatting. `file:lines` / `page` are optional precision. **Read live
from the source** at session time (always current, keeps the deck small), with
the source as the oracle.

## Grading — self-graded by default, `--grade` opt-in

- **Default (self-graded, offline, free):** on reveal you see the real excerpt +
  the key points and rate yourself Got / Partial / Missed (like flip/explain
  today).
- **`flash trace --grade` (or `[trace] grade = live`):** Claude judges your typed
  prediction against the key points + excerpt and produces the specific delta
  (reuses the exam grading machinery). Costs a model call per hop.

## SRS & "understood"

Each checkpoint's Got/Partial/Missed updates its strength in the store. A
**mispredicted checkpoint is a weak edge** → resurfaces sooner (a later, shorter
session can revisit just the weak hops); nailed ones fade. Completing the path +
a clean compression marks the trace "understood." Retention later = **re-walking
from memory** (SRS surfaces weak checkpoints) + re-compressing — the trace analog
of the L24 exam-decay question, but it falls out of per-checkpoint scheduling.

## Exams on traces — the compression *is* the exam

A trace carries `% source:` (the path origin), which collides with the exam's
`% source:` (exam corpus). Resolution: **each deck kind carries its own
verification, and they never cross.**

- review deck → mechanical drill.
- source deck → the AI exam (source-wide).
- **trace deck → the predict-verify walk + the compression step, which is its
  exam, correctly scoped to the path.**

So generic `flash exam` **refuses** on a trace; a trace's `% source:` is read as
*locator base*, not exam corpus. **Decided:** the kind is **derived, not declared**
— `% trace:` is the trace marker and takes precedence over `% source:` (no new
`% kind:` directive; consistent with L43's `% link:` = "not exam ground truth").
Kinds today: card vs trace; "source/exam" is a *modifier* on a card deck, not a
third kind. (Promote to an explicit `% kind:` only if the kind count grows, e.g.
generative cards.) A workspace can hold both a source/exam deck and trace decks
over the *same* corpus — same material, two artifacts, two verifications, never
mixed in one file.

## Files & organization

**One file per trace** (forced cleanly by the format: `% trace:`/`% source:` are
header-level, one set per file — so you can't put two traces in one file anyway).
Traces live **inside a workspace** (already built; `flash.toml` manifest). The
orient-trace's output (map + child goals) is the workspace's index. `% requires:`
between traces sequences *understanding* via the existing lock/unlock + mastery
machinery — a repo becomes a dependency-ordered set of traces you climb.

```
~/decks/flashcard2/            (a workspace)
  flash.toml                   (title + shared directives; later: trace corpus)
  orient.txt                   (the orient-trace → map + menu of goals)
  card-identity.txt            (a trace)
  keypress-to-grade.txt        (a trace)
```

## Trust & honest hard parts

- The make-or-break is **generating a *correct* path** — a wrong trace teaches
  wrong things. Mitigation = the reveal is always a **real excerpt with line
  refs**: the model's job is *selecting the path and judging your delta*, not
  being the oracle. You can always see the real code under the claim.
- Per-hop live grading costs latency/model calls → that's why it's opt-in.
- Stale source → identity ties to source content; changed source flags `--build`.

## Frontends

CLI first (mirrors how `flash exam` shipped), reusing: `% source:`, the
`claude -p` plumbing, exam's Pass/Partial/Fail + rubric grading, the store +
scheduling, `% mode: explain` cards. A web trace surface comes later (the web is
the primary surface per ROADMAP L4).

## Reuse map (existing pieces this builds on)

- `% source:` + WebFetch/file reading (exam).
- `% mode: explain` cards (open prompt + key points + self-grade).
- `exam::Sitting` + `spawn_*` runners + Pass/Partial/Fail rubric grading.
- The store + per-card SRS identity + scheduling; deck states/unlocks (`% requires:`).
- Workspaces + `flash.toml` (the home for a trace set; later a `[trace]` corpus).
- `directories`/`claude -p`/`config` plumbing.

## Suggested build order (phased, de-risked)

The riskiest unknown is **whether the predict-verify trace actually feels like
understanding as a tool** (the conversation was a manual dry-run that felt right,
but a chat ≠ the tool). So validate the mechanic cheaply before heavy plumbing:

1. **Atom / single-source trace:** `flash trace` over one explicit `% source:`,
   predict→reveal→delta→compress, self-graded, on a feature branch. Prove it's
   good.
2. **Build (path discovery)** from a scope; cached trace artifact. Its prompt
   must encode the [generation-prompt spec](#generation-prompt-spec--a-chain-not-a-set)
   (chain-not-a-set) so generated traces are paths by construction.
3. **Orient tier** (`orient` as the top trace + `--map`), producing the workspace
   map + `% requires:`-ordered child traces.
4. **`--grade`** (live delta), **web surface**, richer `flash.toml [trace]` config.

## Open / deferred decisions

- Exact `% at:` syntax (quoted span vs symbol-anchor vs `file:lines`) — lean
  quoted span primary, line refs as precision.
- Whether the orient-trace also emits a standalone map artifact vs only the
  in-trace compression output (lean: emit the map as the compression result).
- How `--build` bounds path length / picks the spine on large repos.
- Inline-cached excerpt vs live-read (lean: live-read; source is the oracle).
