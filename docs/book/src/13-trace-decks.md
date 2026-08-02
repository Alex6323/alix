# 13 · Trace decks

> **Experimental.** Traces are new and still evolving — the deck format and the
> flow may still change.

Cards drill *facts* — the nodes of what you know. A **trace** drills the
*connections between them* — the edges — by walking a **path** through a real
source and making you **predict each hop before it's revealed**. Where the
[AI exam](12-the-ai-exam.md) verifies a *set* of independent answers, a trace
verifies you can follow one chain of reasoning, and the gap between your
prediction and the truth is where the understanding forms.

This is the most direct expression of the book's [opening bet](01-why-alix.md):
understanding is the chain of *because-this-therefore-that*, and a trace makes you
build that chain yourself.

## What a trace looks like

A trace is a deck with a `trace:` (a path description — what it walks, and the
thing that marks the deck a trace) and a `source:` (the path's origin), then a
sequence of **checkpoint** cards. Each checkpoint is an `explain`-style card — an
open *predict* prompt and the key points a good prediction should hit — plus a
`<!-- at: -->` locator pointing at the real lines in the source:

```
---
trace: how `let s2 = s1` moves a String and avoids a double free
source: .
---

## You write `let s2 = s1`. What gets copied onto the stack, and what stays shared?
Only the stack data (pointer, length, capacity) is copied.
So s1 and s2 point at the *same* heap allocation.
<!-- at: src/ch04-01-what-is-ownership.md:290-297 fingerprint: xxh64-0123456789abcdef -->
> The heap contents themselves are never copied here.

## So s1 and s2 point at one heap allocation. What breaks when both go out of scope, and how does Rust stop it?
Both would call drop on that memory (a double free).
Rust treats the assignment as a move: s1 is invalidated, so only s2 frees it.
<!-- at: src/ch04-01-what-is-ownership.md:322-343 fingerprint: xxh64-123456789abcdef0 -->
> Using s1 after the move is a compile-time error.
```

The trace description, checkpoint prompt, `given` values, key points, and note
all support the same inline Markdown and LaTeX rendering as ordinary cards.
Inline-code terms in a checkpoint's key points are also highlighted wherever
they occur in its revealed source excerpt. Matching is exact and case-sensitive,
so the author controls the emphasis by choosing which terms to put in backticks.

The `<!-- at: -->` locator's `at:` field is a single contiguous range
`file:start-end` (or just line numbers when `source:` is one file), never
comma-separated, since a stitched excerpt makes disjoint code look adjacent. Its
`fingerprint: xxh64-...` field fingerprints the displayed lines. A live walk
reveals the source only while that fingerprint matches, so a shifted numeric
range cannot silently show unrelated lines. When a
tight excerpt leans on a
symbol defined off-screen, name it with a `<!-- given: -->` line (`<!-- given: state — the
parser's position so far -->`, repeatable); these show as a list under the question,
so the excerpt stays focused without orphaning the names it needs.

## Building it with the model

You don't have to hand-write checkpoints. Declare just the `trace:` and
`source:`, then name the stub deck as [`alix generate`](11-generating-decks.md)'s
source:

```sh
alix generate mytrace.md
```

The model explores the source — **read-only** `Read`/`Glob`/`Grep`, source root as
its working directory, no write or shell access — finds the single load-bearing
path, and writes the checkpoints (with their `<!-- at: -->` locators) back into the deck.
Alix fingerprints those locators before placing the result. The result is
cached and version-controlled there, so review it (especially the locators) and
edit freely; re-run it to regenerate.

Building is one-shot, correctness-critical, and **fails silently** when the model
is weak — you still get parseable checkpoints, just a loose chain you then drill.
So the `[trace]` config defaults the build to a strong model (`model = "opus"`)
and high reasoning effort (`effort = "high"`): slower than the other AI features,
but it runs once and is amortized over many reviews. The suggestions pass
(`--trace --plan`, below) shares those settings; walk grading (`[trace]
auto_grade`) does not (it's a light per-hop call at the tutor tier).

## Don't know what to trace? — `--trace --plan`

```sh
alix generate . --trace --plan
```

does a single read-only recon pass over a source (a repo `.`, a directory, a file,
or a URL) and prints a **ranked menu of candidate traces** — each a path-question,
a one-line spine sketch, and a suggested `source:` scope. The list is sized by
**coverage** (the central spine plus one main path per major subsystem), so it's
as long as the source needs. It also names the *node-shaped* subsystems it skips —
a config table, a store's on-disk format — as **facts-deck material**, because
facts are a deck's job and edges are a trace's. It writes nothing: pick one, paste
its header into a new deck, and `alix generate` it. Knowing *what* is worth tracing (and
how deep) is the genuinely hard part — it needs you to already understand the
source — so this hands that judgment to the model.

## Write it as a chain, not a quiz

A trace's whole value is that it's a *path*: each checkpoint picks up where the
last *reveal* left off (notice how hop 2 above opens with hop 1's conclusion, "s1
and s2 point at one heap allocation"), so you follow one thread — a data flow, a control flow, a
derivation — to an outcome. If the checkpoints are independent facts hanging off
one thing, you've written a *set*, which is what cards and the exam already do;
choose a subject with a real sequence instead.

## Walking it

Pick the trace in the [web picker](15-the-web-app.md), or on the
[mobile app](18-the-mobile-app.md) (the walk runs fully offline there too):
a trace opens as a **walk**: a checkpoint-by-checkpoint
descent (the hop list rides the wire but is not yet rendered as a rail) with each checkpoint's source shown in a line-numbered excerpt. It goes
hop by hop:

1. **Predict** — type a guess before anything reveals (committing is the point).
2. **Reveal** — `alix` shows the real excerpt from the source, then the key points
   and note.
3. **Gap** — you judge yourself **Missed it / Partly / Got it** (the same three
   grades review uses). Self-judged and offline by default; set **`[trace]
   auto_grade = true`** in the [config](16-configuration.md) to have the model
   judge your typed prediction against the key points and return a verdict plus a
   line of feedback (a model call per hop; a desktop/web setting, since the
   phone's walk is always self-judged). Either way, a failed or partly hop is
   a **weak edge** that resurfaces sooner — a failed one resets, a partly steps
   back one stage — while a passed hop advances and fades. Each checkpoint is an
   ordinary card underneath, so this is the normal per-card SRS.
4. **Done** — after the last hop the walk is complete. That's the *drill*; the
   *verification* (what masters the trace) is its separate **exam**, below.

## The exam — the compression

A trace's `trace:` is a *question* ("how X becomes Y"). The **exam** is to
answer it: retrace the whole path in a sentence or two, from memory. The model grades
that compression against the path's checkpoints (AI-graded, exactly like a
[fact deck's exam](12-the-ai-exam.md)) and
**passing masters the trace**, which unlocks its dependents. So the symmetry is:

- walking the checkpoints (predict → verify each edge) is the **drill**;
- the compression is the **exam**.

You reach it in the browser: the **capstone** offered at the end of a walk
(`Take the exam?`), or the picker's
**"Take exam"** button. A [paired phone](19-pairing.md) offers the same
capstone from its own walk. Like a fact deck, you can sit it **early to test
out** — gated only by `requires:` (a trace's sourced prerequisites must be
mastered first).

A **failed** trace exam is **re-walked**, not turned into remediation cards (a
trace is a path, not a card pile) — the weak checkpoints already resurface sooner
through their own SRS. After a fail the exam **cools down** for a while before you
can re-sit it, so the graded feedback can't simply be pasted back into the one
fixed question (`[exam] retry_cooldown_secs`, default one hour; `0` disables it).

## Immediate freezing

Because `<!-- at: file:lines -->` reads the **live** source, editing a traced
file could shift every excerpt to unrelated lines. Initializing any workspace
member therefore freezes its evidence immediately. An explicitly named source
file is copied in full. A source directory is reduced to the excerpts cited by
its cards, so Alix does not export an entire repository.

Every copied excerpt lives below `assets/deck-<token>/` and is named
`sha256-<digest>.<ext>`, where the digest covers its exact stored bytes. Freezing
leaves the deck's `source:` pointed at the real material (a path or a URL): it is
never rewritten to point into `assets/`. Each `<!-- at: -->` keeps its real
`at:` path and lines, keeps its excerpt `fingerprint:`, and gains an `asset:`
field naming the content-addressed object:

```markdown
<!-- at: scheduler.rs:90-98 fingerprint: xxh64-0123456789abcdef asset: sha256-<digest>.rs -->
```

Review reveals the frozen excerpt (display evidence, numbered from the `at:`
start line), and the `fingerprint:` verifies those stored bytes. The `at:` path
and the deck's `source:` retain provenance for drift reporting and a future
deliberate source update. When the live source is available and permitted, the
tutor and exam may consult it for surrounding context and staleness detection;
offline, they report the missing live source rather than silently degrading. A
loose trace over a live source is left as-is.

## Checking the locators

For a trace that *isn't* frozen — a loose `.md` over a live `source:` —
[`alix doctor <deck>`](17-command-reference.md) validates that every
`<!-- at: -->` still resolves and matches its fingerprint. A missing fingerprint,
missing file, changed excerpt, or ambiguous exact match is reported without
writing. If the exact text moved to one other range, doctor reports that safe
rebase. After reviewing it, run
`alix doctor <deck> --repair-source-locators` to stamp missing fingerprints and
apply only unique exact rebases. Changed and ambiguous excerpts remain
untouched. Frozen assets do not move, but doctor still verifies their
captured text and separately reports live-source drift.

A trace deck degrades gracefully — even outside a walk it's a valid deck of
`explain` cards. See `docs/examples/workspace-showcase/decks/ownership-move.md`
for a complete trace, frozen evidence from The Rust Book's ownership
chapter, so it walks offline.
