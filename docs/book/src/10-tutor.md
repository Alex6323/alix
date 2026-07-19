# 10 · The Tutor

This is where the AI layer begins. Everything so far (drilling, scheduling,
workspaces) runs entirely offline. From here on `alix` shells out to the
configured model CLI, and the first place it does is the most useful: a tutor
on any card.

(One reminder: every AI feature shells out to the configured model CLI, so it
needs the CLI installed and logged in. See [chapter 2](02-getting-started.md).
The flashcard core never calls it.)

## Asking about a card

On any post-answer screen (a revealed flip card, the feedback after a typed
answer, an answered choice) an **Ask** button (or the `?` key) opens a chat
panel without leaving the session: type a question, **Send**, **Make this a note**,
**Close**. `alix` hands the tutor the card (its front, answer, note, and deck
name) as context, and you can ask "why is that the answer?", "what's a
simpler way to see this?", or anything else, and follow up. The server runs
the model CLI on a background thread and the page polls for the reply, so
the single-threaded server never blocks and the session stays responsive
while it works.

One conversation spans the **whole review run**. For Claude, `alix` uses
`--session-id` for the first question and `--resume` for each follow-up, so the
model remembers earlier cards and questions efficiently. Other backends re-inline
the accumulated Q&A transcript into each prompt, so the context carries over, at
the cost of a growing prompt rather than a resumed session. Either way you can ask
how the current card relates to one from ten minutes ago, and the tutor knows.

Ask is available wherever you serve, including over `--lan`, but the request
runs the model CLI on the host machine, so, like `--lan` in general, only
enable it on a network you trust.

## Saving what you learn: `Ctrl-N`

When an exchange clears something up, press `Ctrl-N`: the tutor condenses the
conversation into at most three short note lines and appends them to the card in
its deck file. Notes aren't part of the card's identity, so its progress is
untouched: you just keep the insight. (In the web panel, **Make this a note**
does the same.)

## Reference links: `link:`

A deck can point the tutor at background reading with a `link:` list in its
frontmatter:

```
---
link:
  - https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
  - https://tokio.rs/tokio/tutorial
---
```

These are handed to the tutor with your first question as material to consult when
useful: fetched once and remembered for the rest of the run. They're tutor-only:
unlike `source:` (the exam's ground truth, covered next chapter), a `link:`
never becomes exam material. And like every directive, they don't affect a
card's identity.

## Grounding a frozen card: `origin:`

A workspace frozen by `alix generate`'s workspace build shows you *snapshots* of its
source (the `assets/` copies), not the live files. When you ask about one of those
cards, the tutor reads the **live source** the snapshot came from (recorded in the
deck's `origin:`) for surrounding context, while keeping the frozen excerpt you
see as the anchor, so it never reasons about a drifted copy. If the live source is
gone, it says so plainly instead of guessing. In review, that reply comes back
immediately, with no model call. The same grounding applies when you ask during a
trace walk.

## How it's sandboxed

Because the CLI runs headless, it can't show interactive permission prompts: an
unanswerable prompt would just hang the call. So `alix` runs it locked down with a
locked permission mode plus an **exclusive tool allowlist** (`WebFetch`, `WebSearch`
by default). The listed tools work without prompting; every other tool is silently
denied. That means a malicious page behind a deck link can't make the tutor run
shell commands or touch your files. Both the permission mode and the allowlist live
in the `[ask]` section of the config, along with the command, a `--model` override,
and the timeout.

## Make this a card

During an Ask exchange, if the tutor's reply answers a question about a concept
you'd like to drill, click **Make this a card**.
The tutor distills the conversation into a draft front/back for you to edit.
Once you're satisfied, click **Add** to land it as a new card on the current deck.

The card is **virtual** until you promote it to the deck file; it lives in the
review progress store but doesn't yet appear in the `.md` deck. You drill it like
any other card, building up history. When you're ready to make it permanent, the
**Promote** button (or `p` key, visible on the card's review screen) writes it to
the deck file at the end of the session, so future runs see it.

This is an **adult-review feature only**; it's not available in the kids interface.
If the tutor's draft can't be parsed as a valid front/back pair, `alix` reports the
error plainly rather than inventing a card, so you can ask for a clearer format.
