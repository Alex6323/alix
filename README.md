# `alix`

[![CI](https://github.com/Alex6323/alix/actions/workflows/ci.yml/badge.svg)](https://github.com/Alex6323/alix/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/Alex6323/alix/branch/main/graph/badge.svg)](https://codecov.io/gh/Alex6323/alix)
[![crates.io](https://img.shields.io/crates/v/alix)](https://crates.io/crates/alix)
[![docs.rs](https://img.shields.io/docsrs/alix)](https://docs.rs/alix)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/alix)](https://crates.io/crates/alix)

> **Early WIP.** The deck format and the progress store still change between
> commits, with no migration path, so expect to lose progress. Fine for
> tinkering; not yet for study you care about.

`alix` is a plain-text spaced-repetition learning tool that turns your own
material into decks you study, then has you prove you actually grasped it, not
just memorized it. It uses AI only where that helps: generating decks from a
source you bring (an article, a paper, a codebase), a tutor on any card, and an
exam that grades your written answers against that source. Reviewing runs fully
offline; you work through several **degrees of depth**, from recognizing an
answer to reconstructing it from memory, and only passing that exam marks a deck
as understood.

![A review card in the alix web app: a multiple-choice question over a plain-text deck](https://alix.study/img/shot-3-modes.png)

**Manual → [alix.study/book](https://alix.study/book/)  ·  Slides → [alix.study/slides.html](https://alix.study/slides.html)  ·  Site → [alix.study](https://alix.study)**

**Feedback → bugs via the [issue form](https://github.com/Alex6323/alix/issues/new/choose), ideas at [contact@alix.study](mailto:contact@alix.study)**

## Install

```sh
cargo install alix
# or a prebuilt binary:
curl -sSf https://alix.study/install.sh | sh
```

The core needs nothing else. The AI features (`generate`, `deck augment`, the
exam, and the tutor) shell out to a model CLI you
install and log in to yourself: [Claude Code](https://www.anthropic.com/claude-code)
by default, or Gemini, Codex, or Copilot via `[ask] backend`. Each backend gets
read-only tools only, and `alix` stores no API keys. See
[Getting started](docs/book/src/02-getting-started.md) and
[Configuration](docs/book/src/16-configuration.md).

## Quick start

```sh
# make decks and workspaces from your own material
alix generate <url-or-file>            # -> a deck
alix generate <repo> --goal "how X works" --workspace ~/decks/topic   # -> a workspace

# then study, always in the web app (pick a deck or workspace, then a depth)
alix
```

## A deck is a text file

```text
% Comments start with %. A card's front sits at column 0.

# What does a String own?
    A Vec<u8>, its bytes on the heap.
    ! Capacity can exceed its length.

# Fill in the blank
    % reveal: cloze
    let {{mut}} x = 0;
```

The front is the `#` line; the indented lines below are the answer. An `!` line
is a note shown after you answer, and a `%` directive on a card tunes it (here
`reveal: cloze` turns `{{mut}}` into a fill-in-the-blank). Full format and every
directive: [the deck format](docs/book/src/03-the-deck-format.md),
[directives](docs/book/src/07-directives.md).

## Commands

| Command | What it does |
|---|---|
| `alix` | Open the web app to pick a deck or workspace |
| `alix generate <source>` | Draft a deck, a trace (`--trace`), or a whole workspace |
| `alix deck augment <deck> --target …` | Add distractors, notes, or key points |
| `alix workspace init <dir>` | Scaffold an empty workspace to grow into |
| `alix workspace deadline <dir> [<date>\|clear]` | Show, set, or clear a workspace's "ready by" date |
| `alix <dir>` | Serve a folder or workspace as its own scoped instance |
| `alix deck import <file.tsv>` | Import an Anki TSV export (`--workspace` places it) |
| `alix stats <path>` · `alix list <path>` | Progress · schedules (deck, folder, or workspace) |
| `alix reset <path>` | Clear stored progress (deck, folder, or workspace) |
| `alix share <path>` · `alix receive <code>` | Send/get decks over [magic-wormhole](https://magic-wormhole.readthedocs.io) |
| `alix config` | Show the config (`--init` writes a starter file) |
| `alix doctor [path]` | Check the setup, or lint a deck; `--backends` probes the AI |

Every flag and option: [command reference](docs/book/src/17-command-reference.md).

## What's inside

Each links to its chapter in the manual:

- **Reveal methods and session depths** (Recognize / Recall / Reconstruct),
  picked per session. → [Reveal & session depths](docs/book/src/04-review-modes.md)
- **FSRS scheduling**, with card retirement and per-deck completion states.
  → [Scheduling](docs/book/src/05-scheduling.md)
- **A tutor** you can ask "why is that the answer?" without leaving review.
  → [The tutor](docs/book/src/10-tutor.md)
- **Deck generation and augmentation** from a URL or a local source (distractors,
  notes, key points). → [Generating decks](docs/book/src/11-generating-decks.md)
- **An AI exam** that grades open questions against the source and gates unlocks.
  → [The AI exam](docs/book/src/12-the-ai-exam.md)
- **Traces**: predict-and-verify walks along one path through real source.
  → [Trace decks](docs/book/src/13-trace-decks.md)
- **Workspaces and `explore`**: group decks with shared settings, or turn a
  source (including a repo) into an ordered workspace of decks and traces.
  → [Workspaces](docs/book/src/08-workspaces.md), [Explore](docs/book/src/14-explore.md)
- **A local web app** for review and the exam, LAN-shareable, no accounts or
  database. → [The web app](docs/book/src/15-the-web-app.md)
- **Self-contained, portable decks.** Your decks and their progress live in one
  folder; drop it in a cloud drive (Dropbox, iCloud, Syncthing) and it follows
  you across devices, no accounts. → [Configuration](docs/book/src/16-configuration.md)
- **A personal deadline on a workspace**, with scheduling that leans toward it
  and a readiness readout. → [Workspaces](docs/book/src/08-workspaces.md)
- **A paired phone** can borrow your desktop's AI for the tutor and the exam,
  over your LAN. → [Pairing a device](docs/book/src/19-pairing.md)

## Development

```sh
make check   # clippy + tests, the gate before a change is done
make fmt     # format (nightly rustfmt, not plain cargo fmt)
make web     # run the web frontend
```

Building a client? The web JSON API is a documented contract:
[docs/API.md](docs/API.md). See [CONTRIBUTING.md](CONTRIBUTING.md).

## Support

alix is free and open source. Telling someone who studies is the best
support; if you want to support the project financially, you can do so
[here](https://github.com/sponsors/Alex6323).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

Bundled third-party assets (fonts) are separately licensed; see
[NOTICE](NOTICE).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
