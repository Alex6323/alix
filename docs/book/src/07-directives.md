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
| `format-version` | deck | The deck **format's** version, not the deck's own. Written by `alix deck init` above `id`, stays `1`, and any other number is refused rather than guessed at. Mandatory once a deck has an `id`. [→ ch 3](03-the-deck-format.md) |
| `id` | deck | The frontmatter deck ID (`deck-<token>`) marks an initialized deck and authorizes maintenance of missing card IDs. Its `deck-` prefix is what tells alix's decks apart. [→ ch 3](03-the-deck-format.md) |
| `id` | card | The HTML-comment card ID (`card-<token>`) anchors review history. It is minted by `alix deck init` or a deck-creation workflow and maintained by alix, never hand-authored. [→ ch 3](03-the-deck-format.md) |
| `reveal` | deck · card | [How the answer is uncovered](04-review-modes.md): flip (default) or line. Cloze is triggered by `\blank{...}` markers, never by a `reveal:` value. |
| `order` | deck | Card order: `scheduled` (default) or `sequential`. [→ ch 5](05-scheduling.md) |
| `input` | deck · card | `draw`: answer on a canvas instead of typing. [→ ch 4](04-review-modes.md) |
| `direction` | deck · card | [Review direction](06-cloze-direction-images.md): forward, reverse, both. |
| `shape` | deck | The author's statement about the deck's content. The one value, `uniform-answers`, declares every answer the same kind of thing, which licenses [sampled Recognize options](04-review-modes.md) from the deck's own answers. Any other value is refused rather than silently ignored. Workspace `[defaults]` may set it too; card scope is an unknown-key lint. |
| `strictness` | workspace | [Exam](12-the-ai-exam.md) grading rigor for the members, in `alix.toml`'s `[defaults]` only: a learner setting, so a deck declaring it gets an unknown-key lint. |
| `requires` | deck | [Prerequisite deck](09-dependencies.md) that gates unlocks (repeatable). |
| `authors` | deck | Who made the deck: one value or a list. Holds people and any AI that helped, so there is no separate generated-by key. Yours to fill in; alix never rewrites it. |
| `license` | deck | The deck's licence, a single string, by convention an SPDX identifier. |
| `tags` | deck | Free-form labels: one value or a list. |
| `created-at` | deck | When the deck was made, a single string, by convention an ISO 8601 date. Stored verbatim and not validated. |
| `link` | deck | [tutor reference](10-tutor.md) URL, tutor-only (repeatable). |
| `source` | deck | [Exam ground truth](12-the-ai-exam.md): a YAML list of URLs, files, or directories (one entry is the norm), also a [trace](13-trace-decks.md)'s cited path and a tutor reference. It identifies evidence but never grants access to a wider local tree. A workspace `alix.toml` may declare a `source` too, as supporting context for its members. |
| `trace` | deck | What a [trace](13-trace-decks.md) walks; its presence makes the deck a trace. |
| `at` | card | A repeatable named-field locator into the `source` (`at: file:lines fingerprint: xxh64-...`, plus `asset:` once frozen; a range-less path or URL cites the whole source): a [trace checkpoint's](13-trace-decks.md) reveal target, or a [fact card's source citation](06-cloze-direction-images.md#source-citations) shown on reveal. |
| `given` | card | A [trace checkpoint's](13-trace-decks.md) off-screen symbol, as `name - meaning` (repeatable). |

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
(the picker's Depth… menu), the same way for every deck (see
[Reveal & session depths](04-review-modes.md)).
