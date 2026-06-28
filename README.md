# `alix`

[![crates.io](https://img.shields.io/crates/v/alix)](https://crates.io/crates/alix)
[![docs.rs](https://img.shields.io/docsrs/alix)](https://docs.rs/alix)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/alix)](https://crates.io/crates/alix)

**DISCLAIMER**
This is WIP - don't use it for serious learning just yet. There will be breaking
changes to the deck format, to the progress store, and what not. Most likely you'll
lose all your progress and I won't provide a migration path. You have been warned!

Your **personal AI tutor**, built for understanding — not just remembering.
Under the hood it's a plain-text spaced-repetition trainer — Leitner and SM-2
scheduling, several answer modes, cloze and dual-direction cards, images, and
deck dependencies — and the layer on top is the **Claude integration**: an
*ask-Claude tutor* on any card, *AI deck generation* from a web page,
*understanding cards*, and an **AI exam** (`alix exam`) that checks whether you
grasped the material — not just recalled it — and gates your progress on
passing. Decks stay simple plain-text files you own, reviewable in a ratatui
terminal UI or a local web app.

## Requirements

The flashcard **core** — review, scheduling, every answer mode, browse, the TUI
and the web frontend — runs standalone, with no external services or accounts.

The **AI features** shell out to the [Claude Code](https://www.anthropic.com/claude-code)
CLI, so they need it **installed and logged in** (which in turn needs a Claude
subscription or API access). Install the CLI and run `claude` once to
authenticate. The features that require it:

- `alix deck generate` — generate a facts deck from a URL or a local
  file/directory; `alix deck augment` — add AI distractors or notes to one;
- `alix exam` — the AI exam;
- `alix trace --build` / `--suggest` / `--grade` — discover, suggest, and grade
  traces;
- `alix explore` — goal-driven learning plans;
- ask-Claude (`?`) — the in-session tutor, in both the TUI and the web frontend.

`alix` invokes the CLI headless (`claude -p`) under a locked-down permission
model (see [Ask Claude](#ask-claude-about-a-card)); the command, model and
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
[Workspaces](#workspaces) for the full details. (The AI steps need the Claude
CLI — see [Requirements](#requirements).)

## Usage

The binary is called `alix`:

```sh
alix                            # pick decks interactively (recent + ~/decks)
alix mydeck.txt                 # review due cards (flip mode, Leitner)
alix --mode typing mydeck.txt   # type the answer character by character
alix --mode fuzzy mydeck.txt    # whole-line input, small typos tolerated
alix --mode choice mydeck.txt   # multiple choice (distractors sampled from the deck)
alix --mode line mydeck.txt     # reveal the answer one line at a time (lyrics)
alix --scheduler sm2 mydeck.txt # SM-2 intervals instead of Leitner
alix --cram mydeck.txt          # ignore cooldowns, review everything
alix browse mydeck.txt          # read through cards, no grading or scheduling
alix deck generate <url-or-path>   # generate a facts deck from a web page or a file/dir
alix deck augment mydeck.txt --target choices   # AI distractors (cached; review reads them)
alix deck augment mydeck.txt --target notes --with "add trivia"   # AI notes
alix import cards.tsv            # import an Anki TSV (front<TAB>back) into a deck
alix exam mydeck.txt            # AI exam against the deck's % source: (gates unlocks)
alix trace mytrace.txt          # walk a predict-and-verify path through a % source:
alix trace --build mytrace.txt  # let Claude discover the path (writes checkpoints back)
alix trace --suggest .          # recon a source for candidate traces worth authoring
alix explore .                  # an ordered learning plan (decks + traces) toward a goal
alix explore --walk .           # walk an explore tour of the source's shape
alix deps mydeck.txt            # edit a deck's prerequisites (checkbox picker)
alix stats mydeck.txt           # progress overview
alix list mydeck.txt            # every card with stage and due time
alix check mydeck.txt           # lint a deck (syntax, duplicates, trace locators)
alix reset mydeck.txt           # clear stored progress (also --card / --all)
```

A session is one deck file — review them one at a time. Useful flags for
`review`: `--new N` (max unseen cards to introduce, default 10), `--limit N`
(cap session size), `--max-typos N` (fuzzy tolerance per line, default 2).

Run `alix` with no deck arguments (as the desktop launcher does) to open the
**deck picker**, grouped into three sections: **[Workspaces](#workspaces)**
(each showing when it last made progress) · **Recent** (loose decks you reviewed
lately) · **Folders** (plain decks folders). A deck that lives inside a workspace
stays out of Recent — you reach it by opening its workspace. Mastered/done decks
are kept out of Recent (it's a quick launchpad) but stay reachable by filtering;
an exam-locked deck that's still drillable stays in Recent. Decks live in the
decks directory (`~/decks` by default,
set `decks_dir` in the config). The focus is on the **list** by default, with
Vim-style keys (rebindable in the config's `[picker]` section): `j`/`k` (or
`↑`/`↓`) move, `l` (or `Enter`) opens the focused row, `h` (or `Esc`/`Backspace`)
steps back, `m` opens the **Mastered** window (your completed decks, kept out of
Recent), and `/` (or `Ctrl-F`) starts **filtering** by name (searching *every*
loose deck, not just the recent ones); `Esc` leaves the filter. Jumping to the
first/last row stays fixed at `g`/`G` (or Home/End), like the `[browse]` pager.
A deck with nothing to launch right now is dimmed and `Enter` on it does nothing:
🕒 nothing due (all on cooldown — `--cram` reviews it anyway), or a fully-drilled
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
| `#?` front | card | [Cloze card](#cloze-cards-fill-in-the-blank) — blanks are `{{spans}}` in the answer line. |
| `!` line | card | A note shown after you answer. |
| `%` line | anywhere | A comment — ignored, unless it is one of the directives below. |
| `% mode:` | deck · card | [Answer mode](#deck-directives): `flip`, `typing`, `fuzzy`, `choice`, `line`, `explain`. |
| `% order:` | deck | [Card order](#deck-directives): `scheduled` (default) or `sequential`. |
| `% scheduler:` | deck | [Scheduler](#deck-directives): `leitner` (default) or `sm2`. |
| `% direction:` | deck · card | [Review direction](#dual-direction-cards--direction): `forward`, `reverse`, or `both`. |
| `% unlock-stage:` | deck | [Stage that opens the gate](#deck-directives) `1`–`5`: the exam/unlock fires once every card reaches it (cards keep drilling, no early retirement). |
| `% frontend:` | deck · card | [Restrict](#deck-directives) a card/deck to `any`, `tui`, or `web`. |
| `% img:` / `% img-back:` | card | [Image](#images--img--img-back) on the front / back (web frontend). |
| `% img-dir:` | deck | [Base directory](#images--img--img-back) that image filenames resolve against. |
| `% strictness:` | deck | [Exam grading rigor](#the-ai-exam-alix-exam): `strict`, `balanced`, or `lenient`. |
| `% requires:` | deck | [Prerequisite deck](#deck-dependencies) that gates unlocks (repeatable). |
| `% link:` | deck | [ask-Claude reference](#ask-claude-about-a-card) URL — **tutor only** (repeatable). |
| `% source:` | deck | [Exam ground truth](#the-ai-exam-alix-exam) — a URL or file (repeatable). For a [trace](#traces-alix-trace), the source the path runs through (the frozen `assets/` copy in an explored workspace). Also a tutor reference. |
| `% trace:` | deck | What a [trace](#traces-alix-trace) walks — a path description ("how X becomes Y"); its presence makes the deck a trace. |
| `% at:` | card | A locator into the `% source:` (`file:lines`, or just `lines` for a single-file source): a [trace checkpoint's](#traces-alix-trace) reveal target, or a [fact card's source citation](#source-citations--at-on-a-fact-card) shown on reveal. In a frozen workspace it points at the `assets/` snapshot and carries the original location after ` from ` (`29.rs from src/caching.rs:46-66`). |
| `% origin:` | workspace · deck · card | The live source root a frozen deck's snapshots came from (set in a workspace's `alix.toml` at build time). The ask-tutor grounds in it for context and `alix check` reads it to flag drift; `% source:` itself points at the frozen `assets/`. |
| `% given:` | card | A [trace checkpoint's](#traces-alix-trace) "given" (repeatable) — an off-screen symbol the question leans on, as `name — meaning`; shown as a list under the question. |
| `% title:` | deck | [Display name](#workspaces) shown instead of the file name (a workspace sets `title` in its `alix.toml`). |

**`% link:` vs `% source:`** — both point at the material a deck is about, but
they are not interchangeable. `% source:` is the **exam's ground truth**:
questions are generated from it and answers graded against it, and a URL source
*also* doubles as an ask-Claude reference. `% link:` is **only** a tutor
reference and never becomes exam material — use it for supplementary reading (a
blog post, a Stack Overflow answer) you don't want the exam to test. The
implication runs one way: a `% source:` URL is offered to the tutor, but a
`% link:` is never promoted to an exam source.

### Deck directives

A deck can set its own defaults with `% key: value` comment lines in the deck
header (before the first card), so you do not have to repeat flags on the
command line:

```
% mode: line
% order: sequential
% scheduler: sm2
```

- `mode` — default answer mode (`flip`, `typing`, `fuzzy`, `choice`, `line`,
  `explain`);
  can also be overridden per card (see below).
- `order` — `scheduled` (the default) or `sequential` to walk the deck in
  file order, top to bottom (ideal for lyrics with `% mode: line`).
- `scheduler` — `leitner` (default) or `sm2`.
- `direction` — `forward` (default), `reverse`, or `both`; per card or deck-wide
  (see below).
- `frontend` — `any` (default), `tui`, or `web`; restricts a card (or deck) to a
  frontend. Image cards are `web` automatically (see Images below).
- `img-dir` — directory that card `% img:` / `% img-back:` filenames resolve
  against (deck header only; see Images below).
- `unlock-stage` — the Leitner stage (`1`–`5`) every card must reach for the deck
  to **unlock**: a `% source:` deck becomes *exam due* (its exam opens), a
  source-less one becomes *finished* (its dependents unlock). The cards are
  **not** retired — they keep drilling to the top stage; the directive only lowers
  the unlock bar. Default: the deck unlocks only when every card retires at the
  top stage.
- `strictness` — how strictly the AI exam grades answers (`strict`, `balanced`,
  `lenient`); only affects `alix exam` (see [the AI exam](#the-ai-exam-alix-exam)).

These are ordinary `%` comments, so they don't affect parsing and card hashes
are unaffected. An explicit CLI flag always wins over a directive, which wins over
the built-in default. Directives are read only from the deck(s) you ask to
review. When several requested decks disagree on a setting, the default is used.
`alix check <deck>` prints a deck's directives.

**Per-card mode.** A `% mode:` directive placed *after* a card's front (and
before the next one) overrides the deck's mode for that card only, so one deck
can mix modes — e.g. a `line` lyrics card among `flip` cards. The effective mode
is resolved per card: CLI `--mode` > the card's `% mode:` > the deck's
`% mode:` > the default (so `--mode` still forces every card). Other directives
(`order`, `scheduler`) stay deck-level.

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

Every deck has a **completion state**, derived from its cards' stages:
*not started* (no card reviewed), *finished* (every card at the top stage), or
*started* (in between). The deck picker (terminal and web) shows it on each row
— `new`, `m/total` (cards at the top stage), or `done ✓` — and `alix stats`
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
[grounded ask-tutor](#ask-claude-about-a-card) may read this workspace's source,
beating the global `[ask] source_access`), and a `[defaults]` table of directives
shared by every deck:

```toml
# ~/decks/english/alix.toml
title = "English"
description = "everyday conversational vocabulary"
# source_access = true   # let the ask-tutor read this workspace's % source:

[defaults]
direction = "both"
mode = "typing"
```

```
alix workspace ~/decks/english/     # open the workspace; pick a member to review
```

The `[defaults]` keys are the deck directive names, and they fill in only what a
deck *doesn't* set itself, so precedence runs **CLI flag > card > deck >
workspace > default** — set `direction = "both"` once for the whole cluster, and
an individual deck can still override it with its own `% direction:`.

**A workspace keeps its own progress.** Its decks track their stages in a
`progress.json` *inside the workspace folder* (override the path with a
`store = "..."` line in the `alix.toml`), separate from the global store every
loose deck shares. So a workspace is a **self-contained, portable unit** — its
decks, its `assets/` (frozen trace excerpts), and its progress all live in one
folder you can move or share, and its history stays isolated from everything
else. Decks *outside* a workspace keep using the global store; `--store <path>`
overrides either.

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

A front marked `#?` (no space) turns the card into a cloze card: every
`{{...}}` in its answer lines is a hole, and the card expands into one card
per hole. Each one shows the answer with that hole blanked out and the
others filled in, and you only produce the hidden text:

```
#? Complete the Rust declaration
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
single hole with nothing around it (e.g. `` `{{IdentStr}}` ``), `alix check`
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
both` before the first card) with per-card overrides — like `mode`. The two
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
or deck to a frontend explicitly. `alix check` warns about a referenced image
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
fact came from, and `alix check` warns about a citation that no longer resolves
(a moved or shrunk file). In a workspace built with `alix explore --into
--build`, the cited excerpts are also **frozen** into `assets/` (like trace
excerpts), so they never drift and the workspace travels without the upstream
source.

## Review

The default **flip** mode is Anki-style: you reveal the answer and grade
yourself **failed / partly / got it** — the same three grades the trace walk
uses. *Failed* resets the card to stage 1, *partly* drops it one stage (a soft
miss — it returns sooner but you keep most of your progress), and *got it*
advances it one stage. The
**typing** mode has you type the back of the card character by
character with instant green/red feedback; `TAB` reveals the next two
characters as a hint, and pressing it again uncovers two more each time until
the line is fully shown, but a hinted card counts as failed. In **fuzzy** mode
you submit whole lines with Enter and small typos are tolerated. A wrong
card goes back to stage 1 and reappears later in the same session until you
get it right. Whichever mode a card uses is shown as a small badge above the
answer (`flip`, `typing exact`, `typing fuzzy`, `choice`, `line by line`).

In **choice** mode you pick the answer out of four options with `1`–`4`.
The three wrong options are sampled from the answers of the other cards in
the session, preferring similar-looking ones (years compete with years), so
no distractors ever have to be written. Recognition is easier than recall: a
correct pick grades as *got it*, a wrong pick *failed* (an auto-graded mode has
no *partly*).
Cloze siblings are never used as distractors, and if a session has fewer
than four distinct answers the card falls back to flip mode.

For **AI-written** distractors instead — plausible, tempting wrong answers
tailored to each card — augment the deck ahead of time:

```sh
alix deck augment mydeck.txt --target choices --with "use common misconceptions"
```

This generates them once with Claude and caches them by card id (in
`augment.json` beside your progress), so review stays instant and fully offline —
no waiting, no live calls during study. Review reads the cache automatically: a
card with cached distractors uses them, anything else falls back to the offline
sampler above (so a card never loses its options), and because the AI brings its
own wrong answers, choice mode works even on a deck too thin to sample from.
Editing a card's answer regenerates its distractors next time you augment. See
[Augment a deck](#augment-a-deck--alix-deck-augment).

In **line** mode the back is revealed one line at a time: press the reveal
key (`Space`) to uncover the next line, recalling it first. It is meant for
lyrics, poems, or any ordered list. Once every line is shown you grade
yourself failed / partly / got it, exactly like flip mode. Pair it with
`--order sequential` (or `% order: sequential` in the deck) to walk the
sections top to bottom — e.g. one card per verse/chorus of a song.

In **explain** mode the card is an open prompt and its back lines are the **key
points** a good answer should cover (not a string to reproduce). You optionally
type your explanation, reveal the points, and grade yourself on whether you
covered them — for cards aimed at *understanding* rather than recall. Set it with
`% mode: explain` (per card or deck-wide). The typing is optional and never
checked: a self-graded mode can't verify your answer, so it doesn't pretend to
(in the web frontend your typed answer is shown next to the points for honest
comparison). It pairs with the ask-Claude helper, and is the day-to-day,
self-graded tier below the [AI exam](#the-ai-exam-alix-exam).

To throw a card away, press the **remove** key (`Ctrl-X` by default) on it
instead of grading — it is dropped from the session without being asked again
(cloze siblings go too). The marked cards are deleted from their deck files,
and their progress is pruned, when the session ends. The same key works in
`alix browse`.

Schedulers:

- **leitner** (default): a 6-stage box system with cooldowns
  0 / 1 h / 6 h / 24 h / 1 week. Pass moves a card up one stage, fail resets
  it to stage 1.
- **sm2**: SuperMemo-2 style with per-card ease factors and growing
  intervals. Existing Leitner progress is used as a starting point, and the
  Leitner stage is kept in sync, so you can switch back and forth.

## Browse

`alix browse <deck>` is a walk through one deck's cards — front and back shown
together, in file order — without grading or scheduling. It is for a first
read-through of a new deck or just checking its contents, without affecting
your schedule. Navigate with `l`/`h` (next/previous, vim-style — `n`/`p`, the
arrow keys, and `Space` also work), `g`/`G` (first/last, also Home/End), and
`q` to quit. Pressing the remove key (`x` by default) marks the current card;
on quit the marked cards are deleted from their deck files and their progress
is pruned — the only thing browsing ever writes. The next/previous/remove/quit
keys are configurable in the `[browse]` section of the config file (see below);
first/last stay `g`/`G`. Run `alix browse` with no deck argument to choose
decks from the same picker `alix` uses.

## Web frontend

Add `--serve` to `review` or `browse` to run it in the browser instead of the
terminal — useful on a tablet or phone, where touch (and images) beats a TUI.
It runs the same session logic and writes to the same progress store, so
a card you grade or remove in the browser shows up on the command line and vice
versa.

```
alix review rust.txt --serve              # open http://127.0.0.1:7777
alix review rust.txt --serve --port 8080
alix review rust.txt --serve --lan        # reachable from other devices on your network
alix browse rust.txt --serve              # the browse view in the browser
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
there. Keyboard nav follows your `[picker]` config (`j`/`k` or arrows move, `/`
or `Ctrl-F` filter, `m` the Mastered window). When you finish a session, "Choose
other decks" (on the summary, or in the ⋮ menu) returns here — and a session
launched inside a workspace returns **into that workspace**. Naming decks on the
command line skips the screen and goes straight to review/browse.

Every answer mode works in the browser: **flip** (reveal, then self-grade
Failed / Partly / Got it), **line** (reveal a verse one line at a time — it
auto-scrolls to follow the newest line), **typing** / **fuzzy** (type your
answer and submit; checked exactly or with your configured typo tolerance, each
line marked ✓/✗ with the correct answer shown), and **choice** (tap one of the
options). The note appears once the answer is shown. Controls are big tap
targets and follow your configured key bindings — the page reads them from the
server, so the chips show your own keys. The overflow menu (⋮) holds **Remove**,
which deletes the current card from its deck file and prunes its progress, and
**Choose decks**, which returns to the deck-selection screen.

A **gallery of themes** ships with the web UI — the alix **Dark**/**Light**
originals and a playful **Kid** theme, plus crowd-favourite editor/slide palettes
(GitHub, Dracula, Nord, Solarized, Gruvbox, Catppuccin, Tokyo Night, Monokai, One
Dark, Ayu, Rosé Pine, Everforest). Open the **Theme…** popover from the ⋮ menu — a
grid grouped Light / Dark that previews the whole UI live as you hover, and
remembers your choice in the browser; no configuration needed.

It is deliberately local-only — no accounts, no database. By default it binds
to `127.0.0.1` (this machine only); `--lan` binds all interfaces so a device on
the same network can reach it at `http://<your-machine-ip>:<port>` (no
authentication, so only use `--lan` on a network you trust). `--port` and
`--lan` require `--serve`; the default port lives in the `[serve]` section of
the config file and `--port` overrides it.

## Ask Claude about a card

On any post-answer screen (feedback, revealed flip card, answered choice),
press `?` to ask Claude about the card without leaving the session. The
card (front, answer, note, deck name) is sent as context to the Claude Code
CLI (`claude -p`), so you can ask "why is that the answer?" and follow up.
One CLI conversation spans the whole review run (`--session-id` on the
first question, `--resume` afterwards), so Claude remembers earlier cards
and questions — but the panel shows only the **current card's** exchanges, not
the whole history. By default the tutor answers from the card text plus its own
knowledge (tools: `WebFetch`/`WebSearch`), and it uses the **CLI's default
model** — set `[ask] model`/`effort` to pin a stronger one (the web panel shows
which model is answering). For a deck built from source, set **`[ask]
source_access = true`** to let the tutor **read the card's source** to verify its
answer: it runs `Read`/`Glob`/`Grep` with its working directory at the deck's
`% source:` project root (the nearest `Cargo.toml`/`.git`/… above the cited
files) and is told to check the real files before answering. It's off by default
because it grants the tutor file-read access — only enable it on a machine and
network you trust (especially with `--serve --lan`). A [workspace](#workspaces)
can override it per-folder: put `source_access = true` (or `false`) in its
`alix.toml` to decide for that crate alone, beating the global default. While
Claude thinks, the session stays responsive; Esc returns exactly where you were.

This works in the **web frontend** too (`--serve`): an "Ask" button (and the
`?` key) on an answered card opens a chat panel — type a question, **Send**,
**Save note**, **Close**. The server runs `claude -p` on a background thread and
the page polls for the reply, so the single-threaded server never blocks. Ask is
reachable wherever you serve, including `--lan` (the request runs `claude` on the
host, so — like `--lan` generally — only use it on a network you trust).

While typing a question you can edit it like a normal input line: `←`/`→`
move the caret, `Home`/`End` (or `Ctrl-A`/`Ctrl-E`) jump to the ends, and
`Backspace`/`Delete` remove the character before/under it.

Decks can carry reference links as comment lines:

```
% link: https://docs.rs/async-compat
% link: https://tokio.rs/tokio/tutorial
```

They are handed to Claude with the first question as background material to
consult when useful — fetched once, remembered for the rest of the run. These
lines do not affect card hashes.

Because the CLI runs headless (`-p`), it cannot show interactive permission
prompts — an unanswerable prompt would hang the call. `alix` therefore
runs it with `--permission-mode dontAsk` and an exclusive tool allowlist
(`WebFetch`, `WebSearch` by default): the listed tools work without
prompting, and every other tool is silently denied, so a malicious page
behind a deck link cannot make the tutor run shell commands. Both the
permission mode and the allowlist are configurable in `[ask]`.

`Ctrl-N` condenses the conversation into at most three short note lines and
appends them to the card in the deck file (notes are not hashed, so the
card's progress is untouched). Requires the `claude` CLI to be installed
and logged in; the command, a `--model` override and the timeout are
configurable in the `[ask]` section of the config file.

## Generate a facts deck — `alix deck generate`

`alix deck generate <source>` turns a **source** into a deck of fact cards using
the Claude CLI. The source is a web page URL *or* a local file/directory path
(the deck-side mirror of `alix trace`):

```sh
alix deck generate https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
alix deck generate src/scheduler.rs        # a local file (or a directory)
alix deck generate <source> -o ownership   # choose the file name
alix deck generate <source> --cards 15     # cap the number of cards
alix deck generate <source> --review       # 2nd pass to remove redundant cards
alix deck generate <source> --print        # print to stdout instead of writing
```

For a **web page**, Claude reads it with the **WebFetch** tool and the deck
starts with a `% link:` line back to it (so you can use the *ask* feature on the
cards). For a **local source**, Claude explores it read-only with
`Read`/`Glob`/`Grep` at the source root and the deck starts with a `% source:`
line (so `alix exam` can later grade your understanding against it); it also
adds a [`% at:` citation](#source-citations--at-on-a-fact-card) to each fact that
maps to specific lines, so the card can show its source on reveal. Either way
Claude only returns text — never a write or shell tool — and `alix` validates it
(`parse_str`) and writes `~/decks/<slug>.txt`. The cards are spread across four
layers of understanding (facts → concepts → application → connections) and use
cloze (`#?`) cards for terminology.

The prompt tells Claude to draft, then re-read the whole set and merge or drop
cards that test the same fact, so the deck doesn't repeat itself. For a
stronger pass, `--review` (or `generate.review = true`) runs a **second**
Claude call that takes the draft and returns a deduplicated, tightened version
— an extra call, worth it when the source is repetitive.

The prompt and limits live in the `[generate]` section of the config:
`model`, `timeout_secs` (default 300), `max_cards` (default 30), `extra`
(guidance appended to the built-in prompt), `prompt` (a full override, with
`{url}` and `{max_cards}` placeholders), and `review`. It reuses the `[ask]`
command and permission settings. Review the result before relying on it — it is a starting
point, not a final deck. Generation needs the `claude` CLI installed and
logged in, and works best on a single page or chapter (a whole book overruns
the context budget).

## Augment a deck — `alix deck augment`

`alix deck augment <deck> --target <kind>` enriches an *existing* deck with
Claude. It's a **deliberate, one-off command** — generation happens here, in the
foreground (so any error surfaces immediately), and the result is cached beside
your progress (`augment.json`, keyed by card id). Review then reads the cache, so
study stays instant and fully offline — Claude is never called mid-session.

```sh
alix deck augment mydeck.txt --target choices    # multiple-choice distractors
alix deck augment mydeck.txt --target notes      # a trivia / mnemonic note per card
alix deck augment mydeck.txt --target questions  # reworded phrasings of each question
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
- **`--target topology`** *(experimental)* derives a **graph of how the deck's
  cards relate** — labeled edges, a suggested **walk**, and a handful of coarse
  named **regions** — cached like the rest. A deck can hold several topologies,
  one per `--with` principle and keyed by it (`auto` when none). `alix review
  <deck> --topology <name>` then serves the **due** cards in that walk's order
  instead of at random — SRS still decides *which* cards are due, the topology
  only reorders them — and review shows a thin **region breadcrumb** ("where am
  I", current emphasized) so the sequence reads as a path, not a shuffle. With a
  single cached topology, `--topology` (no name) picks it automatically.
- **`--with "<guidance>"`** steers *how* (e.g. "use common misconceptions",
  "add a surprising historical fact", "phrase questions as real-world scenarios",
  or a topology principle like "by type dependency" / "north to south").

Nothing here touches a card's identity, so augmenting never resets progress
(distractors, notes, and variants all key off the answer, which the id hashes —
not the front); editing a card's answer changes its id, so it simply regenerates
next time you augment. Tuned under `[ai]` (`model`, `distractor_count`,
`variant_count`, `timeout_secs`). Augmentation needs the `claude` CLI installed
and logged in.

## Import an Anki deck (`alix import`)

`alix import <file.tsv>` turns an Anki export into an `alix` deck — no Claude
needed. Export your notes from Anki as **Notes in Plain Text** (`.txt`/`.tsv`)
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

A URL `% source:` doubles as an [ask-Claude](#ask-claude-about-a-card) reference,
so you don't need to repeat it as a `% link:`. The reverse isn't true: a
`% link:` stays a tutor reference and never becomes exam ground truth — keep
supplementary links (a blog, an SO answer) as `% link:` so the exam ignores them.

Once every card in a `% source:` deck reaches the top stage, the deck is **exam
due** rather than finished (so it does not yet unlock its dependents). To open
the exam earlier — while the cards keep drilling — set `% unlock-stage: N`: the
deck turns exam due once every card reaches stage `N` (see
[deck directives](#deck-directives)).

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

`alix` asks Claude to read the source (URLs via the **WebFetch** tool; local
files are embedded) and write fresh **open understanding** questions —
application and connections, not the card facts — each with the key points a
correct answer must contain. You type a prose answer per question, and an
examiner grades them Pass / Partial / Fail **against the source's rubric, never
against your cards** (grading the cards would be circular), at the deck's
configured strictness (below). The Claude calls run
on a background thread so the UI stays responsive while it thinks.

- **Pass** (every question by default; tune with `pass_threshold`) marks the
  deck **mastered** (shown as `mastered ✓`). Mastery — not mere drilling — is
  what unlocks decks that `% requires:` it. Source-less decks are unchanged:
  finishing = drilled (`done ✓`).
- **Fail** lists the gaps and offers to turn them into remediation cards
  appended to the deck — the card type is picked per gap (a cloze/plain card for
  a missed fact, a `% mode: explain` card for a missed concept), with overlapping
  gaps merged into one card. Re-drill those and re-sit.

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
permission mode and tool allowlist, and needs the `claude` CLI installed and
logged in.

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
% trace: how pressing the Good key in the browser becomes a saved grade
% source: ..

# You press Good. What does the page send the server — and what does it not?
	grade(g) POSTs to /api/grade with a body of just { grade: g } — no card id.
	% at: assets/serve/review.html:338-341
	! The page is a thin view; it doesn't even track card identity.

# So the request has no card id. How does the server know which card you graded?
	The handler grabs the live, server-side review session and grades on it.
	% at: src/serve.rs:682-689
	! State lives server-side; the page only ever names the grade.
```

The locator is a single contiguous range `file:start-end` (e.g.
`src/serve.rs:682-689`), or — when `% source:` is a single file — just the line
numbers; it is never comma-separated (a stitched excerpt makes disjoint code look
adjacent). The lines are **read live from the source** each time you walk, so the excerpt is
always current and the deck stays small; the source is the oracle, not an
invented answer. When a tight excerpt uses a symbol defined off-screen, name it
with a `% given:` line (`% given: state — the parser's position so far`,
repeatable) — these show as a list under the question, so the excerpt stays
focused without orphaning the names it leans on.

**Building it with Claude.** Instead of hand-writing the checkpoints, declare
just the `% trace:` and `% source:`, then run `alix trace --build <deck>`:
Claude explores the source — with **read-only** `Read`/`Glob`/`Grep` and the
source root as its working directory (no write or shell access) — traces the
single load-bearing path, and writes the checkpoints (with their `% at:`
locators) back into the deck file. The result is cached and version-controlled
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
its original line numbers, so when those matter the original `file:lines` is kept
in the card's note (`! from scheduler.rs:90-98`). It's automatic for explored
workspaces, not a command; a loose trace over a live `% source:` is left as-is.
See [docs/traces.md](docs/traces.md) for the rationale.

**Checking the locators.** For a trace that *isn't* frozen — a loose `.txt` over
a live `% source:` — **`alix check`** validates that every `% at:` still
resolves into its source: it warns about a locator that names a missing file,
runs past the end of the file, or (for a single-file source) gives bare line
numbers it can't place. It's a quick structural check — *does this excerpt still
exist?* — so a moved or trimmed source is caught before you walk into it, not
mid-hop. (Frozen snapshots don't move, but their snippets are validated the same
way.)

Because building is one-shot, correctness-critical, and **fails silently** when
the model is weak (you still get parseable checkpoints, just a loose chain you
then drill), the `[trace]` config section defaults it to a strong model
(`model = "opus"`) and high reasoning effort (`effort = "high"`) — slower than
the other AI features, but it runs once and is amortized over many reviews.
Override `model`, `effort` (`low`–`max`) and `timeout_secs` there. `--suggest`
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
understand the source), so this hands that judgment to Claude.

**Write it as a chain, not a quiz.** A trace's whole value is that it's a *path*:
each checkpoint should pick up where the last *reveal* left off (note how hop 2
above opens with hop 1's conclusion, "the request has no card id"), so you're
following one thread — a data flow, a control flow, a derivation — to the
outcome. If the checkpoints are independent facts that all hang off one thing, you've
written a *set*, which is what cards and the exam already do; reach for a subject
that has a real sequence.

**Walking it** (`alix trace keypress-to-grade.txt`) goes hop by hop:

1. **Predict** — you type a guess before anything reveals (committing is the
   point).
2. **Reveal** — `alix` prints the real excerpt from the source, then the
   checkpoint's key points and note.
3. **Gap** — you judge yourself **Got it / Partial / Missed**. Grading is
   self-judged and offline (no model call) by default; pass **`--grade`** to have
   Claude judge your typed prediction against the key points and return the
   verdict plus a line of feedback (a model call per hop). Either way, a Partial
   or Missed is a **weak edge** that resets so it resurfaces sooner, while a
   nailed hop advances and fades. Each checkpoint is an ordinary card under the
   hood, so this scheduling is the normal per-card SRS.
4. **Compress** — after the last hop you restate the whole path in two
   sentences: if you can re-derive it, you understood it.

**In the browser** — add **`--serve`** (`alix trace <deck> --serve`) to walk it
in the web frontend instead of the terminal, the same way `review`/`browse`
serve. The walk page shows the **path** as a rail you descend (its nodes color in
by Got / Partial / Missed) and reveals each checkpoint's real source in a
line-numbered excerpt; `--serve --grade` runs the live grading and the page waits
on Claude per hop. `--port`/`--lan` work as elsewhere. Progress saves to the same
store, so a walk started in the terminal continues in the browser.

`alix trace <deck> --map` prints the path (every prompt, its key points and
locator) without quizzing — a quick "just show me the route".

**A trace's exam is the compression.** Walking the checkpoints is the *drill*;
the *verification* is `alix exam <trace>` — one fixed question, the `% trace:`,
which you answer by retracing the whole path in a sentence or two. Claude grades
that against the path's checkpoints (AI-graded, like a fact deck's exam), and
**passing masters the trace** (unlocking its dependents). You reach it three
ways: directly with `alix exam <trace>`, as the **capstone** offered at the end
of a walk, or via the picker's **"Take exam"** — and, like a fact deck, you can
sit it early to *test out* without walking. A failed trace exam is **re-walked**,
not remediated into cards (a trace is a path, not a card pile); after a fail it
**cools down** before you can re-sit (so the graded feedback can't just be pasted
back — `[exam] retry_cooldown_secs`, default 1h).

A trace deck degrades gracefully — even without `alix trace` it is a valid deck
of `explain` cards. See `examples/keypress-to-grade.txt` for a complete trace
over this repo's own source.

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
items yourself (`alix trace --build` a trace, `alix deck` a facts deck).

With **`--into <dir>`** it materializes the plan into a ready-made **workspace**:

```
alix explore . --goal "how review scheduling works" \
  --into ~/decks/scheduling/ --title "Scheduling internals"
```

That writes an `alix.toml` and one stub file per item — a `% trace:` deck for
each trace (run `alix trace --build` on it) and a `% title:` facts deck for each
deck (author it or `alix deck`) — wired together with `% requires:` so they
unlock in dependency order, with each `% source:` pointing back at the real
source. The `--goal` becomes the workspace's `description`; **`--title`** names
it (omitted, the folder name is used); **`--unlock-stage <1–5>`** writes a shared
`[defaults]` `unlock-stage` so a member unlocks once its cards reach that stage
(without retiring early). (Refuses a non-empty folder unless `--force`.)

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

```toml
[keys]
failed = ["j"]
partly = ["k"]
got = ["l"]
```

Keys are written as a single character (`"j"`), a special key name
(`"space"`, `"enter"`, `"tab"`, `"esc"`, `"backspace"`), or either with a
`ctrl-` prefix (`"ctrl-s"`). Rebindable actions: `failed`, `partly`, `got`,
`reveal`, `hint`, `submit`, `skip`, `remove` (mark the card for deletion from
its deck file, default `ctrl-x`), `continue`, `restart` (start a new session
from the summary screen, default `r`), `quit`. While you are typing
an answer (typing and fuzzy mode), plain character bindings are ignored so
they cannot shadow text input — use `ctrl-`/special keys for `hint`, `skip`
and `quit`. A different config file can be passed with `--config <path>`.

The browser (`alix browse`) has its own bindings in a `[browse]` section:

```toml
[browse]
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
