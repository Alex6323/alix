# flash

An **AI-augmented spaced-repetition learning tool** for the terminal and the
web. At its core it's a fast, plain-text flashcard trainer — Leitner and SM-2
scheduling, several answer modes, cloze and dual-direction cards, images, and
deck dependencies. What sets it apart is the **Claude integration**: an
*ask-Claude tutor* on any card, *AI deck generation* from a web page,
*understanding cards*, and an **AI exam** (`flash exam`) that verifies you
actually grasped the material and gates your progress on passing it — so you're
not just memorizing, you're being checked. Decks stay simple plain-text files
you own, reviewable in a ratatui terminal UI or a local web app.

## Requirements

The flashcard **core** — review, scheduling, every answer mode, browse, the TUI
and the web frontend — runs standalone, with no external services or accounts.

The **AI features** shell out to the [Claude Code](https://www.anthropic.com/claude-code)
CLI, so they need it **installed and logged in** (which in turn needs a Claude
subscription or API access). Install the CLI and run `claude` once to
authenticate. The features that require it:

- `flash deck` — generate a facts deck from a URL or a local file/directory;
- `flash exam` — the AI exam;
- `flash trace --build` / `--suggest` / `--grade` — discover, suggest, and grade
  traces;
- `flash explore` — goal-driven learning plans;
- ask-Claude (`?`) — the in-session tutor, in both the TUI and the web frontend.

flash invokes the CLI headless (`claude -p`) under a locked-down permission
model (see [Ask Claude](#ask-claude-about-a-card)); the command, model and
timeouts are configurable per feature in the [config file](#configuration).

## Usage

The binary is called `flash`:

```sh
flash                            # pick decks interactively (recent + ~/decks)
flash mydeck.txt                 # review due cards (flip mode, Leitner)
flash --mode typing mydeck.txt   # type the answer character by character
flash --mode fuzzy mydeck.txt    # whole-line input, small typos tolerated
flash --mode choice mydeck.txt   # multiple choice (distractors from the deck)
flash --mode line mydeck.txt     # reveal the answer one line at a time (lyrics)
flash --scheduler sm2 mydeck.txt # SM-2 intervals instead of Leitner
flash --cram mydeck.txt          # ignore cooldowns, review everything
flash browse mydeck.txt          # read through cards, no grading or scheduling
flash deck <url-or-path>         # generate a facts deck from a web page or a file/dir
flash import cards.tsv            # import an Anki TSV (front<TAB>back) into a deck
flash exam mydeck.txt            # AI exam against the deck's % source: (gates unlocks)
flash trace mytrace.txt          # walk a predict-and-verify path through a % source:
flash trace --build mytrace.txt  # let Claude discover the path (writes checkpoints back)
flash trace --suggest .          # recon a source for candidate traces worth authoring
flash explore .                  # an ordered learning plan (decks + traces) toward a goal
flash explore --walk .           # walk an explore tour of the source's shape
flash deps mydeck.txt            # edit a deck's prerequisites (checkbox picker)
flash stats mydeck.txt           # progress overview
flash list mydeck.txt            # every card with stage and due time
flash check mydeck.txt           # lint a deck (syntax, duplicates)
flash reset mydeck.txt           # clear stored progress (also --card / --all)
```

Several decks can be given at once; their due cards are merged into one
session. Useful flags for `review`: `--new N` (max unseen cards to introduce,
default 10), `--limit N` (cap session size), `--max-typos N` (fuzzy tolerance
per line, default 2).

Run `flash` with no deck arguments (as the desktop launcher does) to open the
**deck picker**, grouped into three sections: **[Workspaces](#workspaces)**
(each showing when it last made progress) · **Recent** (loose decks you reviewed
lately) · **Folders** (plain decks folders). A deck that lives inside a workspace
stays out of Recent — you reach it by opening its workspace. Mastered/done and
locked decks are also kept out of Recent (it's a quick launchpad) but stay
reachable by filtering. Decks live in the decks directory (`~/decks` by default,
set `decks_dir` in the config). The focus is on the **list** by default, with
Vim-style keys (all rebindable in the config's `[picker]` section): `j`/`k` (or
`↑`/`↓`) move, `g`/`G` jump to ends, `l` (or `Enter`) opens the focused row, `h`
(or `Esc`) steps back, `m` opens the **Mastered** window (your completed decks,
kept out of Recent), and `/` (or `Ctrl-F`) starts **filtering** by name (searching
*every* loose deck, not just the recent ones); `Esc` leaves the filter. A deck you
can't start right
now is dimmed and `Enter` on it does nothing: 🔒 a
[locked](#completion-states--unlocks) deck (an unfinished `% requires:`), 🕒 a deck
with nothing due (all on cooldown — `--cram` reviews it anyway); a mastered deck
reads `mastered 🎉`. `Enter` on a **Workspace** or **Folder** opens it (drills in) — `Esc`
or `Backspace` steps back out to the list, all within one screen; a
[trace](#traces-flash-trace) opened from the picker **walks** instead of being
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
| `% max-stage:` | deck | [Top Leitner stage](#deck-directives) `1`–`5`; reaching it retires the card. |
| `% frontend:` | deck · card | [Restrict](#deck-directives) a card/deck to `any`, `tui`, or `web`. |
| `% img:` / `% img-back:` | card | [Image](#images--img--img-back) on the front / back (web frontend). |
| `% img-dir:` | deck | [Base directory](#images--img--img-back) that image filenames resolve against. |
| `% strictness:` | deck | [Exam grading rigor](#the-ai-exam-flash-exam): `strict`, `balanced`, or `lenient`. |
| `% requires:` | deck | [Prerequisite deck](#deck-dependencies) that gates unlocks (repeatable). |
| `% link:` | deck | [ask-Claude reference](#ask-claude-about-a-card) URL — **tutor only** (repeatable). |
| `% source:` | deck | [Exam ground truth](#the-ai-exam-flash-exam) — a URL or file (repeatable). For a [trace](#traces-flash-trace), the source the path runs through (the frozen `assets/` copy in an explored workspace). Also a tutor reference. |
| `% trace:` | deck | What a [trace](#traces-flash-trace) walks — a path description ("how X becomes Y"); its presence makes the deck a trace. |
| `% at:` | card | A [trace checkpoint's](#traces-flash-trace) locator into the `% source:` (`file:lines`, or just `lines` for a single-file source). |
| `% given:` | card | A [trace checkpoint's](#traces-flash-trace) "given" (repeatable) — an off-screen symbol the question leans on, as `name — meaning`; shown as a list under the question. |
| `% title:` | deck | [Display name](#workspaces) shown instead of the file name (a workspace sets `title` in its `flash.toml`). |

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
- `max-stage` — the deck's top Leitner stage, `1`–`5` (default `5`). A card that
  reaches it **retires**: it rests and is no longer scheduled (not even under
  `--cram`) until `flash reset`. Use it for material you only need a couple of
  times — `% max-stage: 1` means "get it right once and it's done." With the
  default `5`, a card still retires once it climbs to stage 5. A deck counts as
  *finished* (see Completion states) once all its cards have retired.
- `strictness` — how strictly the AI exam grades answers (`strict`, `balanced`,
  `lenient`); only affects `flash exam` (see [the AI exam](#the-ai-exam-flash-exam)).

These are ordinary `%` comments, so they don't affect parsing and card hashes
are unaffected. An explicit CLI flag always wins over a directive, which wins over
the built-in default. Directives are read only from the deck(s) you ask to
review — never from prerequisites pulled in via `% requires:` (see below). When
several requested decks disagree on a setting, the default is used. `flash check
<deck>` prints a deck's directives.

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

When you review a deck, its prerequisites are pulled in automatically —
transitively, de-duplicated — and the session is ordered **foundations
first**: a prerequisite deck's cards (both due reviews and newly introduced
ones) come before the cards of the deck that depends on it, while normal
scheduler order is kept within each deck. A prerequisite name resolves next to
the requiring deck or in the decks directory, with or without `.txt`. Missing
prerequisites and dependency cycles are reported as errors. They are ordinary
`%` comments, so hashes are unaffected.

A prerequisite contributes only its **cards** to the session, not its
directives: the `mode`, `order` and `scheduler` come from the deck you asked to
review, so requiring a `% mode: line` lyrics deck as a prerequisite won't switch
your session into line mode. Dependencies apply to `review` only — `browse` and
`stats` operate on exactly the decks you name.

You can edit a deck's prerequisites without hand-typing (and without typos)
with `flash deps <deck>` (alias `flash require`): it opens the deck picker over your
decks directory, pre-ticked to the current prerequisites. `Space` toggles,
`Enter` saves (rewriting the `% requires:` lines), `Esc` cancels; unticking
everything clears them. Since the lines are comments, editing dependencies
never affects card progress.

### Completion states & unlocks

Every deck has a **completion state**, derived from its cards' stages:
*not started* (no card reviewed), *finished* (every card at the top stage), or
*started* (in between). The deck picker (terminal and web) shows it on each row
— `new`, `m/total` (cards at the top stage), or `done ✓` — and `flash stats`
prints it too. A deck that declares a `% source:` adds one more state between
drilled and finished — *exam due* (`exam due`, tinted) — because drilling alone
no longer finishes it; see [the AI exam](#the-ai-exam-flash-exam).

Completion drives **unlocks**, with no extra syntax: a deck is **locked** while
any of its `% requires:` prerequisites isn't finished, so finishing a foundation
unlocks the decks that build on it. Locked decks are shown dimmed with a 🔒, but
the lock is **advisory** — you can still pick one (its prerequisite cards are
pulled in foundations-first anyway). State and locks are recomputed live, so if a
finished deck later lapses below the top stage, its dependents lock again.

### Workspaces

A **workspace** is a folder of decks reviewed together with shared directives —
ideal for a cluster like all your vocabulary decks. A folder becomes a workspace
when you drop a **`flash.toml`** in it — a scoped version of the
[config file](#configuration) — setting a title and a `[defaults]` table of
directives shared by every deck:

```toml
# ~/decks/english/flash.toml
title = "English"

[defaults]
direction = "both"
mode = "typing"
```

```
flash review ~/decks/english/        # review every deck in the folder, together
```

The `[defaults]` keys are the deck directive names, and they fill in only what a
deck *doesn't* set itself, so precedence runs **CLI flag > card > deck >
workspace > default** — set `direction = "both"` once for the whole cluster, and
an individual deck can still override it with its own `% direction:`.

**A workspace keeps its own progress.** Its decks track their stages in a
`progress.json` *inside the workspace folder* (override the path with a
`store = "..."` line in the `flash.toml`), separate from the global store every
loose deck shares. So a workspace is a **self-contained, portable unit** — its
decks, its `assets/` (frozen trace excerpts), and its progress all live in one
folder you can move or share, and its history stays isolated from everything
else. Decks *outside* a workspace keep using the global store; `--store <path>`
overrides either.

**Workspaces** and plain **Folders** appear in their own picker sections
(terminal and web): a folder *with* a `flash.toml` shows under **Workspaces**, one
*without* as a plain **Folder**. Both open (drill in) to their
members drawn as an **unlock dependency tree**: a deck nests under the
`% requires:` prerequisite that gates it, foundations at the roots, and siblings
ordered startable-first. Each row is badged `· trace ·` or `· deck ·`. The drill-in
is a **single-launch list** (no checkboxes): `Enter` on a facts deck reviews it,
`Enter` on a trace **walks** it. (Typing a filter flattens the tree to a plain
search; `flash review <folder>` reviews the whole cluster merged.) A manifest-less
folder is still reviewable (`flash review <folder>`), it just applies no shared
directives.

**`flash workspace <dir>`** opens a workspace straight into that same drill-in
picker, routing each member to the right experience — a **facts deck** to a
review, a **trace deck** to a [predict-verify walk](#traces-flash-trace) —
returning you to the picker when done. (`flash review <dir>` instead flattens the
whole folder into one review, so trace decks get quizzed as flat cards.)

**`% title:`** (on a deck) or **`title`** (in a workspace's `flash.toml`) gives a
display name, shown in the picker, the session header, `flash list` and `flash
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
In the terminal, `flash review` skips such cards with a note, and if a whole
deck is web-only it points you at `--serve`. Use `% frontend:` to force a card
or deck to a frontend explicitly. `flash check` warns about a referenced image
file that doesn't exist (it doesn't fail the check).

## Review

The default **flip** mode is Anki-style: you reveal the answer and grade
yourself (again / good / easy — easy jumps two Leitner stages). The
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
correct pick grades as *good* (never *easy*), a wrong pick fails the card.
Cloze siblings are never used as distractors, and if a session has fewer
than four distinct answers the card falls back to flip mode.

In **line** mode the back is revealed one line at a time: press the reveal
key (`Space`) to uncover the next line, recalling it first. It is meant for
lyrics, poems, or any ordered list. Once every line is shown you grade
yourself again / good / easy, exactly like flip mode. Pair it with
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
self-graded tier below the [AI exam](#the-ai-exam-flash-exam).

To throw a card away, press the **remove** key (`Ctrl-X` by default) on it
instead of grading — it is dropped from the session without being asked again
(cloze siblings go too). The marked cards are deleted from their deck files,
and their progress is pruned, when the session ends. The same key works in
`flash browse`.

Schedulers:

- **leitner** (default): a 6-stage box system with cooldowns
  0 / 1 h / 6 h / 24 h / 1 week. Pass moves a card up one stage, fail resets
  it to stage 1.
- **sm2**: SuperMemo-2 style with per-card ease factors and growing
  intervals. Existing Leitner progress is used as a starting point, and the
  Leitner stage is kept in sync, so you can switch back and forth.

## Browse

`flash browse <deck>...` is a walk through every card — front and back shown
together, in file order — without grading or scheduling. It is for a first
read-through of a new deck or just checking its contents, without affecting
your schedule. Navigate with `l`/`h` (next/previous, vim-style — `n`/`p`, the
arrow keys, and `Space` also work), `g`/`G` (first/last, also Home/End), and
`q` to quit. Pressing the remove key (`x` by default) marks the current card;
on quit the marked cards are deleted from their deck files and their progress
is pruned — the only thing browsing ever writes. The next/previous/remove/quit
keys are configurable in the `[browse]` section of the config file (see below);
first/last stay `g`/`G`. Run `flash browse` with no deck argument to choose
decks from the same picker `flash` uses.

## Web frontend

Add `--serve` to `review` or `browse` to run it in the browser instead of the
terminal — useful on a tablet or phone, where touch (and images) beats a TUI.
It runs the same session logic and writes to the same progress store, so
a card you grade or remove in the browser shows up on the command line and vice
versa.

```
flash review rust.txt --serve              # open http://127.0.0.1:7777
flash review rust.txt --serve --port 8080
flash review rust.txt --serve --lan        # reachable from other devices on your network
flash browse rust.txt --serve              # the browse view in the browser
flash --serve                              # no decks -> pick them in the browser
```

Run `--serve` **without** naming any decks and the browser opens a
deck-selection screen that mirrors the terminal [picker](#getting-started): the
same three sections — **Workspaces** (each with its last-progress time) ·
**Recent** loose decks · **Folders** — and the same **single-launch**, so you
**click a deck to start it** (an exam-due deck sits its exam). Open a
**Workspace** or **Folder** to drill into its **unlock dependency tree**, where
each deck nests under the prerequisite that gates it. A deck you can't start is
dimmed: 🔒 locked (an unfinished `% requires:`), 🕒 nothing due; a `mastered 🎉`
deck is tucked into the **Mastered window** (press `m`), and mastered/done/locked
decks stay out of Recent (a quick launchpad) but are reachable by filtering — the
filter searches *every* loose deck. `browse` ignores locking, so any deck opens
there. Keyboard nav follows your `[picker]` config (`j`/`k` or arrows move, `/`
or `Ctrl-F` filter, `m` the Mastered window). When you finish a session, "Choose
other decks" (on the summary, or in the ⋮ menu) returns here — and a session
launched inside a workspace returns **into that workspace**. Naming decks on the
command line skips the screen and goes straight to review/browse.

Every answer mode works in the browser: **flip** (reveal, then self-grade
Again / Good / Easy), **line** (reveal a verse one line at a time — it
auto-scrolls to follow the newest line), **typing** / **fuzzy** (type your
answer and submit; checked exactly or with your configured typo tolerance, each
line marked ✓/✗ with the correct answer shown), and **choice** (tap one of the
options). The note appears once the answer is shown. Controls are big tap
targets and follow your configured key bindings — the page reads them from the
server, so the chips show your own keys. The overflow menu (⋮) holds **Remove**,
which deletes the current card from its deck file and prunes its progress, and
**Choose decks**, which returns to the deck-selection screen.

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
and questions. While Claude thinks, the session stays responsive; Esc
returns exactly where you were.

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
prompts — an unanswerable prompt would hang the call. flash therefore
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

## Generate a facts deck — `flash deck`

`flash deck <source>` turns a **source** into a deck of fact cards using the
Claude CLI. The source is a web page URL *or* a local file/directory path (the
deck-side mirror of `flash trace`):

```sh
flash deck https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
flash deck src/scheduler.rs            # a local file (or a directory)
flash deck <source> -o ownership       # choose the file name
flash deck <source> --cards 15         # cap the number of cards
flash deck <source> --review           # add a 2nd pass to remove redundant cards
flash deck <source> --print            # print to stdout instead of writing
```

For a **web page**, Claude reads it with the **WebFetch** tool and the deck
starts with a `% link:` line back to it (so you can use the *ask* feature on the
cards). For a **local source**, Claude explores it read-only with
`Read`/`Glob`/`Grep` at the source root and the deck starts with a `% source:`
line (so `flash exam` can later grade your understanding against it). Either way
Claude only returns text — never a write or shell tool — and flash validates it
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

## Import an Anki deck (`flash import`)

`flash import <file.tsv>` turns an Anki export into a flash deck — no Claude
needed. Export your notes from Anki as **Notes in Plain Text** (`.txt`/`.tsv`)
with fields separated by a tab; the first field becomes the front, the second
the back, and any further fields are ignored:

```sh
flash import french.tsv                 # writes ~/decks/french.txt
flash import french.tsv -o vocab        # choose the deck name
flash import french.tsv --print         # print to stdout instead of writing
flash import french.tsv --force          # overwrite an existing deck
```

It skips Anki's `#`-prefixed header lines (`#separator:tab`, `#html:true`, …),
turns `<br>` tags into separate answer lines, decodes the common HTML entities
(`&amp;`, `&lt;`, `&nbsp;`, …), and backslash-escapes a back line that would
otherwise read as a flash comment or note. Rows missing a side are dropped. The
result is validated (`parse_str`) and written to `~/decks/<name>.txt`; review it
and clean up any leftover HTML by hand. It works best on a plain two-field
export — rich notetypes, media, and tags don't carry over.

## The AI exam (`flash exam`)

Mechanical review *loads* a deck's material into memory; the **AI exam**
*verifies you understood it* and is what gates progression. The idea: drilling
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
due** rather than finished (so it does not yet unlock its dependents).

**Sitting the exam — interactive, in either frontend.** The exam is a guided,
one-question-at-a-time flow (Back/Next, then a per-question breakdown), the same
in the terminal and the browser:

- **Terminal**: `flash exam ownership.txt` (or `--questions 8`, `--strictness …`)
  opens the exam in the TUI. You also reach it by **picking an `exam due` deck**
  in the launcher (it starts the exam instead of an empty review), or from the
  **session-end summary** — when you drill a deck's last cards and it turns exam
  due, the summary offers "press `x` to take it".
- **Web** (`flash serve`): picking an `exam due` deck in the deck list launches
  the exam in the page; a finished session likewise offers it at the summary.

flash asks Claude to read the source (URLs via the **WebFetch** tool; local
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

Resetting a whole deck's progress (`flash reset <deck>`) also clears its mastered
state, so a re-drilled deck must pass the exam again (resetting only individual
cards with `--card`/`--cards` leaves the mastered state intact).

**Grading strictness** is a property of the *material*, so it's per deck. A
checklist-style topic (a procedure, exact syntax, a security drill) should fail
you for omitting a step; a conceptual topic shouldn't. Set it with a
`% strictness:` header directive (or `flash exam --strictness …`, or the
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
`strictness` (default `balanced`), and `extra` (guidance appended to question
generation). It reuses the `[ask]` command, permission mode and tool allowlist,
and needs the `claude` CLI installed and logged in.

## Traces (`flash trace`)

> **Experimental.** Traces are a new, evolving feature — the deck format and the
> flow may still change.

Cards drill *facts* (the nodes of what you know); a **trace** drills the
*connections between them* (the edges) by walking a **path** through a real
source and making you **predict each hop before it's revealed**. Where the
[AI exam](#the-ai-exam-flash-exam) verifies a *set* of independent answers, a
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
just the `% trace:` and `% source:`, then run `flash trace --build <deck>`:
Claude explores the source — with **read-only** `Read`/`Glob`/`Grep` and the
source root as its working directory (no write or shell access) — traces the
single load-bearing path, and writes the checkpoints (with their `% at:`
locators) back into the deck file. The result is cached and version-controlled
there, so review it — especially the locators — and edit freely; re-run
`--build` to regenerate. The build prompt encodes the chain rules below, so a
generated trace comes out a path, not a quiz.

**Snapshotting the source.** Because `% at: file:lines` reads the **live** source,
editing a traced file shifts every excerpt to the wrong lines. So when you create
a workspace by exploring a source ([`flash explore --into --build`](#exploring-a-source--flash-explore-experimental)),
its final step **freezes the cited excerpts** into the workspace's `assets/`
folder — one tiny snippet per checkpoint — and repoints each `% at:` (and the
trace's `% source:`) at them. The excerpts never drift and the workspace is
self-contained, **without copying whole source files**. A re-based snippet loses
its original line numbers, so when those matter the original `file:lines` is kept
in the card's note (`! from scheduler.rs:90-98`). It's automatic for explored
workspaces, not a command; a loose trace over a live `% source:` is left as-is.
See [docs/traces.md](docs/traces.md) for the rationale.

Because building is one-shot, correctness-critical, and **fails silently** when
the model is weak (you still get parseable checkpoints, just a loose chain you
then drill), the `[trace]` config section defaults it to a strong model
(`model = "opus"`) and high reasoning effort (`effort = "high"`) — slower than
the other AI features, but it runs once and is amortized over many reviews.
Override `model`, `effort` (`low`–`max`) and `timeout_secs` there. `--suggest`
shares these `[trace]` settings (it's also one-shot recon), but **`--grade` does
not**: judging a prediction is a light, interactive, per-hop call, so it runs at
the tutor tier — the `[ask]` model, effort and timeout — instead.

**Don't know what to trace?** `flash trace --suggest <source>` does a single
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

**Walking it** (`flash trace keypress-to-grade.txt`) goes hop by hop:

1. **Predict** — you type a guess before anything reveals (committing is the
   point).
2. **Reveal** — flash prints the real excerpt from the source, then the
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

**In the browser** — add **`--serve`** (`flash trace <deck> --serve`) to walk it
in the web frontend instead of the terminal, the same way `review`/`browse`
serve. The walk page shows the **path** as a rail you descend (its nodes color in
by Got / Partial / Missed) and reveals each checkpoint's real source in a
line-numbered excerpt; `--serve --grade` runs the live grading and the page waits
on Claude per hop. `--port`/`--lan` work as elsewhere. Progress saves to the same
store, so a walk started in the terminal continues in the browser.

`flash trace <deck> --map` prints the path (every prompt, its key points and
locator) without quizzing — a quick "just show me the route". The generic AI
exam refuses a trace (`flash exam <trace>` points you here): a trace's
verification *is* its predict-verify walk plus the compression, correctly scoped
to the path.

A trace deck degrades gracefully — even without `flash trace` it is a valid deck
of `explain` cards. See `examples/keypress-to-grade.txt` for a complete trace
over this repo's own source.

### Exploring a source — `flash explore` (experimental)

`--suggest` lists central *traces*; **`flash explore <source>`** goes one layer
up and prints an ordered **learning plan** toward a goal — the facts decks **and**
traces worth authoring, dependency-ordered:

```
flash explore .                                      # plan to understand the whole source
flash explore . --goal "how review scheduling works" # a narrow goal → a focused subset
```

Each item is tagged `[trace]` or `[deck]` — chosen by the shape of the knowledge
(a *path* you predict hop by hop becomes a trace; a *table of facts*, like a
config's knobs or a store's on-disk format, becomes a facts deck) — carries its
`% requires:` prerequisites (the list is dependency-ordered, foundations first),
and a `% source:` scope. The `--goal` scopes coverage: a broad goal covers every
subsystem, a narrow one collapses to just its slice (and traces it in more
detail). By default it's **read-only** — it prints the plan and you author the
items yourself (`flash trace --build` a trace, `flash deck` a facts deck).

With **`--into <dir>`** it materializes the plan into a ready-made **workspace**:

```
flash explore . --goal "how review scheduling works" --into ~/decks/scheduling/
```

That writes a `flash.toml` (the goal) and one stub file per item — a `% trace:`
deck for each trace (run `flash trace --build` on it) and a `% title:` facts deck
for each deck (author it or `flash deck`) — wired together with `% requires:`
so they unlock in dependency order, with each `% source:` pointing back at the
real source. (Refuses a non-empty folder unless `--force`.)

Add **`--build`** to go all the way: `flash explore … --into <dir> --build`
explores the source **once** and then reuses that same session to fill every
item — predict-verify checkpoints for the traces and fact cards for the decks —
so the workspace comes out review-ready in one command. Writing the whole set
from one understanding keeps the items coherent (each builds on its prerequisites
instead of repeating them), and it fills facts decks too. As a final step it
**freezes the source** into the workspace's `assets/` (see
[Snapshotting the source](#traces-flash-trace)), so the workspace is
self-contained and the trace locators never drift.

**Explore walk.** Before you even know what to trace, `flash explore --walk
<source>` builds a short **tour of the source's shape** and walks it like a trace:
you predict what kind of program it is (from the manifest), its domain nouns (from
the module list), how it's driven (the entry point), its spine (the central file),
and finally the first paths worth tracing — each hop revealing the real lines.
It's written to a file (`-o`, default `explore.txt`), so `flash trace explore.txt`
re-walks it.

## Configuration

Key bindings can be changed in `~/.config/flash/config.toml`. Create the
file with `flash config --init`, inspect the active bindings with
`flash config`. Every action takes a list of keys; the first one is
shown in the footer. For example, to grade flip-mode cards with j/k/l:

```toml
[keys]
again = ["j"]
good = ["k"]
easy = ["l"]
```

Keys are written as a single character (`"j"`), a special key name
(`"space"`, `"enter"`, `"tab"`, `"esc"`, `"backspace"`), or either with a
`ctrl-` prefix (`"ctrl-s"`). Rebindable actions: `again`, `good`, `easy`,
`reveal`, `hint`, `submit`, `skip`, `remove` (mark the card for deletion from
its deck file, default `ctrl-x`), `continue`, `restart` (start a new session
from the summary screen, default `r`), `quit`. While you are typing
an answer (typing and fuzzy mode), plain character bindings are ignored so
they cannot shadow text input — use `ctrl-`/special keys for `hint`, `skip`
and `quit`. A different config file can be passed with `--config <path>`.

The browser (`flash browse`) has its own bindings in a `[browse]` section:

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
- Progress is stored at `~/.local/share/flash/progress.json` and config at
  `~/.config/flash/config.toml` (created on first use).
- `flash reset <deck>...` clears stored progress so cards become "new" again —
  for whole decks, a single card (`--card <id-or-front-text>`), or the entire
  store (`--all`). Run it with no decks to pick from the deck list, or add
  `--cards` to pick individual cards from a checkbox list. It confirms first
  unless you pass `-y`/`--yes`.

## Desktop integration

To launch flash from the desktop menu (Cinnamon, GNOME, KDE, ...), run:

```sh
assets/install-desktop.sh
```

It installs the icon (`assets/flash.svg`, rendered to the standard
PNG sizes), a launcher that reviews everything due in `~/decks` in a
terminal, and a `.desktop` entry under `~/.local/share`. Re-run it after
editing the SVG. The launcher prefers an installed `flash` (`cargo install
--path .`) and falls back to the project build.

## Development

```sh
cargo test       # unit tests
cargo clippy     # lints
```
