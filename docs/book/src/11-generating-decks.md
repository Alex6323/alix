# 11 · Generating decks — `alix deck generate`

Authoring cards by hand is the slow part of any flashcard habit. `alix deck
generate` removes it: point it at a source and the model drafts a deck of fact cards
for you. (`alix deck` is now a command group — `generate` plus `augment`.)

```sh
alix deck generate https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
alix deck generate src/scheduler.rs   # a local file (or a whole directory)
```

The source is a **web page URL** or a **local file/directory** — the deck-side
mirror of `alix trace` (a later chapter), which builds *traces* from the same
kinds of source.

## What you get

The model reads the source and returns a deck spread across **four layers of
understanding** — facts → concepts → application → connections — using
`% reveal: cloze` cards for terminology. The prompt has it draft, then re-read the whole set
and merge or drop cards that test the same fact, so the deck doesn't repeat
itself. `alix` validates the text it gets back (it only ever accepts cards, never a
write or shell command) and writes it to `~/decks/<slug>.txt`.

How the source is recorded depends on its kind, and it matters later:

- A **web page** is read with the `WebFetch` tool, and the deck opens with a
  `% link:` line back to it — so the [tutor](10-tutor.md) can
  consult the page on your cards.
- A **local source** is explored read-only with `Read`/`Glob`/`Grep`, and the
  deck opens with a `% source:` line — so the **AI exam** can later grade your
  understanding against that same source (next chapter). Each fact that maps to
  specific lines also gets a [`% at:` citation](06-cloze-direction-images.md#source-citations),
  so you can flip the card to its source on reveal.

## Useful flags

```sh
alix deck generate <source> -o ownership    # choose the output file name
alix deck generate <source> --cards 15      # cap the number of cards
alix deck generate <source> --review        # a 2nd pass that dedups and tightens
alix deck generate <source> --print         # print to stdout instead of writing a file
```

`--review` runs a **second** model call that takes the draft and returns a
deduplicated, tightened version. It costs an extra call, but it's worth it when
the source is repetitive. The prompt and limits — `model`, `timeout_secs`
(default 300), `max_cards` (default 30), and an `extra` instruction field — live
in the `[generate]` section of the config.

## Generate, then own it

A generated deck is just a plain-text deck like any other: read it, edit it, cut
the weak cards, add your own. Treat the output as a strong first draft, not
gospel — the point is to skip the blank page, not to outsource judgment. That's
the same division the whole tool runs on (see [how `alix` was made](how-alix-was-made.md)).
