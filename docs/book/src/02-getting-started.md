# 2 · Getting started

## Install

`alix` is a single Rust binary, built from source — you need a Rust toolchain
(install [`rustup`](https://rustup.rs) if you don't have one):

```sh
git clone <repo-url> alix
cd alix
make install        # or: cargo install --path .
```

That puts `alix` on your `PATH`. Check it:

```sh
alix --help
```

The flashcard **core** — reviewing, scheduling, every answer mode, browse, and
the web app — runs with nothing else installed, no accounts, no
network. The **AI features** (deck generation, the exam, traces, workspace
generation, and the in-session tutor) shell out to a supported model CLI — [Claude Code](https://www.anthropic.com/claude-code)
by default; the Gemini, Codex, and Copilot CLIs are also supported. Install at
least one and authenticate with it. See
[chapter 16](16-configuration.md#backends) for how to switch backends. You can use the
entire core without ever touching the AI layer.

## Your first deck

A deck is a plain `.txt` file. A card is a `#` line — the question — with its
answer on the indented lines beneath it:

```
# What does SRS stand for?
    Spaced repetition system.
    ! It schedules each card just before you'd forget it.

# Which scheduler does alix use?
    FSRS — it predicts when you're about to forget each card.
```

Save it as `srs.txt` in your decks directory (`~/decks` by default). Indentation
is optional (lines are trimmed) — it's just for
readability. A line starting with `!` is a **note**, shown after you answer.

## Review it

```sh
alix
```

`alix` opens the web app (printing its URL) — pick `srs.txt` there and **Learn**
it. The question shows in the browser; you
recall the answer, press a key to reveal it, then
grade yourself — **failed** (you missed it), **partly** (got the gist but
stumbled), or **passed**. Your grade moves the card along its schedule, so cards
you know come back rarely and cards you miss come back soon. That self-graded reveal is **flip mode**, the default;
later chapters cover the modes that make you *type* the answer, pick from
choices, or reveal it line by line.

When nothing is due, there's nothing to review — come back when cards mature.

## The deck picker

That page `alix` opens is the **picker**, over your
decks directory (`~/decks` by default; change it with `decks_dir` in the
config). It groups your decks into Workspaces, Recent, and Folders and is
driven by Vim-style keys (`j`/`k` to move, `Enter` to open, `/` to filter by
name). Every review starts here — there's no direct deck launch. This is what
the desktop launcher opens. Focus a deck
and press
**Browse** to read through its cards with no grading or scheduling.

## The everyday commands

```sh
alix stats srs.txt     # a progress overview
alix list srs.txt      # every card with its per-depth schedule and due time
alix doctor srs.txt    # lint the deck (syntax errors, duplicate cards)
alix reset srs.txt     # clear stored progress (also --card / --all)
```

A session is one deck — review them one at a time. From here the book goes deep:
the next chapter is the [deck format](03-the-deck-format.md) in full, then
[reveal & session depths](04-review-modes.md) and scheduling.
