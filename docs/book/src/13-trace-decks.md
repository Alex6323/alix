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

A trace is a deck with a `% trace:` (a path description — what it walks, and the
thing that marks the deck a trace) and a `% source:` (the path's origin), then a
sequence of **checkpoint** cards. Each checkpoint is an `explain`-style card — an
open *predict* prompt and the key points a good prediction should hit — plus a
`% at:` locator pointing at the real lines in the source:

```
% trace: how `let s2 = s1` moves a String and avoids a double free
% source: .

# You write `let s2 = s1`. What gets copied onto the stack, and what stays shared?
    Only the stack data — pointer, length, capacity — is copied.
    So s1 and s2 point at the *same* heap allocation.
    % at: src/ch04-01-what-is-ownership.md:290-297
    ! The heap contents themselves are never copied here.

# So s1 and s2 point at one heap allocation. What breaks when both go out of scope, and how does Rust stop it?
    Both would call drop on that memory — a double free.
    Rust treats the assignment as a move: s1 is invalidated, so only s2 frees it.
    % at: src/ch04-01-what-is-ownership.md:322-343
    ! Using s1 after the move is a compile-time error.
```

The `% at:` locator is a single contiguous range `file:start-end` (or just line
numbers when `% source:` is one file) — never comma-separated, since a stitched
excerpt makes disjoint code look adjacent. The lines are **read live from the
source** each walk, so the excerpt is always current and the deck stays small —
the source is the oracle, not an invented answer. When a tight excerpt leans on a
symbol defined off-screen, name it with a `% given:` line (`% given: state — the
parser's position so far`, repeatable); these show as a list under the question,
so the excerpt stays focused without orphaning the names it needs.

## Building it with the model

You don't have to hand-write checkpoints. Declare just the `% trace:` and
`% source:`, then name the stub deck as [`alix generate`](11-generating-decks.md)'s
source:

```sh
alix generate mytrace.txt
```

The model explores the source — **read-only** `Read`/`Glob`/`Grep`, source root as
its working directory, no write or shell access — finds the single load-bearing
path, and writes the checkpoints (with their `% at:` locators) back into the deck.
The result is cached and version-controlled there, so review it (especially the
locators) and edit freely; re-run it to regenerate.

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
a one-line spine sketch, and a suggested `% source:` scope. The list is sized by
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
[phone](18-the-phone-app.md) (the walk runs fully offline there too):
a trace opens as a **walk**: a **path rail** you descend (nodes coloring in by Missed it / Partly /
Got it) with each checkpoint's source shown in a line-numbered excerpt. It goes
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

A trace's `% trace:` is a *question* ("how X becomes Y"). The **exam** is to
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
out** — gated only by `% requires:` (a trace's sourced prerequisites must be
mastered first).

A **failed** trace exam is **re-walked**, not turned into remediation cards (a
trace is a path, not a card pile) — the weak checkpoints already resurface sooner
through their own SRS. After a fail the exam **cools down** for a while before you
can re-sit it, so the graded feedback can't simply be pasted back into the one
fixed question (`[exam] retry_cooldown_secs`, default one hour; `0` disables it).

## Snapshotting

Because `% at: file:lines` reads the **live** source, editing a traced file would
shift every excerpt to the wrong lines. So when you create a workspace by exploring
a source ([`alix generate <dir>`](14-explore.md)), its final step
**freezes** the cited excerpts into the workspace's `assets/` folder — one tiny
snippet per checkpoint — and repoints each `% at:` at them, so they never drift and
the workspace is self-contained, without copying whole files. The freeze also
records the live source root in an `% origin:` directive and keeps each snippet's
original location on its `% at:` line, after ` from ` (`% at: 12.rs from
scheduler.rs:90-98`) — so the tutor can still reach the real source and
[`alix doctor`](17-command-reference.md) can tell when it has drifted. It's
automatic for explored workspaces; a loose trace over a live source is left as-is.

## Checking the locators

For a trace that *isn't* frozen — a loose `.txt` over a live `% source:` —
[`alix doctor <deck>`](17-command-reference.md) validates that every `% at:` still
resolves into its source: it warns about a locator that names a missing file,
runs past the end of the file, or (for a single-file source) gives bare line
numbers it can't place. It's a quick structural check — *does this excerpt still
exist?* — so a moved or trimmed source is caught before you walk into it, not
mid-hop. (Frozen snapshots don't move, but `alix doctor` still validates each
snippet — and, via the `% origin:` they were frozen with, warns when the live
source has **drifted** from the frozen excerpt.)

A trace deck degrades gracefully — even outside a walk it's a valid deck of
`explain` cards. See `docs/examples/rust-ownership/ownership-move.txt` for a complete
trace — a frozen snapshot over The Rust Book's ownership chapter, so it walks offline.
