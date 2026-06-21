# 2 · Getting started

## Install

flash is a single Rust binary, built from source — you need a Rust toolchain
(install [`rustup`](https://rustup.rs) if you don't have one):

```sh
git clone <repo-url> flash
cd flash
make install        # or: cargo install --path .
```

That puts `flash` on your `PATH`. Check it:

```sh
flash --help
```

The flashcard **core** — reviewing, scheduling, every answer mode, browse, the
terminal UI and the web app — runs with nothing else installed, no accounts, no
network. The **AI features** (deck generation, the exam, traces, explore, and the
ask-Claude tutor) shell out to the [Claude Code](https://www.anthropic.com/claude-code)
CLI, so for those you need it installed and logged in — run `claude` once to
authenticate (it needs a Claude subscription or API access). You can use the
entire core without ever touching the AI layer.

## Your first deck

A deck is a plain `.txt` file. A card is a `#` line — the question — with its
answer on the indented lines beneath it:

```
# What does SRS stand for?
    Spaced repetition system.
    ! It schedules each card just before you'd forget it.

# Which scheduler does flash use by default?
    Leitner — a six-stage box with growing cooldowns.
```

Save it as `srs.txt`. Indentation is optional (lines are trimmed) — it's just for
readability. A line starting with `!` is a **note**, shown after you answer.

## Review it

```sh
flash srs.txt
```

flash shows the question; you recall the answer, press a key to reveal it, then
grade yourself — **again** (you missed it), **good**, or **easy**. Your grade
moves the card along its schedule, so cards you know come back rarely and cards
you miss come back soon. That self-graded reveal is **flip mode**, the default;
later chapters cover the modes that make you *type* the answer, pick from
choices, or reveal it line by line.

When nothing is due, flash says so and exits — come back when cards mature, or
pass `--cram` to review everything regardless of cooldowns.

## The deck picker

Run `flash` with no arguments to open the **picker** over your decks directory
(`~/decks` by default; change it with `decks_dir` in the config). It groups your
decks into Workspaces, Recent, and Folders and is driven by Vim-style keys
(`j`/`k` to move, `Enter` to open, `/` to filter by name). This is what the
desktop launcher opens.

## The everyday commands

```sh
flash browse srs.txt    # read through the cards — no grading, no scheduling
flash stats srs.txt     # a progress overview
flash list srs.txt      # every card with its stage and due time
flash check srs.txt     # lint the deck (syntax errors, duplicate cards)
flash reset srs.txt     # clear stored progress (also --card / --all)
```

Give several decks at once and their due cards merge into one session. From here
the book goes deep: the next chapter is the [deck format](03-the-deck-format.md)
in full, then the answer modes and scheduling.
