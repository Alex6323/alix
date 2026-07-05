# `alix`

[![CI](https://github.com/Alex6323/alix/actions/workflows/ci.yml/badge.svg)](https://github.com/Alex6323/alix/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/alix)](https://crates.io/crates/alix)
[![docs.rs](https://img.shields.io/docsrs/alix)](https://docs.rs/alix)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/alix)](https://crates.io/crates/alix)

**DISCLAIMER**
This is WIP - don't use it for serious learning just yet. There will be breaking
changes to the deck format, to the progress store, and what not. Most likely you'll
lose all your progress and I won't provide a migration path. You have been warned!

Your **personal AI tutor**, built for understanding — not just remembering.
Under the hood it's a plain-text spaced-repetition trainer — FSRS
scheduling, several answer modes, cloze and dual-direction cards, images, and
deck dependencies — and the layer on top is the **AI integration**: a
*tutor* on any card, *AI deck generation* from a web page,
*understanding cards*, and an **AI exam** (`alix exam`) that checks whether you
grasped the material — not just recalled it — and gates your progress on
passing. Decks stay simple plain-text files you own, reviewable in a ratatui
terminal UI or a local web app.

## Requirements

The flashcard **core** — review, scheduling, every answer mode, browse, the TUI
and the web frontend — runs standalone, with no external services or accounts.

The **AI features** shell out to a supported model CLI — [Claude Code](https://www.anthropic.com/claude-code)
by default; the Gemini, Codex, and Copilot CLIs are also supported (see
[Backends](#backends)). Install at least one and authenticate with it. The
features that require a model CLI:

- `alix deck generate` — generate a facts deck from a URL or a local
  file/directory; `alix deck augment` — add AI distractors or notes to one;
- `alix exam` — the AI exam;
- `alix trace --build` / `--suggest` / `--grade` — discover, suggest, and grade
  traces;
- `alix explore` — goal-driven learning plans;
- the in-session tutor (`?`), in both the TUI and the web frontend.

`alix` invokes the CLI headless under a locked-down permission model (see
[The Tutor](#the-tutor)); the command, model and
timeouts are configurable per feature in the [config file](#configuration).

## Learn a codebase (the main workflow)

`alix`'s main use: point it at a repo (or any source) and it builds a
self-contained **learning workspace** — facts decks and predict-and-verify
[traces](#traces-alix-trace), dependency-ordered — that you then study with
spaced repetition, the AI tutor, and the [exam](#the-ai-exam-alix-exam). Three
steps:

```sh
# 1. Preview the plan (read-only): the decks and traces worth authoring.
alix explore ./my-crate --goal "how the request pipeline works"

# 2. Build the workspace in one pass: stubs filled, sources frozen into assets/.
alix explore ./my-crate --goal "how the request pipeline works" \
  --into ~/decks/request-pipeline --title "Request pipeline" --build

# 3. Study it — in the terminal, or the browser with --serve.
alix workspace ~/decks/request-pipeline    # or: alix --serve, then open it
```

`--goal` scopes what gets authored (and becomes the workspace's description),
`--title` names it, and `--build` fills every facts deck and trace in one
coherent pass — freezing the cited source into the
workspace so its line locators never drift. Inside the workspace a **facts deck reviews** and a
**trace walks** (predict → reveal → judge the gap), unlocking in dependency
order, with progress kept in the workspace's own store. See
[Exploring a source](#exploring-a-source--alix-explore) and
[Workspaces](#workspaces) for the full details. (The AI steps need a model CLI
— see [Requirements](#requirements).)

## Usage

The binary is called `alix`:

```sh
alix                            # pick decks interactively (recent + ~/decks)
alix mydeck.txt                 # review due cards
alix --cram mydeck.txt          # review everything now (a correct answer refreshes, doesn't reward)
alix browse mydeck.txt          # read through cards, no grading or scheduling
alix deck generate <url-or-path>   # generate a facts deck from a web page or a file/dir
alix deck augment mydeck.txt --target choices   # AI distractors (cached; review reads them)
alix deck augment mydeck.txt --target notes --with "add trivia"   # AI notes
alix deck augment mydeck.txt --target keypoints   # decompose answers into an explain-mode checklist
alix import cards.tsv            # import an Anki TSV (front<TAB>back) into a deck
alix exam mydeck.txt            # AI exam against the deck's % source: (gates unlocks)
alix trace mytrace.txt          # walk a predict-and-verify path through a % source:
alix trace --build mytrace.txt  # let the model discover the path (writes checkpoints back)
alix trace --suggest .          # recon a source for candidate traces worth authoring
alix explore .                  # an ordered learning plan (decks + traces) toward a goal
alix explore --walk .           # walk an explore tour of the source's shape
alix deps mydeck.txt            # edit a deck's prerequisites (checkbox picker)
alix stats mydeck.txt           # progress overview
alix list mydeck.txt            # every card with stage and due time
alix deck check mydeck.txt      # lint a deck (syntax, duplicates, trace locators)
alix reset mydeck.txt           # clear stored progress (also --card / --all)
```

A session is one deck file — review them one at a time. Useful flags for
`review`: `--new N` (max unseen cards to introduce, default 10) and `--limit N`
(cap session size). How each card is checked isn't a flag: it comes from the
card's authored [`% reveal:`](#deck-directives) method and your personal
[`[review] target`](#review-pacing) depth (see [Review](#review)).

Run `alix` with no deck arguments (as the desktop launcher does) to open the
**deck picker**, grouped into three sections: **[Workspaces](#workspaces)**
(each showing when it last made progress) · **Recent** (loose decks you reviewed
lately) · **Folders** (plain decks folders). A deck that lives inside a workspace
stays out of Recent — you reach it by opening its workspace. Mastered/done decks
are kept out of Recent (it's a quick launchpad) but stay reachable by filtering;
an exam-locked deck that's still drillable stays in Recent. Decks live in the
decks directory (`~/decks` by default,
set `decks_dir` in the config). The focus is on the **list** by default, with
Vim-style keys (rebindable in the config's `[keys.picker]` section): `j`/`k` (or
`↑`/`↓`) move, `l` (or `Enter`) opens the focused row, `h` (or `Esc`/`Backspace`)
steps back, `m` opens the **Mastered** window (your completed decks, kept out of
Recent), and `/` (or `Ctrl-F`) starts **filtering** by name (searching *every*
loose deck, not just the recent ones); `Esc` leaves the filter. Jumping to the
first/last row stays fixed at `g`/`G` (or Home/End), like the `[keys.browse]` pager.
A deck with nothing to launch right now is dimmed and `Enter` on it does nothing:
🕒 nothing due (`--cram` reviews it anyway), or a fully-drilled
deck whose exam is locked. A 🔒 marks a deck whose
[exam is locked](#completion-states--unlocks) (a sourced `% requires:` isn't
passed yet) — but it stays **drillable**, so `Enter` still reviews it if cards are
due. A mastered deck reads `mastered 🎉`. `Enter` on a **Workspace** or **Folder** opens it (drills in) — `Esc`
or `Backspace` steps back out to the list, all within one screen; a
[trace](#traces-alix-trace) opened from the picker **walks** instead of being
reviewed as cards. When you finish a deck/walk/exam launched from a workspace, you
land **back in that workspace** to pick the next one.

## Deck format

The deck format:

```
% This line is a comment.

# What is the front of the card? (this is the front)
    The answer that must be typed. (back, may span several lines)
    A second answer line.
    ! An optional note, shown after answering.
    ! Further !-lines extend the note (multi-line notes).

# Next card
    \# A back line starting with a markup char is escaped with a backslash.
```

A note is shown as a quoted block: each sentence on its own line. A
` ```fenced``` ` block inside a note is rendered verbatim instead — its
indentation is preserved and it is not reflowed, so code stays readable.

Lines are trimmed, so indentation never has to be typed. A `#` only starts
a new card at column 0 — indented `#` lines are answer content (shell
comments, Rust attributes, Dockerfile comments...), no escaping needed.
Notes (`!`) and comments (`%`) work at any indentation.

### Directives at a glance

Every card marker and `% key: value` directive in one place. **Scope** is where
each may appear — *deck* = the header (before the first card), *card* = after a
card's front. Follow a link for the full explanation.

| Token | Scope | Meaning |
| --- | --- | --- |
| `#` front | card | Starts a card at column 0; the indented lines below are the answer. |
| `!` line | card | A note shown after you answer. |
| `%` line | anywhere | A comment — ignored, unless it is one of the directives below. |
| `% reveal:` | deck · card | [How the answer is uncovered](#deck-directives): `flip` (default), `cloze` ([fill-in-the-blank](#cloze-cards-fill-in-the-blank), `{{spans}}`), or `line` (one line at a time). |
| `% input:` | deck · card | Answer input: `type` (default) or [`draw`](#review) — draw/handwrite the answer on a web canvas instead of typing, then self-grade. Web-only; honored on self-graded `flip`/`explain` cards; ignored elsewhere and in the TUI. |
| `% order:` | deck | [Card order](#deck-directives): `scheduled` (default) or `sequential`. |
| `% direction:` | deck · card | [Review direction](#dual-direction-cards--direction): `forward`, `reverse`, or `both`. |
| `% frontend:` | deck · card | [Restrict](#deck-directives) a card/deck to `any`, `tui`, or `web`. |
| `% img:` / `% img-back:` | card | [Image](#images--img--img-back) on the front / back (web frontend). |
| `% img-dir:` | deck | [Base directory](#images--img--img-back) that image filenames resolve against. |
| `% strictness:` | deck | [Exam grading rigor](#the-ai-exam-alix-exam): `strict`, `balanced`, or `lenient`. |
| `% requires:` | deck | [Prerequisite deck](#deck-dependencies) that gates unlocks (repeatable). |
| `% link:` | deck | [tutor reference](#the-tutor) URL — **tutor only** (repeatable). |
| `% source:` | deck | [Exam ground truth](#the-ai-exam-alix-exam) — a URL or file (repeatable). For a [trace](#traces-alix-trace), the source the path runs through (the frozen `assets/` copy in an explored workspace). Also a tutor reference. |
| `% trace:` | deck | What a [trace](#traces-alix-trace) walks — a path description ("how X becomes Y"); its presence makes the deck a trace. |
| `% at:` | card | A locator into the `% source:` (`file:lines`, or just `lines` for a single-file source): a [trace checkpoint's](#traces-alix-trace) reveal target, or a [fact card's source citation](#source-citations--at-on-a-fact-card) shown on reveal. In a frozen workspace it points at the `assets/` snapshot and carries the original location after ` from ` (`29.rs from src/caching.rs:46-66`). |
| `% origin:` | workspace · deck · card | The live source root a frozen deck's snapshots came from (set in a workspace's `alix.toml` at build time). The tutor grounds in it for context and `alix deck check` reads it to flag drift; `% source:` itself points at the frozen `assets/`. |
| `% given:` | card | A [trace checkpoint's](#traces-alix-trace) "given" (repeatable) — an off-screen symbol the question leans on, as `name — meaning`; shown as a list under the question. |
| `% title:` | deck | [Display name](#workspaces) shown instead of the file name (a workspace sets `title` in its `alix.toml`). |

**`% link:` vs `% source:`** — both point at the material a deck is about, but
they are not interchangeable. `% source:` is the **exam's ground truth**:
questions are generated from it and answers graded against it, and a URL source
*also* doubles as a tutor reference. `% link:` is **only** a tutor
reference and never becomes exam material — use it for supplementary reading (a
blog post, a Stack Overflow answer) you don't want the exam to test. The
implication runs one way: a `% source:` URL is offered to the tutor, but a
`% link:` is never promoted to an exam source.

### Deck directives

A deck can set its own defaults with `% key: value` comment lines in the deck
header (before the first card), so you do not have to repeat flags on the
command line:

```
% reveal: line
% order: sequential
```

- `reveal` — how a card's answer is uncovered: `flip` (default, all at once),
  `cloze` ([fill-in-the-blank](#cloze-cards-fill-in-the-blank); `{{spans}}` in the
  answer mark the gaps), or `line` (one line at a time); can also be overridden per
  card (see below).
- `order` — `scheduled` (the default) or `sequential` to walk the deck in
  file order, top to bottom (ideal for lyrics with `% reveal: line`).
- `direction` — `forward` (default), `reverse`, or `both`; per card or deck-wide
  (see below).
- `frontend` — `any` (default), `tui`, or `web`; restricts a card (or deck) to a
  frontend. Image cards are `web` automatically (see Images below).
- `img-dir` — directory that card `% img:` / `% img-back:` filenames resolve
  against (deck header only; see Images below).
- `strictness` — how strictly the AI exam grades answers (`strict`, `balanced`,
  `lenient`); only affects `alix exam` (see [the AI exam](#the-ai-exam-alix-exam)).

These are ordinary `%` comments, so they don't affect parsing and card hashes
are unaffected. An explicit CLI flag always wins over a directive, which wins over
the built-in default. Directives are read only from the deck(s) you ask to
review. When several requested decks disagree on a setting, the default is used.
`alix deck check <deck>` prints a deck's directives.

**Per-card reveal.** A `% reveal:` directive placed *after* a card's front (and
before the next one) overrides the deck's reveal-method for that card only, so one
deck can mix them — e.g. a `line` lyrics card among `flip` cards. It resolves per
card: the card's `% reveal:` > the deck's `% reveal:` > the default (`flip`). The
`order` directive stays deck-level. `cloze` is effectively a per-card method: a
card is turned into fill-in-the-blanks only from its *own* `% reveal: cloze` (with
`{{spans}}` in the answer), so a deck-wide `% reveal: cloze` default won't convert
plain cards — mark it on the cards that have gaps.

**Depth is a separate axis — and not a deck directive.** `% reveal:` is *how* an
answer is shown; how *deeply* you're asked to retrieve it (recognize / recall /
reconstruct) is the learner's choice, not the author's, so it lives in your
personal config as [`[review] target`](#review-pacing), never in a shared deck.
The two combine into the check you actually get — see [Review](#review).

### Deck dependencies

A deck can declare prerequisite decks with `% requires:` lines (repeatable):

```
% requires: rust-basics
% requires: rust-ownership
```

`% requires:` declares *order and gating*, not session contents: reviewing or
browsing a deck uses exactly that deck's cards — prerequisites are never pulled
in. A prerequisite name resolves next to the requiring deck or in the decks
directory, with or without `.txt`; a missing prerequisite or a dependency cycle
is non-blocking (it never hides a deck). They are ordinary `%` comments, so
hashes are unaffected.

What dependencies *do* drive is the picker's **dependency tree** (foundations
shown first) and the **exam gate**: a deck with a `% source:` can't sit its exam
until its sourced prerequisites have passed theirs — see
[Completion states & unlocks](#completion-states--unlocks). Drilling is never
gated, so you can review any deck at any time.

You can edit a deck's prerequisites without hand-typing (and without typos)
with `alix deps <deck>` (alias `alix require`): it opens the deck picker over your
decks directory, pre-ticked to the current prerequisites. `Space` toggles,
`Enter` saves (rewriting the `% requires:` lines), `Esc` cancels; unticking
everything clears them. Since the lines are comments, editing dependencies
never affects card progress.

### Completion states & unlocks

Every deck has a **completion state**, derived from how far its cards have
progressed: *not started* (no card reviewed), *finished* (every card has
**graduated** — reached FSRS's review phase, past the initial learning steps),
or *started* (in between). The deck picker (terminal and web) shows it on each
row — `new`, `m/total` (graduated cards), or `done ✓` — and `alix stats`
prints it too. A deck that declares a `% source:` adds one more state between
drilled and finished — *exam due* (`exam due`, tinted) — because drilling alone
no longer finishes it; see [the AI exam](#the-ai-exam-alix-exam).

Completion drives **unlocks**, with no extra syntax — but the gate is the
**exam**, not drilling. A sourced deck's **exam is locked** while any of its
*sourced* `% requires:` prerequisites hasn't passed *its* exam; passing a
foundation's exam unlocks the exams that build on it. A **source-less**
prerequisite has no exam to pass, so it never gates — its `% requires:` edge is
purely informational (a suggested order in the dependency tree). Crucially, the
lock never blocks **drilling**: you can review any deck at any time, in any order
— you drill only to prepare for an exam. A deck whose exam is locked shows a 🔒
but stays drillable; the 🔒 just means "its exam isn't available yet." State and
locks are recomputed live, so if a foundation later lapses, its dependents' exams
lock again.

### Workspaces

A **workspace** is a folder of decks reviewed together with shared directives —
ideal for a cluster like all your vocabulary decks. A folder becomes a workspace
when you drop a **`alix.toml`** in it — a scoped version of the
[config file](#configuration) — setting a `title`, an optional one-line
`description`, an optional `source_access` override (whether the
[grounded tutor](#the-tutor) may read this workspace's source,
beating the global `[ask] source_access`), and a `[defaults]` table of directives
shared by every deck:

```toml
# ~/decks/english/alix.toml
title = "English"
description = "everyday conversational vocabulary"
# source_access = true   # let the tutor read this workspace's % source:
# icon = "assets/logo.svg"  # a picker emblem (else assets/icon.*; SVGs are themed)

[defaults]
direction = "both"
reveal = "line"
```

```
alix workspace ~/decks/english/     # open the workspace; pick a member to review
```

The `[defaults]` keys are the deck directive names, and they fill in only what a
deck *doesn't* set itself, so precedence runs **CLI flag > card > deck >
workspace > default** — set `direction = "both"` once for the whole cluster, and
an individual deck can still override it with its own `% direction:`.

**A workspace keeps its own progress.** Its decks track their progress in a
`progress.json` *inside the workspace folder* (override the path with a
`store = "..."` line in the `alix.toml`), separate from the global store every
loose deck shares. So a workspace is a **self-contained, portable unit** — its
decks, its `assets/` (frozen trace excerpts), and its progress all live in one
folder you can move or share, and its history stays isolated from everything
else. Decks *outside* a workspace keep using the global store; `--store <path>`
overrides either.

**Personal pacing per workspace.** Drop an `alix.local.toml` beside the
`alix.toml` to override the global `[review]` config (FSRS `retention`,
`retire_after`, and the ladder `target`) for this workspace's decks only. It uses the same `[review]` keys
as the [config file](#configuration) and is **personal** — kept separate from the
shared `alix.toml`, so it never travels when you share the workspace.

**A workspace can carry an icon** shown next to it in the web picker, for quick
recognition in a long list. Drop an image in the workspace's `assets/` and point
`icon = "assets/<file>"` at it (or just name it `assets/icon.{svg,png,jpg}` and
skip the key); an **SVG is tinted to the active theme**, a raster shows as-is.
Building a workspace with `alix explore --into <dir> --build` draws an abstract
SVG emblem from the topic automatically, unless you pass `--icon <file>`.

**Workspaces** and plain **Folders** appear in their own picker sections
(terminal and web): a folder *with* an `alix.toml` shows under **Workspaces**, one
*without* as a plain **Folder**. Both open (drill in) to their
members drawn as an **unlock dependency tree**: a deck nests under the
`% requires:` prerequisite that gates it, foundations at the roots, and siblings
ordered startable-first. Each row is badged `· trace ·` or `· deck ·`. The drill-in
is a **single-launch list** (no checkboxes): `Enter` on a facts deck reviews it,
`Enter` on a trace **walks** it — one deck per session, never the whole folder at
once. (Typing a filter flattens the tree to a plain search.) A manifest-less
folder works the same way — open it (`alix workspace <folder>`) and pick a
member; it just applies no shared directives.

**`alix workspace <dir>`** opens a workspace straight into that same drill-in
picker, routing each member to the right experience — a **facts deck** to a
review, a **trace deck** to a [predict-verify walk](#traces-alix-trace) —
returning you to the picker when done. (A session is one deck file, so a whole
workspace is never reviewed at once — open it and pick a member.)

**`% title:`** (on a deck) or **`title`** (in a workspace's `alix.toml`) gives a
display name, shown in the picker, the session header, `alix list` and `alix
stats` instead of the file name. It's display-only — you still refer to decks by
file path on the command line — and never affects a card's identity.

### Cloze cards (fill in the blank)

A card marked `% reveal: cloze` becomes a cloze card: every `{{...}}` in its
answer lines is a hole, and the card expands into one card per hole. Each one
shows the answer with that hole blanked out and the others filled in, and you
only produce the hidden text:

```
# Complete the Rust declaration
    % reveal: cloze
    let {{mut}} x: {{u64}} = 0;
```

This makes two cards: `let ____ x: […] = 0;` (type `mut`) and
`let […] x: ____ = 0;` (type `u64`). The asked hole shows `____`; the other
holes are hidden as `[…]` so no card reveals its siblings' answers, and the
session queue keeps sub-cards of the same source card apart whenever other
cards are available. Only the doubled `{{` / `}}` are special — a lone `{` or
`}` is literal, so code like `let p = Foo {};` is fine in a cloze answer (write
a literal `{{` as `\{\{` if you ever need one). Progress of a cloze card
survives rewording its front and even a future change to the hole markup, but
editing its answer text or hole contents resets the affected holes.

A cloze needs surrounding text to recall *from*: if the whole answer is a
single hole with nothing around it (e.g. `` `{{IdentStr}}` ``), `alix deck check`
rejects it — that is a plain `#` card in disguise, so write it as one. A lone
hole is fine the moment the answer has other words around it, and answers with
two or more holes are always allowed (each hole's siblings, shown as `[…]`,
give it context).

### Dual-direction cards (`% direction:`)

A `% direction:` directive reviews a card both ways — useful for vocabulary and
other reversible facts:

```
# purported
    angeblich
    % direction: both
```

`both` makes two cards (`purported → angeblich` and `angeblich → purported`);
`reverse` keeps only the swapped one; `forward` (the default) is the card as
written. It works per card, or deck-wide as a header directive (`% direction:
both` before the first card) with per-card overrides — like `% reveal:`. The two
directions get distinct progress, are kept apart in the queue (you won't see one
right after the other), and are removed together. The reversed card keeps the
note. Best for single-line cards; it does not apply to cloze cards.

### Images (`% img:`, `% img-back:`)

A card can show an image on the question side with `% img:`, the answer side
(revealed with the back) with `% img-back:`, or both:

```
% img-dir: /home/me/decks/img

# What phase is this?
    % img: moon-waxing.png
    Waxing gibbous

# Play this chord
    G major
    % img-back: g-major-tab.png
```

The deck-level `% img-dir:` (header only) is the directory filenames resolve
against; it may be absolute or relative to the deck file. Without it, filenames
resolve against the deck file's own folder. A card value that is itself an
absolute path is used as-is. One image per side.

Images render in the **web frontend only** — the terminal can't draw them — so
an image card is automatically *web-only* (as if it declared `% frontend: web`).
In the terminal, `alix review` skips such cards with a note, and if a whole
deck is web-only it points you at `--serve`. Use `% frontend:` to force a card
or deck to a frontend explicitly. `alix deck check` warns about a referenced image
file that doesn't exist (it doesn't fail the check).

### Source citations (`% at:` on a fact card)

A plain fact card can cite where its answer comes from. Add a `% at:` locator
pointing into the deck's `% source:`, and on reveal the card offers to swap its
worded answer for the exact source lines:

```
% source: src/string.rs

# What does the `String` struct hold?
    A `Vec<u8>` (its bytes).
    % at: src/string.rs:1-3
```

The locator takes the same form a [trace checkpoint](#traces-alix-trace) uses —
`file:lines` (e.g. `src/string.rs:1-3`), or just `lines` when the `% source:` is
a single file. On reveal a `</>` marker appears on the answer: in the web,
**click the answer** (or press `s`) to swap it for the line-numbered excerpt, and
again to swap back; in the terminal, press **`s`**. The excerpt is read **live**
from the source, so a moved or missing file shows *"source unavailable"* rather
than a stale quote. `% at:` is a comment to the scheduler, so adding it never
changes a card's identity or resets its progress.

You can write `% at:` by hand, but the deck generator adds them for you:
[`alix deck generate <local source>`](#generate-a-facts-deck--alix-deck-generate) and
[`alix explore --build`](#exploring-a-source--alix-explore) cite the lines each
fact came from, and `alix deck check` warns about a citation that no longer resolves
(a moved or shrunk file). In a workspace built with `alix explore --into
--build`, the cited excerpts are also **frozen** into `assets/` (like trace
excerpts), so they never drift and the workspace travels without the upstream
source.

## Review

Two things decide how a card is checked, and alix derives the check from both —
you don't hand-pick a "mode" per card:

- the card's authored **reveal-method** (`% reveal:` — *how* the answer is
  uncovered), and
- your personal **target depth** (`[review] target` — *how deeply* you're asked to
  produce it).

**Reveal-methods** (authored, `% reveal:`, default `flip`) are `flip` (reveal the
whole answer at once), `cloze` (reveal with a gap to fill —
[cloze cards](#cloze-cards-fill-in-the-blank)), and `line` (reveal one line at a
time). **Target depths** (personal, `[review] target`, default `recall`) form a
small ladder, `recognize` ⊂ `recall` ⊂ `reconstruct`: *recognize* is picking it
out (the ungraded acquire on-ramp below), *recall* is bringing the answer to mind,
*reconstruct* is producing it in full.

**The check is the combination:**

- At **recall**, a `flip`/`cloze` card **reveals** its answer and you self-grade;
  a `line` card reveals line by line (press `Space` to uncover the next line,
  recalling it first), then you self-grade.
- At **reconstruct**, you **produce** the answer: a `cloze` card has you **type**
  the gap; a card with a short, single-line answer has you **type** it exactly
  (`TAB` reveals two more characters as a hint, but a hinted card counts as
  missed); a card with a richer, multi-line answer becomes an **explain** prompt
  whose back lines are the **key points** you self-grade against.

Grading is always **missed it / partly / got it**, mapping to FSRS *Again* /
*Hard* / *Good* — a miss lapses the card (it comes back soon, interval shrinks),
*partly* is a weak pass (a shorter next interval), *got it* grows the interval. A
typed answer that's wrong or hinted counts as missed and the card returns later in
the same session.

**A default-target deck reviews as recall** — reveal-and-self-grade — even for
cards once written to be typed or explained (the retired `% mode:` directive). To
get the reconstruction checks (typing, explain), raise `[review] target` to
`reconstruct`.

**New cards are introduced as an *attempt*, not a hand-out.** A card you've never
seen isn't quizzed cold (you can't recall what you haven't read), but it isn't just
shown to you either. The first encounter is a low-stakes try, then the answer, then
one key (**Seen** / `Space`) records it as seen — *ungraded either
way*, with its first real quiz a **later session** (after a ~5-minute settle), so
nothing is tested the instant you've seen it. Two forms:

- **Recall** (default): the **front shows first** — try to bring it to mind — then
  you reveal the answer and press Seen.
- **Recognition**: if the deck was augmented with AI distractors (`alix deck augment
  <deck> --target choices`), an **atomic** (single-line) card greets you as a
  **multiple-choice** question instead — pick one, see which was right, press Seen.
  A correct guess doesn't promote it and a wrong one doesn't punish. This is the
  only place recognition appears in v1; it needs a full set of *AI* distractors, so
  a card without them (or with a multi-line answer) gets the recall attempt above.

How many new cards a session introduces is the `--new N` cap (default 10) — run
another session for the next batch.

**AI distractors** — plausible, tempting wrong answers tailored to each card —
are generated once with the model and cached by card id (in `augment.json` beside
your progress), so the recognition on-ramp stays instant and fully offline:

```sh
alix deck augment mydeck.txt --target choices --with "use common misconceptions"
```

Editing a card's answer regenerates its distractors next time you augment. See
[Augment a deck](#augment-a-deck--alix-deck-augment).

**The ladder climbs and descends per card.** Below your target, a card that has
graduated (reached FSRS's review phase) and then survives one *more* spaced pass
**climbs** to the next depth on a fresh schedule — so a card you've settled at
recall is later asked to reconstruct. A miss **descends** one rung and relearns
there (never below recall in this version). This is **v1**: it schedules *recall*
and *reconstruct* only, recognition stays the unscheduled acquire on-ramp, and a
reconstruct check on a rich answer is **self-graded** — there's no machine reading
of a full explanation.

**The rung badge.** In the web frontend a small badge above the answer shows the
card's current depth (`recognize` / `recall` / `reconstruct`); its **opacity
tracks FSRS retrievability** — bright when the memory is fresh, dimming as the card
comes due. The terminal shows the concrete check instead (`flip`, `typing exact`,
`line by line`, `explain`).

A `line`-reveal deck pairs with `% order: sequential` to walk its sections top to
bottom — e.g. one card per verse/chorus of a song.

The **explain** check (a reconstruct card with a multi-line answer) is the
day-to-day, self-graded tier below the [AI exam](#the-ai-exam-alix-exam): you
optionally type an explanation — never checked, just to make you commit before you
peek (the web shows it next to the points for honest comparison) — reveal the key
points, and grade whether you covered them. It pairs with the tutor. If you
[augment the deck with **key points**](#augment-a-deck--alix-deck-augment)
(`--target keypoints`), the reveal becomes a **checklist**: you tick each cached
point you covered and the grade is *derived* from the coverage — all → got it,
some → partly, none → missed it — a per-claim check instead of a gut judgment.
Atomic-answer cards aren't given key points, so they keep the plain reveal.

**Draw input (web).** A self-graded card marked `% input: draw` — a `flip`-reveal
card, or an explain check — is answered by drawing or handwriting on a canvas
(pen, touch, or mouse) instead of typing — for diagrams, circuits, math, or just
the retention of writing by hand — then self-graded against the card's normal
reveal (an answer image, key points, or text). Nothing is typed or sent to the server; the drawing is
discarded once you grade. For a card that *can* be typed, the web ☰ menu's
**Draw answers** toggle switches it to the canvas too, per device (remembered
in the browser). Drawn answers are self-assessed, not machine-checked — there
is no OCR or vision model reading the canvas.

To throw a card away, press the **remove** key (`Ctrl-X` by default) on it
instead of grading — it is dropped from the session without being asked again
(cloze siblings go too). The marked cards are deleted from their deck files,
and their progress is pruned, when the session ends. The same key works in
`alix browse`.

**Scheduling.** alix schedules with **FSRS** (the Free Spaced Repetition
Scheduler, FSRS-5, via the `rs-fsrs` crate) — one scheduler, no choice to make.
Each review feeds your grade (*missed it* / *partly* / *got it* → FSRS
*Again* / *Hard* / *Good*) into the card's memory model, and FSRS sets the next
interval from its estimated stability: successful reviews grow the gap, a lapse
shrinks it. Two knobs, both in the `[review]` config section (see
[Configuration](#configuration)):

- `retention` — the recall probability FSRS aims for (0.70–0.99, default 0.9).
  Higher means shorter intervals (you see cards more often).
- `retire_after` — once a card's interval reaches this (default `1y`), it
  **retires**: it rests and is no longer scheduled until you `alix reset` it. Set
  `never` to keep drilling forever. A workspace can override either knob in its
  own `alix.local.toml` (see [Workspaces](#workspaces)).

## Browse

`alix browse <deck>` is a walk through one deck's cards — front and back shown
together, in file order — without grading or scheduling. It is for a first
read-through of a new deck or just checking its contents, without affecting
your schedule. Navigate with `l`/`h` (next/previous, vim-style — `n`/`p`, the
arrow keys, and `Space` also work), `g`/`G` (first/last, also Home/End), and
`q` to quit. Pressing the remove key (`x` by default) marks the current card;
on quit the marked cards are deleted from their deck files and their progress
is pruned — the only thing browsing ever writes. The next/previous/remove/quit
keys are configurable in the `[keys.browse]` section of the config file (see below);
first/last stay `g`/`G`. Run `alix browse` with no deck argument to choose
decks from the same picker `alix` uses.

In the browser — `alix browse <deck> --serve`, or the **Browse** action in the
[web picker](#web-frontend) — it's the same read-through, as an in-page overlay
(Prev / Next / Leave) rather than a separate page. It's read-only there: card
removal is terminal-only.

## Web frontend

Add `--serve` to `review` or `browse` to run it in the browser instead of the
terminal — useful on a tablet or phone, where touch (and images) beats a TUI.
It runs the same session logic and writes to the same progress store, so
a card you grade in the browser shows up on the command line and vice
versa.

```
alix review rust.txt --serve              # open http://127.0.0.1:7777
alix review rust.txt --serve --port 8080
alix review rust.txt --serve --lan        # reachable from other devices on your network
alix browse rust.txt --serve              # read through a deck in the browser (in-page)
alix --serve                              # no decks -> pick them in the browser
```

Run `--serve` **without** naming any decks and the browser opens a
deck-selection screen that mirrors the terminal [picker](#getting-started): the
same three sections — **Workspaces** (each with its last-progress time) ·
**Recent** loose decks · **Folders** — and the same **single-launch**, so you
**click a deck to start it** (an exam-due deck sits its exam, and a
[trace](#traces-alix-trace) **walks** — predict → verify — at `/walk`, with a
**Back to decks** to return to the picker). Open a **Workspace** or **Folder** to
drill into its **unlock dependency tree**, where each deck nests under the
prerequisite that gates it. A 🔒 marks a deck whose **exam** is locked (a sourced
`% requires:` isn't passed) — still drillable; a deck dimmed with 🕒 has nothing
due. A `mastered 🎉` deck is tucked into the **Mastered window** (press `m`), and
mastered/done decks stay out of Recent (a quick launchpad) but are reachable by
filtering — the filter searches *every* loose deck. `browse` ignores locking, so any deck opens
there. Keyboard nav follows your `[keys.picker]` config (`j`/`k` or arrows move, `/`
or `Ctrl-F` filter, `m` the Mastered window). When you finish a session, "Choose
other decks" (on the summary) or `Esc` returns here — and a session
launched inside a workspace returns **into that workspace**. Naming decks on the
command line skips the screen and goes straight to review/browse.

Every check works in the browser: a **flip** or **cloze** reveal (reveal, then
self-grade Missed it / Partly / Got it), a **line** reveal (one line at a time —
it auto-scrolls to follow the newest line), a **typing** reconstruct (type your
answer and submit, each line marked ✓/✗ with the correct answer shown), an
**explain** reconstruct (reveal the key points and self-grade), and the
recognition **multiple-choice** on-ramp for a new card (tap one of the options).
The note appears once the answer is shown. Controls are big tap
targets and follow your configured key bindings — the page reads them from the
server, so the chips show your own keys. The **☰ menu** is context-aware: during
review it holds **Ask Tutor** and **Remove card** (which deletes the current card
from its deck file and prunes its progress) — plus **Promote to deck** while
reviewing a remediation (virtual) card, which appends it to the deck file; on the
deck picker, **keyboard shortcuts**, **refresh decks**, and **about** — with
**Theme…** in both.

A **gallery of themes** ships with the web UI — the alix **Dark**/**Light**
originals and a playful **Kid** theme, plus crowd-favourite editor/slide palettes
(GitHub, Dracula, Nord, Solarized, Gruvbox, Catppuccin, Tokyo Night, Monokai, One
Dark, Ayu, Rosé Pine, Everforest). Open the **Theme…** popover from the ☰ menu — a
grid grouped Light / Dark that previews on a sample card as you hover and re-themes
the whole app when you click one, remembering your choice in the browser; no
configuration needed.

It is deliberately local-only — no accounts, no database. By default it binds
to `127.0.0.1` (this machine only); `--lan` binds all interfaces so a device on
the same network can reach it at `http://<your-machine-ip>:<port>`. Serving with
`--lan` auto-generates a **pairing token** (printed at startup) and requires it
on `/api/*`, so the endpoint isn't wide open — pin your own with `--token` or
`[serve] token`. `--port`, `--lan`, and `--token` require `--serve`; the default
port lives in the `[serve]` section of the config file and `--port` overrides it.

## The Tutor

On any post-answer screen (feedback, revealed flip card, answered choice),
press `?` to open a tutor panel without leaving the session. The card
(front, answer, note, deck name) is sent as context to the configured model
CLI, so you can ask "why is that the answer?" and follow up. The tutor
remembers earlier cards and questions across the whole review run — Claude
does this via its native session flags (`--session-id` / `--resume`); other
backends re-inline the Q&A transcript into each follow-up so the context
carries over (the prompt grows with the conversation rather than being
resumed efficiently, but memory is preserved). The panel shows only the
**current card's** exchanges. By default the tutor answers from the card
text plus its own knowledge (tools: `WebFetch`/`WebSearch` where available),
and uses the **CLI's default model** — set `[ask] model`/`effort` to pin a
stronger one (the web panel shows which model is answering). For a deck built
from source, set **`[ask] source_access = true`** to let the tutor **read the
card's source** to verify its answer: it runs `Read`/`Glob`/`Grep` with its
working directory at the deck's `% source:` project root (the nearest
`Cargo.toml`/`.git`/… above the cited files) and is told to check the real
files before answering. It's off by default because it grants file-read access
— only enable it on a machine and network you trust (especially with `--serve
--lan`). A [workspace](#workspaces) can override it per-folder: put
`source_access = true` (or `false`) in its `alix.toml` to decide for that
crate alone, beating the global default. While the model CLI runs, the session
stays responsive; Esc returns exactly where you were.

This works in the **web frontend** too (`--serve`): an "Ask" button (and the
`?` key) on an answered card opens a chat panel — type a question, **Send**,
**Save note**, **Close**. The server invokes the model CLI on a background
thread and the page polls for the reply, so the single-threaded server never
blocks. Ask is reachable wherever you serve, including `--lan` (the request
runs the CLI on the host, so — like `--lan` generally — only use it on a
network you trust).

While typing a question you can edit it like a normal input line: `←`/`→`
move the caret, `Home`/`End` (or `Ctrl-A`/`Ctrl-E`) jump to the ends, and
`Backspace`/`Delete` remove the character before/under it.

Decks can carry reference links as comment lines:

```
% link: https://docs.rs/async-compat
% link: https://tokio.rs/tokio/tutorial
```

They are handed to the tutor with the first question as background material to
consult when useful — fetched once, remembered for the rest of the run. These
lines do not affect card hashes.

Because the CLI runs headless, it cannot show interactive permission prompts —
an unanswerable prompt would hang the call. `alix` therefore runs it with a
locked permission mode and an exclusive tool allowlist (`WebFetch`,
`WebSearch` by default): the listed tools work without prompting, and every
other tool is silently denied, so a malicious page behind a deck link cannot
make the tutor run shell commands. Both the permission mode and the allowlist
are configurable in `[ask]`. (Codex uses a sandbox rather than a tool
allowlist — see [Backends](#backends).)

`Ctrl-N` condenses the conversation into at most three short note lines and
appends them to the card in the deck file (notes are not hashed, so the
card's progress is untouched). Requires the configured model CLI to be
installed and logged in; the command, a `--model` override and the timeout are
configurable in the `[ask]` section of the config file.

## Generate a facts deck — `alix deck generate`

`alix deck generate <source>` turns a **source** into a deck of fact cards using
the configured model CLI. The source is a web page URL *or* a local file/directory
path (the deck-side mirror of `alix trace`):

```sh
alix deck generate https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
alix deck generate src/scheduler.rs        # a local file (or a directory)
alix deck generate <source> -o ownership   # choose the file name
alix deck generate <source> --cards 15     # cap the number of cards
alix deck generate <source> --review       # 2nd pass to remove redundant cards
alix deck generate <source> --print        # print to stdout instead of writing
```

For a **web page**, the model CLI reads it with a web-fetch tool and the deck
starts with a `% link:` line back to it (so you can use the *ask* feature on the
cards). For a **local source**, the CLI explores it read-only with file-read tools
at the source root and the deck starts with a `% source:` line (so `alix exam`
can later grade your understanding against it); it also adds a
[`% at:` citation](#source-citations--at-on-a-fact-card) to each fact that maps
to specific lines, so the card can show its source on reveal. The CLI returns
text only — never a write or shell tool — and `alix` validates it (`parse_str`)
and writes `~/decks/<slug>.txt`. The cards are spread across four layers of
understanding (facts → concepts → application → connections) and use
`% reveal: cloze` cards for terminology.

The model drafts, then re-reads the whole set and merges or drops cards that
test the same fact, so the deck doesn't repeat itself. For a stronger pass,
`--review` (or `generate.review = true`) runs a **second** call that takes the
draft and returns a deduplicated, tightened version — an extra call, worth it
when the source is repetitive.

The prompt and limits live in the `[generate]` section of the config:
`model`, `timeout_secs` (default 300), `max_cards` (default 30), `extra`
(guidance appended to the built-in prompt), `prompt` (a full override, with
`{url}` and `{max_cards}` placeholders), and `review`. It reuses the `[ask]`
command and permission settings. Review the result before relying on it — it is a starting
point, not a final deck. Generation needs a model CLI installed and logged in,
and works best on a single page or chapter (a whole book overruns the context
budget). Note: backends without web access (Codex) can't generate from a URL —
point the source at a local file or switch `[ask] backend`.

## Augment a deck — `alix deck augment`

`alix deck augment <deck> --target <kind>` enriches an *existing* deck using the
configured model CLI. It's a **deliberate, one-off command** — generation happens
here, in the foreground (so any error surfaces immediately), and the result is
cached beside your progress (`augment.json`, keyed by card id). Review then reads
the cache, so study stays instant and fully offline — the model CLI is never
called mid-session.

```sh
alix deck augment mydeck.txt --target choices    # multiple-choice distractors
alix deck augment mydeck.txt --target notes      # a trivia / mnemonic note per card
alix deck augment mydeck.txt --target questions  # reworded phrasings of each question
alix deck augment mydeck.txt --target keypoints  # decompose answers into a checklist (explain mode)
alix deck augment mydeck.txt --target format     # reshape badly-shaped cards for cleaner display
alix deck augment mydeck.txt --target topology   # a graph + walk + regions (experimental)
alix deck augment mydeck.txt --target choices --with "use common misconceptions"
```

- **`--target choices`** writes plausible wrong answers for [choice
  mode](#review). Review uses them automatically; cards without them fall back to
  the offline sampler.
- **`--target notes`** writes one short note (trivia, context, a mnemonic) per
  card, shown *alongside* the card's own `! ` deck note on reveal. Your deck file
  is never modified — AI notes live only in the cache.
- **`--target questions`** writes a small pool of **reworded phrasings** of each
  question (the same answer still applies). Review rotates a fresh one in each
  time the card comes up, so you can't pass it by recognizing one fixed wording —
  you have to actually read and understand it. Plain (non-cloze) cards only, since a
  cloze card's "front" is its title.

  It only helps when the question carries *content* to reword. A substantive
  front morphs well:

  ```text
  What does the CAP theorem state?
    → What claim does the CAP theorem make?
    → According to the CAP theorem, what is asserted?
    → What is the central assertion of the CAP theorem?
  ```

  A content-free front can only become other content-free fronts — morphing adds
  nothing:

  ```text
  What is it?
    → What is this?   What's it called?   Can you name it?   Which one is this?
  ```

  So write **self-contained questions** and morphing earns its keep; vague fronts
  like "What is it?" are a smell either way.
- **`--target keypoints`** decomposes each card's answer into the few
  load-bearing claims a from-memory reconstruction must hit. In
  [**explain** mode](#review) the reveal becomes a **checklist** of those points:
  you tick the ones you covered and the grade is *derived* — all → got it, some →
  partly, none → missed it — so the self-grade is a per-claim check, not a vibe.
  An *atomic* answer (a single fact/term/date with nothing to decompose) is left
  alone, keeping its plain reveal. Tune the maximum with `[ai] keypoint_count`.
- **`--target format`** reshapes badly-shaped cards — most often a list crammed
  into a single prose answer — into clean display lines, a tidier front, an
  optional note, and a **suggested reveal-method** (`line` or `flip`).
  It is purely cosmetic: it never edits the deck file, never changes card
  identity, and your progress is untouched. The reshaped text and reveal suggestion
  are cached in `augment.json`; both review and browse apply them at display
  time, so the two views show the same card. Plain (non-
  cloze) cards only — cloze cards are left alone. An explicit `% reveal:` you
  wrote always wins over the suggestion. Because it's an AI heuristic it can miss
  or mis-shape a card; the result is easy to discard via **Remove** in the Augment
  screen or by running `--target` removal. Stacks well under `notes` (trivia).
- **`--target topology`** *(experimental)* derives a **graph of how the deck's
  cards relate** — labeled edges, a suggested **walk**, and a handful of coarse
  named **regions** — cached like the rest. A deck can hold several topologies,
  one per `--with` principle and keyed by it (`auto` when none). `alix review
  <deck> --topology <name>` then serves the **due** cards in that walk's order
  instead of at random — SRS still decides *which* cards are due, the topology
  only reorders them — and review shows a thin **region breadcrumb** ("where am
  I", current emphasized) so the sequence reads as a path, not a shuffle. With a
  single cached topology, `--topology` (no name) picks it automatically. The
  breadcrumb doubles as a **strength heatmap** (a per-card bar under each region,
  red → green) that greens up as you learn the region, and `--region <name>`
  **drills one region** alone. In the **web picker**, selecting a deck that has a
  topology opens an inline **focus drawer** — pick the topology and a region
  (click or ← / →, its heatmap and **due count** shown) to scope the launch — so
  you set all this before the session starts, never mid-card.
- **`--with "<guidance>"`** steers *how* (e.g. "use common misconceptions",
  "add a surprising historical fact", "phrase questions as real-world scenarios",
  or a topology principle like "by type dependency" / "north to south").

Nothing here touches a card's identity, so augmenting never resets progress
(distractors, notes, and variants all key off the answer, which the id hashes —
not the front); editing a card's answer changes its id, so it simply regenerates
next time you augment. Tuned under `[ai]` (`model`, `distractor_count`,
`variant_count`, `timeout_secs`). Augmentation needs a model CLI installed
and logged in.

**From the web picker.** You don't need the command line for any of this: focus a
deck and press **`a`** (or its **Augment** button) to open a screen of each
target's coverage, with **Generate** to fill the cards a target is still missing
and **Remove** to clear one (or all). There generation runs in the **background**
— the page polls while the model works — but it writes the same `augment.json`, so
review reads it identically. Shown on decks, not workspaces.

## Import an Anki deck (`alix import`)

`alix import <file.tsv>` turns an Anki export into an `alix` deck — no model
CLI needed. Export your notes from Anki as **Notes in Plain Text** (`.txt`/`.tsv`)
with fields separated by a tab; the first field becomes the front, the second
the back, and any further fields are ignored:

```sh
alix import french.tsv                 # writes ~/decks/french.txt
alix import french.tsv -o vocab        # choose the deck name
alix import french.tsv --print         # print to stdout instead of writing
alix import french.tsv --force          # overwrite an existing deck
```

It skips Anki's `#`-prefixed header lines (`#separator:tab`, `#html:true`, …),
turns `<br>` tags into separate answer lines, decodes the common HTML entities
(`&amp;`, `&lt;`, `&nbsp;`, …), and backslash-escapes a back line that would
otherwise read as an `alix` comment or note. Rows missing a side are dropped. The
result is validated (`parse_str`) and written to `~/decks/<name>.txt`; review it
and clean up any leftover HTML by hand. It works best on a plain two-field
export — rich notetypes, media, and tags don't carry over.

## The AI exam (`alix exam`)

Mechanical review *loads* a deck's material into memory; the **AI exam**
*checks whether you understood it* and is what gates progression. The idea: drilling
cards proves recall, but not that the ideas connected — so a deck can declare a
ground-truth **source** and require you to pass an exam against it before it
counts as done.

Declare one or more sources in the deck header (a URL or a local file path,
repeatable):

```
% source: https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
% source: notes/ownership.md
```

A URL `% source:` doubles as a [tutor](#the-tutor) reference,
so you don't need to repeat it as a `% link:`. The reverse isn't true: a
`% link:` stays a tutor reference and never becomes exam ground truth — keep
supplementary links (a blog, an SO answer) as `% link:` so the exam ignores them.

Once every card in a `% source:` deck has **graduated** — reached FSRS's review
phase, past the initial learning steps — the deck is **exam due** rather than
finished (so it does not yet unlock its dependents).

**Sitting the exam — interactive, in either frontend.** The exam is a guided,
one-question-at-a-time flow (Back/Next, then a per-question breakdown), the same
in the terminal and the browser:

- **Terminal**: `alix exam ownership.txt` (or `--questions 8`, `--strictness …`)
  opens the exam in the TUI. You also reach it by **picking an `exam due` deck**
  in the launcher (it starts the exam instead of an empty review), or from the
  **session-end summary** — when you drill a deck's last cards and it turns exam
  due, the summary offers "press `x` to take it" (or `b` to browse the deck).
- **Web** (`alix serve`): picking an `exam due` deck in the deck list launches
  the exam in the page; a finished session likewise offers it at the summary.

`alix` asks the configured model CLI to read the source (URLs via a web-fetch
tool; local files are embedded) and write fresh **open understanding** questions —
application and connections, not the card facts — each with the key points a
correct answer must contain. You type a prose answer per question, and an
examiner grades them Pass / Partial / Fail **against the source's rubric, never
against your cards** (grading the cards would be circular), at the deck's
configured strictness (below). The model CLI calls run on a background thread
so the UI stays responsive.

- **Pass** (every question by default; tune with `pass_threshold`) marks the
  deck **mastered** (shown as `mastered ✓`). Mastery — not mere drilling — is
  what unlocks decks that `% requires:` it. Source-less decks are unchanged:
  finishing = drilled (`done ✓`).
- **Fail** lists the gaps and offers to turn them into remediation cards — the
  card type is picked per gap (a `% reveal: cloze` or plain card for a missed
  fact, an open understanding card — a prompt plus key points — for a missed
  concept), with overlapping gaps merged into one card. Re-drill those and re-sit.
  Once created, the screen reports how many remediation cards it added.

**Remediation cards are virtual.** They live in alix's store, not your deck file,
so the deck `.txt` is left unchanged. While drilling one, the review screen's
mode badge reads "remediation card" in place of the "new card" badge. A virtual
card drills like any other — its first pass comes about a minute later, then
FSRS schedules it — and it counts toward a deck's *due* total but not toward its
card count. Regenerating the same
gap won't duplicate it; once its interval reaches the retirement cap
(`retire_after`) it's archived, and re-failing the gap revives it. If a
remediation card has earned a permanent place, **promote** it during review
(`Ctrl-P` in the terminal, or "Promote to deck" in the browser's review menu):
alix appends it to the deck file, drops the virtual copy, and carries over the
card's review progress — it doesn't restart.

Resetting a whole deck's progress (`alix reset <deck>`) also clears its mastered
state, so a re-drilled deck must pass the exam again (resetting only individual
cards with `--card`/`--cards` leaves the mastered state intact).

**Grading strictness** is a property of the *material*, so it's per deck. A
checklist-style topic (a procedure, exact syntax, a security drill) should fail
you for omitting a step; a conceptual topic shouldn't. Set it with a
`% strictness:` header directive (or `alix exam --strictness …`, or the
`[exam]` default):

- `strict` — completeness required: every rubric point must be present, so
  omitting one is a gap.
- `balanced` (default) — judges *understanding*, not phrasing: a point counts as
  covered if your answer shows you grasp it (even briefly), and only a wrong or
  genuinely-absent idea is a gap.
- `lenient` — benefit of the doubt: only clearly wrong or unanswered points are
  gaps.

This dial (how hard each answer is judged) is independent of `pass_threshold`
(how many answers must pass).

Settings live in the `[exam]` section: `model`, `timeout_secs` (default 300),
`num_questions` (default 5), `pass_threshold` (default 1.0 = all must pass),
`strictness` (default `balanced`), `retry_cooldown_secs` (default 3600 — how long
a failed *trace* exam waits before a re-sit; `0` disables it), and `extra`
(guidance appended to question generation). It reuses the `[ask]` command,
permission mode and tool allowlist, and needs a model CLI installed and
logged in. Note: a URL `% source:` requires a backend that can fetch web
content (Codex cannot — use a local file path or switch `[ask] backend`).

## Traces (`alix trace`)

> **Experimental.** Traces are a new, evolving feature — the deck format and the
> flow may still change.

Cards drill *facts* (the nodes of what you know); a **trace** drills the
*connections between them* (the edges) by walking a **path** through a real
source and making you **predict each hop before it's revealed**. Where the
[AI exam](#the-ai-exam-alix-exam) verifies a *set* of independent answers, a
trace verifies that you can follow one chain of reasoning — and the gap between
your prediction and the truth is where the understanding forms.

A trace is just a deck with a `% trace:` (a path description — what it walks,
which also marks it a trace) and a `% source:` (the path's origin), then a
sequence of **checkpoint** cards. Each checkpoint is an `explain`-style card — an
open *predict* prompt, the key points a good prediction should hit — plus a
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

The locator is a single contiguous range `file:start-end` (e.g.
`src/ch04-01-what-is-ownership.md:290-297`), or — when `% source:` is a single file — just the line
numbers; it is never comma-separated (a stitched excerpt makes disjoint code look
adjacent). The lines are **read live from the source** each time you walk, so the excerpt is
always current and the deck stays small; the source is the oracle, not an
invented answer. When a tight excerpt uses a symbol defined off-screen, name it
with a `% given:` line (`% given: state — the parser's position so far`,
repeatable) — these show as a list under the question, so the excerpt stays
focused without orphaning the names it leans on.

**Building a trace.** Instead of hand-writing the checkpoints, declare just the
`% trace:` and `% source:`, then run `alix trace --build <deck>`: the model CLI
explores the source — with **read-only** `Read`/`Glob`/`Grep` and the source
root as its working directory (no write or shell access) — traces the single
load-bearing path, and writes the checkpoints (with their `% at:` locators)
back into the deck file. The result is cached and version-controlled
there, so review it — especially the locators — and edit freely; re-run
`--build` to regenerate. The build prompt encodes the chain rules below, so a
generated trace comes out a path, not a quiz.

**Snapshotting the source.** Because `% at: file:lines` reads the **live** source,
editing a traced file shifts every excerpt to the wrong lines. So when you create
a workspace by exploring a source ([`alix explore --into --build`](#exploring-a-source--alix-explore)),
its final step **freezes the cited excerpts** into the workspace's `assets/`
folder — one tiny snippet per checkpoint — and repoints each `% at:` (and the
trace's `% source:`) at them. The excerpts never drift and the workspace is
self-contained, **without copying whole source files**. A re-based snippet loses
its original line numbers, so when those matter the original location is kept on
the card's `% at:` line, after ` from ` (`% at: 12.rs from scheduler.rs:90-98`),
and the freeze records the live source root in an `% origin:` directive so the
tutor stays grounded and `alix deck check` can flag drift. It's automatic for explored
workspaces, not a command; a loose trace over a live `% source:` is left as-is.

**Checking the locators.** For a trace that *isn't* frozen — a loose `.txt` over
a live `% source:` — **`alix deck check`** validates that every `% at:` still
resolves into its source: it warns about a locator that names a missing file,
runs past the end of the file, or (for a single-file source) gives bare line
numbers it can't place. It's a quick structural check — *does this excerpt still
exist?* — so a moved or trimmed source is caught before you walk into it, not
mid-hop. (Frozen snapshots don't move, but their snippets are validated the same
way.)

Because building is one-shot, correctness-critical, and **fails silently** when
the model is weak (you still get parseable checkpoints, just a loose chain you
then drill), the `[trace]` config section defaults it to a strong model
(`model = "opus"` for Claude; other backends inherit their CLI's default) and
high reasoning effort (`effort = "high"`) — slower than the other AI features,
but it runs once and is amortized over many reviews. Override `model`, `effort`
(`low`–`max`) and `timeout_secs` there. `--suggest`
shares these `[trace]` settings (it's also one-shot recon), but **`--grade` does
not**: judging a prediction is a light, interactive, per-hop call, so it runs at
the tutor tier — the `[ask]` model, effort and timeout — instead.

**Don't know what to trace?** `alix trace --suggest <source>` does a single
read-only recon pass over a source (a repo `.`, a directory, a file, or a URL)
and prints a **ranked menu of candidate traces** — each a path-question, a
one-line spine sketch, and a suggested `% source:` scope. The list is sized by
**coverage** — the central spine plus one main path per major subsystem — so it's
as long as the source needs, not a fixed number, and it's the central *starting
points*, not an exhaustive curriculum. It also names the *node-shaped* subsystems
it skips (a config table, a store's on-disk format) as **facts-deck material**
rather than forcing them into a fake path — facts are a deck's job, edges are a
trace's. It writes nothing: pick one, paste its
header into a new deck, and `--build` it. Knowing *what* is worth
tracing — and how deep — is itself the hard part (it needs you to already
understand the source), so this hands that judgment to the model.

**Write it as a chain, not a quiz.** A trace's whole value is that it's a *path*:
each checkpoint should pick up where the last *reveal* left off (note how hop 2
above opens with hop 1's conclusion, "s1 and s2 point at one heap allocation"), so you're
following one thread — a data flow, a control flow, a derivation — to the
outcome. If the checkpoints are independent facts that all hang off one thing, you've
written a *set*, which is what cards and the exam already do; reach for a subject
that has a real sequence.

**Walking it** (`alix trace docs/examples/rust-ownership/ownership-move.txt`) goes hop by hop:

1. **Predict** — you type a guess before anything reveals (committing is the
   point).
2. **Reveal** — `alix` prints the real excerpt from the source, then the
   checkpoint's key points and note.
3. **Gap** — you judge yourself **Missed it / Partly / Got it** (the same three
   grades review uses). Grading is self-judged and offline (no model call) by
   default; pass **`--grade`** to have the model judge your typed prediction against
   the key points and return the verdict plus a line of feedback (a model call per
   hop). Either way, a failed or partly hop is a **weak edge** that resurfaces
   sooner — a failed one resets, a partly steps back one stage — while a passed
   hop advances and fades. Each checkpoint is an ordinary card under the hood, so
   this scheduling is the normal per-card SRS.
4. **Compress** — after the last hop you restate the whole path in two
   sentences: if you can re-derive it, you understood it.

**In the browser** — run **`alix serve`** and pick the trace in the deck list; it
walks in the web frontend. The walk page shows the **path** as a rail you descend
(its nodes color in by Missed it / Partly / Got it) and reveals each checkpoint's
real source in a line-numbered excerpt. Progress saves to the same store, so a
walk started in the terminal continues in the browser.

`alix trace <deck> --map` prints the path (every prompt, its key points and
locator) without quizzing — a quick "just show me the route".

**A trace's exam is the compression.** Walking the checkpoints is the *drill*;
the *verification* is `alix exam <trace>` — one fixed question, the `% trace:`,
which you answer by retracing the whole path in a sentence or two. The model grades
that against the path's checkpoints (AI-graded, like a fact deck's exam), and
**passing masters the trace** (unlocking its dependents). You reach it three
ways: directly with `alix exam <trace>`, as the **capstone** offered at the end
of a walk, or via the picker's **"Take exam"** — and, like a fact deck, you can
sit it early to *test out* without walking. A failed trace exam is **re-walked**,
not remediated into cards (a trace is a path, not a card pile); after a fail it
**cools down** before you can re-sit (so the graded feedback can't just be pasted
back — `[exam] retry_cooldown_secs`, default 1h).

A trace deck degrades gracefully — even without `alix trace` it is a valid deck
of `explain` cards. See `docs/examples/rust-ownership/ownership-move.txt` for a
complete trace — a frozen snapshot over The Rust Book's ownership chapter.

## Exploring a source — `alix explore`

`--suggest` lists central *traces*; **`alix explore <source>`** goes one layer
up and prints an ordered **learning plan** toward a goal — the facts decks **and**
traces worth authoring, dependency-ordered:

```
alix explore .                                      # plan to understand the whole source
alix explore . --goal "how review scheduling works" # a narrow goal → a focused subset
```

Each item is tagged `[trace]` or `[deck]` — chosen by the shape of the knowledge
(a *path* you predict hop by hop becomes a trace; a *table of facts*, like a
config's knobs or a store's on-disk format, becomes a facts deck) — carries its
`% requires:` prerequisites (the list is dependency-ordered, foundations first),
and a `% source:` scope. The `--goal` scopes coverage: a broad goal covers every
subsystem, a narrow one collapses to just its slice (and traces it in more
detail). By default it's **read-only** — it prints the plan and you author the
items yourself (`alix trace --build` a trace, `alix deck generate` a facts deck).

With **`--into <dir>`** it materializes the plan into a ready-made **workspace**:

```
alix explore . --goal "how review scheduling works" \
  --into ~/decks/scheduling/ --title "Scheduling internals"
```

That writes an `alix.toml` and one stub file per item — a `% trace:` deck for
each trace (run `alix trace --build` on it) and a `% title:` facts deck for each
deck (author it or `alix deck generate`) — wired together with `% requires:` so they
unlock in dependency order, with each `% source:` pointing back at the real
source. The `--goal` becomes the workspace's `description`; **`--title`** names
it (omitted, the folder name is used). (Refuses a non-empty folder unless
`--force`.)

Add **`--build`** to go all the way: `alix explore … --into <dir> --build`
explores the source **once** and then reuses that same session to fill every
item — predict-verify checkpoints for the traces and fact cards for the decks —
so the workspace comes out review-ready in one command. Writing the whole set
from one understanding keeps the items coherent (each builds on its prerequisites
instead of repeating them), and it fills facts decks too. As a final step it
**freezes the cited excerpts** of every cited deck — traces *and* fact decks with
[`% at:` citations](#source-citations--at-on-a-fact-card) — into the workspace's
`assets/` (see [Snapshotting the source](#traces-alix-trace)), so the workspace
is self-contained and its locators never drift. (A snapshotted fact deck's
`% source:` then points at `assets/`, so its exam grades against the frozen
excerpts.)

**Explore walk.** Before you even know what to trace, `alix explore --walk
<source>` builds a short **tour of the source's shape** and walks it like a trace:
you predict what kind of program it is (from the manifest), its domain nouns (from
the module list), how it's driven (the entry point), its spine (the central file),
and finally the first paths worth tracing — each hop revealing the real lines.
It's written to a file (`-o`, default `explore.txt`), so `alix trace explore.txt`
re-walks it.

## Configuration

Key bindings can be changed in `~/.config/alix/config.toml`. Create the
file with `alix config --init`, inspect the active bindings with
`alix config`. Every action takes a list of keys; the first one is
shown in the footer. For example, to grade self-graded cards with j/k/l:

All keybindings live under `[keys]`, one subtable per surface — `[keys.review]`
(the review screen), `[keys.picker]` (the deck picker), and `[keys.browse]`
(`alix browse`):

```toml
[keys.review]
failed = ["j"]
partly = ["k"]
passed = ["l"]
```

Keys are written as a single character (`"j"`), a special key name
(`"space"`, `"enter"`, `"tab"`, `"esc"`, `"backspace"`), or either with a
`ctrl-` prefix (`"ctrl-s"`). Rebindable `[keys.review]` actions: `failed`,
`partly`, `passed`, `reveal`, `hint`, `submit`, `skip`, `remove` (mark the card
for deletion from its deck file, default `ctrl-x`), `promote` (append a
remediation card to its deck and drop the virtual copy — only on virtual cards,
default `ctrl-p`), `continue`, `restart` (start
a new session from the summary screen, default `r`), `quit`. While you are typing
an answer (a reconstruct check), plain character bindings are ignored so
they cannot shadow text input — use `ctrl-`/special keys for `hint`, `skip`
and `quit`. A different config file can be passed with `--config <path>`.

The picker's Vim-style navigation is under `[keys.picker]` (`up`, `down`, `open`,
`back`, `filter`, `mastered`), and the read-only browser (`alix browse`) has its
own bindings under `[keys.browse]`:

```toml
[keys.browse]
next = ["l", "n", "space"]    # default vim-style l, plus n and space
prev = ["h", "p"]
remove = ["x"]                # mark the current card for removal
quit = ["q", "esc", "ctrl-c"]
```

Letter bindings are case-insensitive, so jump-to-first/last stays fixed at
`g`/`G` (and Home/End); the arrow keys always move next/previous too.

The web frontend (`--serve`) reads its default port from a `[serve]`
section; `--port` overrides it:

```toml
[serve]
port = 7777
```

### Review pacing

A `[review]` section tunes the FSRS scheduler and the ladder depth you drill
toward:

```toml
[review]
retention = 0.9         # FSRS target recall probability (0.70–0.99); higher = shorter intervals
retire_after = "1y"     # a card rests once its interval reaches this ("2w", "6m", "30d", or "never")
target = "recall"       # depth ladder target: recognize | recall | reconstruct
```

`retention` is the recall probability FSRS schedules for — raise it to see cards
more often, lower it to stretch intervals. `retire_after` is when a card
**retires** (rests until `alix reset`); `"never"` keeps it in rotation forever.

`target` is how deeply you want to end up retrieving each card (see
[Review](#review)). It's **personal**, not a deck directive — depth is the
learner's call, not the author's — so it lives here, never in a shared deck. At
`recall` (the default) a card reveals and you self-grade; at `reconstruct` a card
that has settled climbs to producing its answer in full (typing a short answer or
a cloze gap, or explaining a longer one). `recognize` clamps up to `recall` in v1
— recognition is the unscheduled acquire on-ramp, not a scheduling target.

A workspace can override all three for its own decks in an `alix.local.toml` (see
[Workspaces](#workspaces)) — a personal file that is never shared.

### Backends

By default, `alix` routes all AI calls through the [Claude Code](https://www.anthropic.com/claude-code)
CLI (`claude -p`). You can switch to one of the other supported CLIs by setting
`backend` in the `[ask]` section:

```toml
[ask]
backend = "claude"   # default — Claude Code CLI
# backend = "gemini"  # Google Gemini CLI
# backend = "codex"   # OpenAI Codex CLI
# backend = "copilot" # GitHub Copilot CLI
```

**Auth is each CLI's own login — alix stores no API keys.** Install the CLI you
want to use, run its login command once (e.g. `claude`, `gemini login`,
`codex login`, or `gh auth login`), and alix picks it up.

Each backend is granted **read-only tools only** — file reading and (where the
CLI supports it) web fetch. No write or shell tool is granted to any backend.
Backends degrade gracefully when they can't fulfil a request: a backend that
can't reach the web (Codex runs under a network-blocking sandbox) will refuse
a URL-based exam or deck generation with a clear message naming the fix — point
the source at a local file, or switch `[ask] backend`. Rate-limit and
authentication errors surface as actionable guidance rather than raw CLI output.

**`alix backend check [--all]`** sends a short tool-free request to the
configured backend (or all four with `--all`) and reports whether each is
installed, signed in, and responding. Useful for confirming the whole path
works before running a longer command.

**Pre-flight size guard.** Before an agentic command (`alix deck generate`,
`alix exam`, `alix trace --build`, `alix explore`) reads a large source, alix
measures its size and prompts for confirmation. Pass `--yes` to skip the
prompt in non-interactive scripts.

**Multi-turn tutoring.** Claude's native session flags (`--session-id` /
`--resume`) keep a running conversation across cards. Other CLIs don't have
those, so alix re-inlines the accumulated Q&A transcript into each follow-up
prompt instead — the tutor remembers earlier questions on every backend, though
the prompt grows with the conversation rather than being resumed efficiently.

## Card identity and storage

- Cards are identified by an XxHash64 over the deck **file name** plus the
  back lines. Your progress survives editing a card's front and adding notes,
  but renaming a deck file or editing its back lines resets the affected
  cards.
- Progress is stored at `~/.local/share/alix/progress.json` and config at
  `~/.config/alix/config.toml` (created on first use).
- `alix reset <deck>...` clears stored progress so cards become "new" again —
  for whole decks, a single card (`--card <id-or-front-text>`), or the entire
  store (`--all`). Run it with no decks to pick from the deck list, or add
  `--cards` to pick individual cards from a checkbox list. It confirms first
  unless you pass `-y`/`--yes`.

## Desktop integration

To launch `alix` from the desktop menu (Cinnamon, GNOME, KDE, ...), run:

```sh
assets/install-desktop.sh
```

It installs the icon (`assets/alix.svg`, rendered to the standard
PNG sizes), a launcher that reviews everything due in `~/decks` in a
terminal, and a `.desktop` entry under `~/.local/share`. Re-run it after
editing the SVG. The launcher prefers an installed `alix` (`cargo install
--path .`) and falls back to the project build.

## Development

```sh
make check       # clippy + tests — the gate before any change is done
make fmt         # format (nightly rustfmt; not plain `cargo fmt`)
make serve       # run the web frontend
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for how contributions are gated — the
focus (fit) gate, the house rules (craft gate), and the PR checklist.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
