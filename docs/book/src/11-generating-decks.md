# 11 · Generating decks: `alix generate deck`

Authoring cards by hand is the slow part of any flashcard habit. `alix generate
deck` removes it: point it at a source and the model drafts a deck of fact cards
for you.

```sh
alix generate deck https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
alix generate deck src/scheduler.rs   # a local file
```

The source is a **web page URL**, a **local file**, or a **directory** taken
whole. `alix generate` has exactly two subcommands, and each one names what you
get: `deck` always writes one deck, `workspace`
([chapter 14](14-explore.md)) always explores a directory for a learning plan
and builds a workspace of decks and traces. Adding `--trace` to `generate deck`
makes that one deck a *trace* ([chapter 13](13-trace-decks.md)).

While the model works, `alix` prints short progress updates to stderr, such as
source fetching, source reading, and drafting. Partial generated cards stay
hidden until the complete result has passed validation. Deck drafting has a
one-hour absolute limit. With a structured-event backend, every generation
path also has a five-minute inactivity limit that resets on each real agent
event. For a backend without structured events, that five-minute value becomes
a nonrenewing absolute fallback because Alix cannot distinguish silence from
work. Set `idle_timeout_secs = 0` to disable either use and leave only the
one-hour limit. Configure the limits under `[generate]`. Trace and workspace
planning calls keep their absolute limit under `[trace]`.

Also available from the web UI's ☰ menu (**Add deck…**), URL sources only.
See [the web app](15-the-web-app.md).

## What you get

The model reads the source and returns a deck spread across **four layers of
understanding** (facts → concepts → application → connections) using
cloze cards for terminology. The prompt has it draft, then re-read the whole set
and merge or drop cards that test the same fact, so the deck doesn't repeat
itself. `alix` validates the text it gets back (it only ever accepts cards, never a
write or shell command) and writes it to `~/decks/<slug>.md`.

How the source is recorded depends on its kind, and it matters later:

- A **web page** is read with the `WebFetch` tool, and the deck opens with a
  `link:` line back to it, so the [tutor](10-tutor.md) can
  consult the page on your cards.
- A **local source** is explored read-only with `Read`/`Glob`/`Grep`, and the
  deck opens with a `source:` line, so the **AI exam** can later grade your
  understanding against that same source (next chapter). Each fact that maps to
  specific lines also gets a fingerprinted
  [`<!-- at: -->` citation](06-cloze-direction-images.md#source-citations), so
  you can flip the card to its source on reveal without trusting a shifted
  numeric range.

## Useful flags

```sh
alix generate deck <source> -o ownership         # choose the output file name
alix generate deck <source> --cards 15           # aim for at most 15 cards (a soft ceiling)
alix generate deck <source> --review             # a 2nd pass that dedups and tightens
alix generate deck <source> --print              # print to stdout instead of writing a file
alix generate deck <source> --into ~/decks/rust/ # write it under that workspace's decks/
alix generate deck <source> --goal "pass the citizenship test"
alix generate deck <source> --language German --audience "new voters"
alix generate deck <source> --card-style authored-choices
```

`--goal` controls what the learner should understand for every new deck or
workspace, including a single deck generated from a URL or file. `--language`
sets the language of fronts, answers, choices, and notes. `--audience` steers
vocabulary, assumed knowledge, examples, and difficulty.

`--card-style` accepts `mixed` (the default), `plain`, `cloze`, or
`authored-choices`. Authored choices use the deck's GitHub task-list format,
with one checked correct answer and unchecked distractors. Alix parses the
result and refuses a generated facts deck containing a card of the wrong shape,
so a model cannot silently turn an authored-choice request into ordinary
recall cards. In a generated workspace the style applies to every `[deck]`
item; `[trace]` items keep their predict-and-verify checkpoint shape. Goal,
language, and audience apply to both.

`--review` runs a **second** model call that takes the draft and returns a
deduplicated, tightened version while preserving the requested language,
audience, and card style. It costs an extra call, but it's worth it when the
source is repetitive. The prompt and defaults (`model`, `timeout_secs`
(default 3600), `idle_timeout_secs` (default 300; structured inactivity or an
unstructured absolute fallback, and `0` disables),
`max_cards` (default 100, a soft ceiling: an overshoot is kept and warned about), `language`, `audience`, `card_style`, and an `extra`
instruction field) live in the `[generate]` section of the config.

## Generate, then own it

A generated deck is just a plain-text deck like any other: read it, edit it, cut
the weak cards, add your own. Treat the output as a strong first draft, not
gospel. The point is to skip the blank page, not to outsource judgment. That's
the same division the whole tool runs on (see [how `alix` was made](how-alix-was-made.md)).
