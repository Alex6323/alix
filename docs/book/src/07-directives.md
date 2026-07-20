# 7 · Directives reference

Every card marker and deck/card key in one place. **Scope** is where each may
appear: a *deck* key is a line in the frontmatter (the `---`-fenced YAML block
at the top of the file), a *card* key is a `<!-- key: value -->` comment after
a card's front, and *deck · card* keys work either way, with the card one
taking precedence. Each links to the chapter that explains it in full.

| Token | Scope | What it does |
| --- | --- | --- |
| `##` front | card | Starts a card at column 0; the lines below are the answer. [→ ch 3](03-the-deck-format.md) |
| `>` line | card | A note, shown after you answer. [→ ch 3](03-the-deck-format.md) |
| `<!-- -->` | anywhere | A comment with no recognized key: ignored. |
| `id` | card | The card's identity token: minted and written by alix the first time it sees the card, never hand-authored. [→ ch 3](03-the-deck-format.md) |
| `reveal` | deck · card | [How the answer is uncovered](04-review-modes.md): flip (default) or line. |
| `order` | deck | Card order: `scheduled` (default) or `sequential`. [→ ch 5](05-scheduling.md) |
| `input` | deck · card | `draw`: answer on a canvas instead of typing. [→ ch 4](04-review-modes.md) |
| `direction` | deck · card | [Review direction](06-cloze-direction-images.md): forward, reverse, both. |
| `requires` | deck | [Prerequisite deck](09-dependencies.md) that gates unlocks (repeatable). |
| `link` | deck | [tutor reference](10-tutor.md) URL, tutor-only (repeatable). |
| `source` | deck | [Exam ground truth](12-the-ai-exam.md) (URL/file, repeatable); also a [trace](13-trace-decks.md)'s path origin, and a tutor reference. |
| `origin` | deck · card | Live source root a [frozen deck](14-explore.md)'s snapshots came from (set in a workspace's `alix.toml`); enables [tutor](10-tutor.md) grounding and [`alix doctor`](17-command-reference.md) drift detection: `source` itself points at the frozen `assets/`. |
| `trace` | deck | What a [trace](13-trace-decks.md) walks; its presence makes the deck a trace. |
| `at` | card | A locator into the `source` (`file:lines`): a [trace checkpoint's](13-trace-decks.md) reveal target, or a [fact card's source citation](06-cloze-direction-images.md#source-citations) shown on reveal. |
| `given` | card | A [trace checkpoint's](13-trace-decks.md) off-screen symbol, as `name — meaning` (repeatable). |

Media (images, and later audio/video) isn't a directive: write a standard
Markdown `![alt](src)` where you want one to appear, and its position decides
the side. See [Image cards](06-cloze-direction-images.md).

## `link` vs `source`

Two that look similar but aren't. Both point at material a deck is about, but
`source` is the **exam's ground truth**: questions are generated from it and
answers graded against it, and a URL source doubles as a tutor reference.
`link` is **only** a tutor reference and never becomes exam material; use it
for supplementary reading the exam should ignore. The implication runs one
way: a `source` URL is offered to the tutor, but a `link` is never promoted to
a source.

## Precedence

Where a directive can come from several places, the more specific wins:

> card `<!-- -->` > deck frontmatter > workspace `[defaults]` > built-in default

So a card's `reveal` directive overrides the deck's, which overrides a
workspace's `[defaults]`, which overrides `alix`'s default (`flip`).

The session depth (Recognize/Recall/Reconstruct) is **not** in this chain
either: it isn't config or a deck directive at all. It's chosen per session
(the picker's Learn ▾ menu), the same way for every deck (see
[Reveal & session depths](04-review-modes.md)).
