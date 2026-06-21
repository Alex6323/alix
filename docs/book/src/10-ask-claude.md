# 10 · Ask-Claude — the tutor

This is where the AI layer begins. Everything so far — drilling, scheduling,
workspaces — runs entirely offline. From here on flash can call Claude, and the
first place it does is the most useful: a tutor on any card.

(One reminder: every AI feature shells out to the Claude Code CLI, so it needs the
CLI installed and logged in — see [chapter 2](02-getting-started.md). The
flashcard core never calls it.)

## Asking about a card

On any post-answer screen — a revealed flip card, the feedback after a typed
answer, an answered choice — press `?`. flash hands Claude the card (its front,
answer, note, and deck name) as context, and you can ask "why is that the
answer?", "what's a simpler way to see this?", or anything else, and follow up.
While Claude thinks the session stays responsive; `Esc` drops you back exactly
where you were.

One conversation spans the **whole review run** (flash uses `--session-id` for the
first question and `--resume` after), so Claude remembers the earlier cards and
questions — you can ask how the current card relates to one from ten minutes ago,
and it knows.

## In the browser

It works in the web frontend (`--serve`) too: an **Ask** button (or the `?` key)
on an answered card opens a chat panel — type a question, **Send**, **Save note**,
**Close**. The server runs `claude -p` on a background thread and the page polls
for the reply, so the single-threaded server never blocks. Ask is available
wherever you serve, including over `--lan` — but the request runs `claude` on the
host machine, so, like `--lan` in general, only enable it on a network you trust.

## Saving what you learn — `Ctrl-N`

When an exchange clears something up, press `Ctrl-N`: Claude condenses the
conversation into at most three short note lines and appends them to the card in
its deck file. Notes aren't hashed, so the card's progress is untouched — you just
keep the insight. (In the web panel, **Save note** does the same.)

## Reference links — `% link:`

A deck can point the tutor at background reading with `% link:` comment lines:

```
% link: https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
% link: https://tokio.rs/tokio/tutorial
```

These are handed to Claude with your first question as material to consult when
useful — fetched once and remembered for the rest of the run. They're tutor-only:
unlike `% source:` (the exam's ground truth, covered next chapter), a `% link:`
never becomes exam material. And like every directive, they don't affect card
hashes.

## How it's sandboxed

Because the CLI runs headless (`claude -p`), it can't show interactive permission
prompts — an unanswerable prompt would just hang the call. So flash runs it locked
down: `--permission-mode dontAsk` plus an **exclusive tool allowlist** (`WebFetch`,
`WebSearch` by default). The listed tools work without prompting; every other tool
is silently denied. That means a malicious page behind a deck link can't make the
tutor run shell commands or touch your files. Both the permission mode and the
allowlist live in the `[ask]` section of the config, along with the command, a
`--model` override, and the timeout.
