# Traces — design notes

> Status: **design, not built.** This captures the full model worked out in
> conversation so it survives across sessions. Workspaces (the foundation) and
> the `alix.toml` manifest are already shipped on `main`. Traces are the next,
> separate experiment — intended to live on its own branch on top of workspaces.
> This is the concrete realization of ROADMAP L8 ("understand and replicate how
> AI/Claude learns — cards == filling the context in the brain").

## Why traces exist

Flashcards (and Anki) are good at one thing: getting **atomic facts** retrievable
to near-automaticity via SRS. That's necessary but it's a ceiling. Real
understanding lives in the **connections between facts** — the *edges* of a
knowledge graph, not the *nodes*. Anki and the existing `alix exam` both test a
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

## Goals and exploration

A **goal** is the long-term aim you hold — "understand this crate", "learn music
theory". It is bigger than any single path, so it is **not** a trace: it lives
one layer up, at the **workspace**. A workspace is the unit that *aims at a
goal*, and its fact **decks** and **trace** decks are the *means* to it. Reserve
the word "goal" for this layer — a single trace ("how X becomes Y") is a
path-question, not a goal, even though to an LLM each is just a task.

You often don't yet know which traces (and decks) serve the goal. That is what
**exploration** is for: the cheap recon step that, given the goal and its source,
*manufactures the means* — it proposes the traces worth doing, and it needs no
prior understanding (just skim structure). [`alix trace --suggest`](#suggesting-traces--alix-trace---suggest-source-recon-lite)
(below) is the first, flat slice of this; the full explore tier turns that menu
into a `% requires:`-ordered set living in the workspace.

**Traces or decks — chosen by the shape of the knowledge.** The means exploration
manufactures are *both* facts decks and traces, and which fits a subsystem is
decided by what it holds: **edge-shaped** parts (a path you predict hop by hop —
the review loop, queue-building, the scheduler) become **traces**; **node-shaped**
parts (a table of facts with no path — config knobs, a store's on-disk format)
become **facts decks**. Forcing a node-shaped subsystem into a trace just
manufactures a fake path (the set-pretending-to-be-a-path failure). So `--suggest`
covers only the trace-shaped subsystems and *names* the node-shaped ones it skips
as facts-deck material; explore proper proposes the decks for those — the goal stays
fully covered, by the right means each time.

How exploration reads a source cold: manifest first (what kind of thing + its
tech), then module names (the nouns / domain model), then the entry point, then
README (treated as *intent*, a claim to verify, not truth). With no specific aim,
the default is "find the **spine**" (central noun + main path).

**Decided model — built as [`alix explore --walk`](#exploring-a-source--alix-explore-source).**
Exploration is itself a **top-level "explore" trace** — you predict the *shape*
at each hop ("from these deps, what kind of program is this?", "from the module
list, what are the domain nouns?", "where's the entry point?", "what's the
spine?"), each revealing **real structural evidence** (the manifest, the
module-declaration lines, the entry enum, the spine's central file), and the
**last hop lands on the menu of candidate traces**. So it's traces all the way
down, and you *learn* the exploration rather than just receive it. It is written
to a file, so `alix trace <file> --map` reprints the map without walking.

## Suggesting traces — `alix trace --suggest <source>` (recon, lite)

The hard part of a trace isn't building the path — `--build` does that — it's
knowing *which* paths through a cold source are worth tracing, and **at what
depth**. That judgment needs you to already understand the source: you can only
size a hop if you know the path. So the person who can author a good `% trace:`
is the one who least needs it. `--suggest` breaks that bootstrap by outsourcing
the judgment to the model: a **cheap reconnaissance pass** over a `% source:`
that proposes a ranked **menu of candidate traces**. It does *not* build
checkpoints. (A *trace* is a path-question — "how X becomes Y". It is **not** a
*goal*, the higher-level statement of intent — "I want to learn music theory" —
that a future curriculum feature will turn into a *set* of traces; keep the two
distinct.)

It is the de-risked precursor to the [explore tier](#goals-and-exploration):
same job (manufacture traces from a cold source), but a flat menu — no quiz, no
hierarchy, no `% requires:` ordering. If the suggestions come back as real,
well-scoped spines, that validates the depth-judgment bet *before* the full
explore-tier machinery is built — and the recon prompt it proves out is the
one explore will reuse.

**Cost vs. depth is the whole point.** Recon must not `--build` every candidate
(that's N expensive builds). It does **one** exploration pass — the same
read-only `Read`/`Glob`/`Grep` (`WebFetch` for a URL), cwd = source root — and
reads it the way you would cold (manifest → module nouns → entry point → spine),
then emits, per candidate: a `% trace:` path description, a **one-line spine
sketch** (3–6 rough hop labels, *not* cited checkpoints), and a suggested
`% source:` scope (narrowed when a tight path lives in one file). The sketch is
cheap but rich enough to choose by; you spend a full build only on the one you
pick.

**How many it lists is decided by coverage, not a cap.** It names the central
spine plus *one main path per major subsystem*, and stops when every subsystem is
covered once — so the length tracks the source (a repo with a dozen subsystems
yields about a dozen suggestions; a small one, a few), never a fixed number. It
won't pad to look thorough or drop a real subsystem to stay concise. The local,
leaf paths *inside* a subsystem are deliberately left out — those are the deeper
dives the [explore tier](#goals-and-exploration) sequences. It also **names the
node-shaped subsystems it skips** — the ones that are a table of facts (config, a
store's format) rather than a path — as facts-deck material, so the deferral is a
visible decision, not a silent omission ([trace-vs-deck by shape](#goals-and-exploration)).
So `--suggest` is the
honest **starting set** — the central entry points into understanding the source,
ranked spine-first — *not* the exhaustive set that fully covers it. (That
complete, goal-sized set is explore's job, whose stopping rule is deeper:
saturation — keep going until a new trace would teach no new mechanism.)

```
Source  .  (rust repo · ~40 files)
Spine   Session → build_queue → scheduler

Suggested traces — predict the spine, then `--build` the one you pick:

  1. how a line of deck text becomes a card waiting in the review queue
     spine:  load → parse_str → Card::plain → direction-expand → build_queue
     % source: .
  2. how pressing Good becomes a saved grade
     spine:  keypress → Session::grade → scheduler.advance → store.save
     % source: .
  3. how card.id() stays stable so progress survives edits
     spine:  id() hash inputs → store key → build_queue lookup
     % source: src/card.rs

Paste a block into a new deck, then `alix trace --build <deck>`.
```

`--suggest` is **side-effect-free**: read-only exploration that prints the menu
to stdout and writes nothing. You read it, paste a header into a new
`<topic>.txt`, and `--build` it. Scaffolding those stub decks for you, a faster
recon model, and the full explore hierarchy are all **deferred** — this slice
exists only to test whether the model's selection-and-depth judgment is
trustworthy.

Plumbing reuses Build wholesale: `trace::suggest(source, cfg, ask_cfg)` mirrors
`trace::build` but takes a **source** (not a deck), runs a `suggest_prompt`
through the same read-only `build_run_config`, and returns the menu text;
`--suggest` (conflicts with `--build`/`--map`) reinterprets the positional as the
source and needs no store or scheduler. Validate it like `build_prompt`: a unit
test pins the recon framing (survey-don't-trace, N candidates, spine-sketch not
checkpoints, rank by centrality, narrow scope, menu-only output), then dogfood
`alix trace --suggest .` on this repo and check the top suggestion is a real,
well-scoped spine that roughly matches the traces we already trust
(`examples/keypress-to-grade.txt`, the scratch deck-text→queue path).

## Exploring a source — `alix explore <source>`

Where `--suggest` is the flat trace menu, **`alix explore`** is the full step:
goal-driven exploration that manufactures the *ordered set of means* toward a
goal. It is `--suggest` grown up along the four axes from
[Goals and exploration](#goals-and-exploration):

1. **Goal as input** — `--goal "<aim>"` (default *understand the whole source*).
   The goal scopes coverage: a broad goal covers every subsystem; a narrow one
   (`--goal "how review scheduling works"`) collapses to just that slice — and
   goes *deeper* there (e.g. it splits Leitner and SM-2 into separate traces
   rather than one scheduler deck). Saturation is **goal-relative**.
2. **Means = traces ∪ decks** — it proposes *both*, choosing per part by shape
   (edges → traces, nodes → facts decks), so the node-shaped subsystems `--suggest`
   only names-and-skips (the store schema, config knobs) become decks here. Full
   coverage, the right means each.
3. **`% requires:` ordering** — every item carries the earlier items it builds on,
   and the list is a valid **topological order** (foundations → flows → surfaces),
   so the plan reads as a curriculum, not a bag.
4. **Saturation, not a count** — covers until one more item would teach no new
   mechanism the learner hasn't met.

The output is a plan: a `Goal`/`Source`/`Spine` header, then numbered items each
tagged `[trace]` or `[deck]`, with a title (a path-question for a trace, a fact
topic for a deck), its `requires:`, and a `% source:` scope.

By default it **prints the plan and writes nothing** — like `--suggest`,
reviewable before anything hits disk. With **`--into <dir>`** it also
**materializes** the plan into a workspace folder: an `alix.toml` (the goal + an
empty `[defaults]`) and one stub file per item — a `% trace:` deck for a trace
(its `% trace:` description doubles as the deck's display name, so it needs no
`% title:`), a `% title:` facts deck for a deck — wired by `% requires:` (item
numbers mapped to
the member file names), with each `% source:` rewritten absolute against the
source root. So the plan becomes a real, buildable workspace: `alix trace
--build` each trace, author or `alix generate` each deck, and the `% requires:`
edges gate them in dependency order. (Refuses a non-empty target unless
`--force`.)

**`--into --build` fills the stubs in the same session.** By default `--into`
writes empty stubs; with `--build`, `alix explore` **explores the source once,
then resumes that same CLI session** (`ask::CliSession`: `--session-id` then
`--resume`) to write the full content of every item — predict-verify checkpoints
for each trace, fact cards for each deck — and `materialize` writes that content
in place of the stub comment. Two model calls total (explore + fill) instead of
re-exploring per item, and because the whole set is written from one
understanding the items stay **coherent** (each aware of its prerequisites, no
overlap, consistent terms). It also fills *facts decks* — content `alix generate`
can't yet produce from a code scope — for free. The fill is one-shot (all items
in one response); an item the response omits simply stays a stub, so it degrades
gracefully (a very large plan may want chunking later). Dogfooded: a 7-item
scheduling workspace came out 7/7 filled, every deck and trace valid
(`alix check` + `alix trace --map`).

The walkable *explore* trace (predict the system's shape) and `--grade` come
after.

Plumbing mirrors `--suggest`: `explore::explore(source, goal, cfg, ask_cfg)` runs
an `explore_prompt` through the same read-only `build_run_config` (reusing the
`[trace]` model/timeout for now) and returns the plan text; a top-level `alix
explore <source>` command prints it, and `--into` hands the plan to
`explore::materialize` (which `parse_plan`s it back into items, then scaffolds the
folder). The prompt encodes the four axes above; unit tests pin them (goal echoed,
two kinds of means by shape, saturation, `requires:`/topological order,
plan-not-built) and cover the round-trip (`parse_plan` + `materialize` emit wired
stubs and refuse a non-empty dir). Dogfooded on this repo with a whole-repo goal
(≈19 items across every subsystem) and a narrow one (≈8, scheduling only, →7 wired
stub files), each a valid topological order.

**`--walk` — the explore walk.** Instead of a plan, `alix explore --walk
<source>` builds the *explore walk* (the [decided model](#goals-and-exploration)):
a short predict-verify walk over the source's *shape* — what it is (the manifest)
→ its domain nouns (the module list) → how it's driven (the entry enum) → its
spine (the central file) → the first paths worth tracing (the dispatch map). Each
hop cites real structural evidence as its `% at:`, so it **reuses the whole
`alix trace` walk**: `explore::walk` generates the checkpoints (a new
`walk_prompt`), they're wrapped in a `% trace:`/`% source:` deck with an
absolute root, and `run_walk` — factored out of `alix trace` and shared — walks
them. It writes the trace to a file (`-o`, default `explore.txt`) and walks it
immediately; re-walk later with `alix trace <file>`. Each explore checkpoint is a
normal card, so the exploration itself enters the SRS.

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

## Build (`alix trace --build <deck>`)

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
   can **reorder two checkpoints** without anything breaking, it's a set. And stay
   on the **spine** — the path *every* instance travels. A step that fires only for
   some inputs (a conditional branch, an optional transform like direction
   `both`/`reverse` that a plain forward card skips) is a **side-branch**, not a
   spine hop: trace what all instances do, and if a branch is worth understanding
   make it a **separate (nested) trace** rather than a detour most instances never
   take. (Dogfooding the deck-text→queue trace surfaced this: the
   direction-expansion hop is a no-op for the common forward card, so it read as a
   slight side-quest off the single-card thread.)
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
7. **The reveal is the real source, and each hop is a self-contained unit.**
   Ground truth is a **live excerpt** via `% at:`, never invented; the key points
   paraphrase *exactly those lines* and the question is answerable *from* them.
   The reader sees **only** the cited lines, so an excerpt must read on its own —
   which is really a **granularity** rule, not a citation trick:
   - **Prefer hops that are a whole small function/method.** Its inputs are its
     parameters, so nothing dangles. Don't dissect one big function into several
     checkpoints — that's what produces excerpts whose variables (`state`,
     `first`, …) were bound off-screen, and no amount of extra line-ranges fixes
     it (you patch two and two more dangle).
   - **A big function on the path is one black-box hop:** cite its signature plus
     the load-bearing line(s) and describe what it does in the key points; if its
     internals deserve understanding, they become a **separate (nested) trace**,
     not more hops here.
   - **One `% at:` is one contiguous range** (`file:start-end`), never
     comma-separated. Stitching disjoint ranges collapses the gaps so lines from
     different branches look adjacent — misleading. If a hop won't fit in one
     contiguous span, it spans more than one region: that's the granularity smell
     telling you to split it or black-box the function.
   - **Gloss what you don't show — completely and correctly.** A `% given:` is a
     *free variable*: a symbol the span **uses but does not bind** in the cited
     lines — typically a function **parameter** (declared in the signature, above
     the body you cite) or a value from an enclosing scope. The test is
     mechanical: if the symbol's binding (a `let`, an assignment, the parameter
     itself) is *inside* the cited lines, it is **not** a given — the reader sees
     it; if it's bound *outside*, it **is** one. (So `defaults`, a parameter used
     in a body excerpt, is a given; a `let settings = …` on a cited line is not —
     don't confuse the two.) Name each with a **`% given:` line** (`% given:
     defaults — the workspace directive defaults`, repeatable), shown under the
     question; never cram it into the question or leave it dangling. But **gloss
     only what the reader can't *derive***: a given earns its place when the
     symbol's meaning or origin is genuinely off-screen and not self-evident. A
     self-documenting field or parameter whose name already says what it is
     (`self.subject` on a `Card`) needs none — glossing the obvious just enumerates
     the answer's ingredients and shrinks the predict gap to nothing (dogfooding
     caught a hash hop whose three givens *were* the three hash inputs, so the
     prediction was forced). The list must be **complete** in the honesty sense —
     no *unexplained* dangling symbol — *and* correct: nothing the span binds
     itself. The gloss names the *inputs* (scaffolding); the cited lines stay the
     oracle for the *predicted* claim. More than ~2–3 givens means the hop is cut
     too fine: re-scope it.

   None of this is code-specific: a proof step leans on earlier lemmas and
   notation, a contract clause on its defined terms (a Definitions section *is*
   this gloss), a paper's result on its method — name those givens the same way.
   The model selects the path and the locator; the source is the oracle.
8. **The last hop lands the outcome.** The final reveal is the **payoff that
   completes the `% trace:`** — it reaches the outcome the path was tracing
   toward; the compression step then retraces the whole path. If the last
   checkpoint doesn't reach that outcome, the path stopped short.

Four self-checks for the builder to run before emitting the deck. **Substance:**
each hop's answer must be the actual *mechanism*, not a deferral — if the key
points amount to "it calls X to do it" (e.g. "calls `build_queue` to produce the
queue" for "how is the order decided?"), the real hop is X: dive in and ask the
question there. A circular or obvious answer has no prediction gap, so it teaches
nothing — cut or re-aim it. **Order:** read
the checkpoints top to bottom using **only each prompt + the prior reveal**; if a
hop needs information no earlier hop (or its own excerpt) established, it's out of
order or off the path. **Atomicity:** read each checkpoint **in isolation, as if
it were the only card you saw today**; its prompt must still make sense (state
carried as fact, no "checkpoint N" / "the last two hops" references) — because in
review it often *will* surface alone. **Grounding:** check each key point against
**only its excerpt + givens**; if it asserts behavior not in the cited lines
(another branch, a later call, the return path), the hop is mis-scoped — black-box
it (key points stay at the signature/return contract) or split it so each hop
cites the region its key points describe.

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
	% at: assets/serve/review.html:978-983
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
| `% given: name — meaning` | a **given** (repeatable) — an off-screen symbol the question leans on; shown as a list under the prompt *before* predicting, so the excerpt can stay tight |
| indented lines | the **key points** a good prediction should hit (the rubric) — shown on reveal; what self-grade / `--grade` checks against |
| `% at: <locator>` | the **reveal** — alix reads those lines from the real file and shows them. Ground truth is the source, not the model |
| `! note` | the connective **insight**, shown after the reveal |
| file order | the **path** — checkpoints walked top to bottom |

It degrades gracefully: even without `alix trace`, it's a valid deck of explain
cards. Each checkpoint gets the normal card identity (hash of its key-point
lines), so **SRS attaches per checkpoint for free**.

### `% at:` locator
A locator points at **one contiguous span** — `file:start-end` (or `file:N`),
or just the line numbers when `% source:` is a single file. It is deliberately
**not** comma-separated: stitching disjoint ranges collapses the gaps and makes
lines from different places look adjacent, which misleads — if a hop won't fit in
one span, it spans more than one region and should be split (see rule 7). For a
prose source, a **quoted span** is the universal, verifiable form ("the passage
that says X") — robust to reformatting; `page` numbers are optional precision.
The span is **read live from the source** at session time (always current, keeps
the deck small), with the source as the oracle.

## Grading — self-graded by default, `--grade` opt-in

- **Default (self-graded, offline, free):** on reveal you see the real excerpt +
  the key points and rate yourself Got / Partial / Missed (like flip/explain
  today).
- **`alix trace --grade` (or `[trace] grade = live`):** Claude judges your typed
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
- source deck → the AI exam (source-wide, questions generated from the source).
- **trace deck → the predict-verify walk is the *drill*; the **compression** is
  the *exam*, scoped to the path.**

**SHIPPED (2026-06-25):** the compression is now a real AI-graded exam, not an
ungraded walk-end step. `alix exam <trace>` (and the picker's "Take exam", and the
walk's capstone) asks one fixed question — the `% trace:` — and grades the
learner's two-sentence retrace *holistically* against the checkpoints' key points
(no question generation, no source read; the checkpoints already paraphrase the
source). Passing sets `deck_mastered`, exactly like a fact deck — superseding the
earlier "a trace masters when every checkpoint retires" rule. A fail is re-walked
(no card remediation) and starts a re-sit cooldown (`[exam] retry_cooldown_secs`).
The engine is `exam::Sitting::start_trace` + `exam::grade_compression`, reusing the
whole `Sitting`/`ExamApp`/web-exam machinery with one fixed question. A trace's
`% source:` stays the *locator base*, never an exam corpus.

The original framing below stands: the kind is **derived, not declared**
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
Traces live **inside a workspace** (already built; `alix.toml` manifest). The
explore walk's output (map + child goals) is the workspace's index. `% requires:`
between traces sequences *understanding* via the existing lock/unlock + mastery
machinery — a repo becomes a dependency-ordered set of traces you climb.

```
~/decks/flashcard2/            (a workspace)
  alix.toml                   (title + shared directives; later: trace corpus)
  explore.txt                   (the explore walk → map + menu of goals)
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

CLI first (mirrors how `alix exam` shipped), reusing: `% source:`, the
`claude -p` plumbing, exam's Pass/Partial/Fail + rubric grading, the store +
scheduling, `% mode: explain` cards. The **web trace surface** then followed
(`alix trace <deck> --serve`, the web is the primary surface per ROADMAP L4):
the same frontend-agnostic [`Walk`](../src/trace.rs) state machine drives both,
so the browser is a thin reader over it exactly like the TUI. The walk page
(`assets/serve/walk.html`) renders a left **path rail** (the spine you walk, its
nodes colored by delta) and an editor-style **live excerpt** (the source is the
oracle); `--grade` runs the per-hop Claude grade on a background thread and the
page polls `GET /api/walk` while it's `thinking`, just like the exam.

## Reuse map (existing pieces this builds on)

- `% source:` + WebFetch/file reading (exam).
- `% mode: explain` cards (open prompt + key points + self-grade).
- `exam::Sitting` + `spawn_*` runners + Pass/Partial/Fail rubric grading.
- The store + per-card SRS identity + scheduling; deck states/unlocks (`% requires:`).
- Workspaces + `alix.toml` (the home for a trace set; later a `[trace]` corpus).
- `directories`/`claude -p`/`config` plumbing.

## Suggested build order (phased, de-risked)

The riskiest unknown is **whether the predict-verify trace actually feels like
understanding as a tool** (the conversation was a manual dry-run that felt right,
but a chat ≠ the tool). So validate the mechanic cheaply before heavy plumbing:

1. **Atom / single-source trace:** `alix trace` over one explicit `% source:`,
   predict→reveal→delta→compress, self-graded, on a feature branch. Prove it's
   good.
2. **Build (path discovery)** from a scope; cached trace artifact. Its prompt
   must encode the [generation-prompt spec](#generation-prompt-spec--a-chain-not-a-set)
   (chain-not-a-set) so generated traces are paths by construction.
3. **Suggest (recon menu):** [`alix trace --suggest <source>`](#suggesting-traces--alix-trace---suggest-source-recon-lite)
   — one read-only pass proposes a ranked menu of candidate traces (path + spine
   sketch + scope), no checkpoints. The cheap precursor that closes the authoring
   bootstrap and de-risks the explore tier by proving the recon prompt it reuses.
4. **Explore tier — [`alix explore <source>`](#exploring-a-source--alix-explore-source)**:
   goal-driven, prints a `% requires:`-ordered plan of *means* (traces **and**
   decks, chosen by shape); **`--into <dir>`** materializes it into a workspace
   folder (an `alix.toml` + a stub deck/trace per item wired by `% requires:`);
   **`--walk`** builds and walks the explore walk over the source's shape.
   **Done.**
5. **`--grade`** (live Claude grading of each prediction) — **done**.
6. **Web walk surface** (`alix trace <deck> --serve`) — **done**; the browser
   drives the same `Walk` state machine, with `--grade` as the live option. Next:
   richer `alix.toml [trace]` config, and surfacing trace decks in the web
   workspace picker.

## Snapshotting the source (frozen `assets/`)

`% at: file:lines` reads the **live** source at walk time, so editing a traced
file silently shifts every excerpt to the wrong lines (we hit this:
`examples/keypress-to-grade.txt` drifted after unrelated edits to review.html and
serve.rs). **Quoted-span anchors** (address by text, grep live) were considered
and rejected as the fix — they hit ambiguity (e.g. three `fn apply` in
scheduler.rs), heavy escaping over quote-laden lines, and fuzzy span boundaries.

The fix **flips the oracle from live → a frozen copy.** When you create a
workspace by exploring a source — `alix explore --into <dir> --build` — its last
step **freezes the cited excerpts** into the workspace's `assets/` folder: for
each checkpoint it writes a small snippet file (`assets/01.rs`, `02.rs`, …) holding
just the lines that checkpoint reveals, repoints that `% at:` at the snippet, and
sets the trace's `% source:` to `assets/`. The excerpts then **never drift** — the
snippets are frozen and committed with the workspace. Crucially it copies **only
the excerpts, not whole (possibly huge) source files** — `assets/` stays tiny. The
source is just text — any file, any topic — so a snippet is a plain copy (no
version-control assumption). Bonus: the workspace is **self-contained/portable**
(no upstream checkout needed) and it reuses the `assets/` folder image cards
already need.

A frozen snippet is re-based to line 1, so the original line numbers would be
lost; when they're non-trivial (the excerpt didn't start at line 1) the original
`file:lines` is preserved in the card's **note** (`! from scheduler.rs:90-98`), so
you can still find the passage in the real source.

It's the **default for explored workspaces**, not a command you run: a workspace
created through exploration is self-contained from birth. A loose `.txt` trace
over a live `% source:` is left untouched (the deliberately-live case), and fact
decks keep their `% source:` (no line locators, so nothing drifts). The freeze is
one-way — there's no refresh or un-snapshot. That's by design: a snapshot is
either long-lived material that won't change, or a throwaway workspace (you'd just
re-explore). We don't track staleness until there's a real need.

## Open / deferred decisions

- Whether the explore walk also emits a standalone map artifact vs only the
  in-trace compression output (lean: emit the map as the compression result).
- How `--build` bounds path length / picks the spine on large repos.
- Whether a hand-authored (non-explored) trace ever needs an explicit freeze
  step — deferred until the need shows up.
