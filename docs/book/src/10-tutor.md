# 10 · The Tutor

This is where the AI layer begins. Everything so far — drilling, scheduling,
workspaces — runs entirely offline. From here on `alix` shells out to the
configured model CLI, and the first place it does is the most useful: a tutor
on any card.

(One reminder: every AI feature shells out to the configured model CLI, so it
needs the CLI installed and logged in — see [chapter 2](02-getting-started.md).
The flashcard core never calls it.)

## Asking about a card

On any post-answer screen — a revealed flip card, the feedback after a typed
answer, an answered choice — an **Ask** button (or the `?` key) opens a chat
panel without leaving the session: type a question, **Send**, **Save note**,
**Close**. `alix` hands the tutor the card (its front, answer, note, and deck
name) as context, and you can ask "why is that the answer?", "what's a
simpler way to see this?", or anything else, and follow up. The server runs
the model CLI on a background thread and the page polls for the reply, so
the single-threaded server never blocks and the session stays responsive
while it works.

One conversation spans the **whole review run**. For Claude, `alix` uses
`--session-id` for the first question and `--resume` for each follow-up, so the
model remembers earlier cards and questions efficiently. Other backends re-inline
the accumulated Q&A transcript into each prompt — so the context carries over, at
the cost of a growing prompt rather than a resumed session. Either way you can ask
how the current card relates to one from ten minutes ago, and the tutor knows.

Ask is available wherever you serve, including over `--lan` — but the request
runs the model CLI on the host machine, so, like `--lan` in general, only
enable it on a network you trust.

## Saving what you learn — `Ctrl-N`

When an exchange clears something up, press `Ctrl-N`: the tutor condenses the
conversation into at most three short note lines and appends them to the card in
its deck file. Notes aren't hashed, so the card's progress is untouched — you just
keep the insight. (In the web panel, **Save note** does the same.)

## Reference links — `% link:`

A deck can point the tutor at background reading with `% link:` comment lines:

```
% link: https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
% link: https://tokio.rs/tokio/tutorial
```

These are handed to the tutor with your first question as material to consult when
useful — fetched once and remembered for the rest of the run. They're tutor-only:
unlike `% source:` (the exam's ground truth, covered next chapter), a `% link:`
never becomes exam material. And like every directive, they don't affect card
hashes.

## Grounding a frozen card — `% origin:`

A workspace frozen by `alix generate`'s workspace build shows you *snapshots* of its
source (the `assets/` copies), not the live files. When you ask about one of those
cards, the tutor reads the **live source** the snapshot came from — recorded in the
deck's `% origin:` — for surrounding context, while keeping the frozen excerpt you
see as the anchor, so it never reasons about a drifted copy. If the live source is
gone, it says so plainly instead of guessing — in review, that reply comes back
immediately, with no model call. The same grounding applies when you ask during a
trace walk.

## How it's sandboxed

Because the CLI runs headless, it can't show interactive permission prompts — an
unanswerable prompt would just hang the call. So `alix` runs it locked down with a
locked permission mode plus an **exclusive tool allowlist** (`WebFetch`, `WebSearch`
by default). The listed tools work without prompting; every other tool is silently
denied. That means a malicious page behind a deck link can't make the tutor run
shell commands or touch your files. Both the permission mode and the allowlist live
in the `[ask]` section of the config, along with the command, a `--model` override,
and the timeout.
