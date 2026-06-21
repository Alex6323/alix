# 11 · Generating decks — `flash deck`

Authoring cards by hand is the slow part of any flashcard habit. `flash deck`
removes it: point it at a source and Claude drafts a deck of fact cards for you.

```sh
flash deck https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
flash deck src/scheduler.rs            # a local file (or a whole directory)
```

The source is a **web page URL** or a **local file/directory** — the deck-side
mirror of `flash trace` (a later chapter), which builds *traces* from the same
kinds of source.

## What you get

Claude reads the source and returns a deck spread across **four layers of
understanding** — facts → concepts → application → connections — using cloze
(`#?`) cards for terminology. The prompt has it draft, then re-read the whole set
and merge or drop cards that test the same fact, so the deck doesn't repeat
itself. flash validates the text it gets back (it only ever accepts cards, never a
write or shell command) and writes it to `~/decks/<slug>.txt`.

How the source is recorded depends on its kind, and it matters later:

- A **web page** is read with the `WebFetch` tool, and the deck opens with a
  `% link:` line back to it — so the [ask-Claude tutor](10-ask-claude.md) can
  consult the page on your cards.
- A **local source** is explored read-only with `Read`/`Glob`/`Grep`, and the
  deck opens with a `% source:` line — so the **AI exam** can later grade your
  understanding against that same source (next chapter).

## Useful flags

```sh
flash deck <source> -o ownership    # choose the output file name
flash deck <source> --cards 15      # cap the number of cards
flash deck <source> --review        # a 2nd pass that dedups and tightens
flash deck <source> --print         # print to stdout instead of writing a file
```

`--review` runs a **second** Claude call that takes the draft and returns a
deduplicated, tightened version. It costs an extra call, but it's worth it when
the source is repetitive. The prompt and limits — `model`, `timeout_secs`
(default 300), `max_cards` (default 30), and an `extra` instruction field — live
in the `[generate]` section of the config.

## Generate, then own it

A generated deck is just a plain-text deck like any other: read it, edit it, cut
the weak cards, add your own. Treat the output as a strong first draft, not
gospel — the point is to skip the blank page, not to outsource judgment. That's
the same division the whole tool runs on (see [how flash was made](how-flash-was-made.md)).
