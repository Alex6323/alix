# 7 · Directives reference

Every card marker and deck/card key in one place. **Scope** is where each may
appear: a *deck* key is a line in the frontmatter (the `---`-fenced YAML block
at the top of the file), a *card* key is a `<!-- key: value -->` comment after
a card's front, and *deck · card* keys work either way, with the card one
taking precedence. The heading rows are structure rather than keys: `#` scopes a
section, the rest scope a card. Each links to the chapter that explains it in
full.

| Token | Scope | What it does |
| --- | --- | --- |
| `##` front | card | Starts a card at column 0; the lines below are the answer. [→ ch 3](03-the-deck-format.md) |
| `#` heading | section | Opens a section: its text and the prose under it are the shared context of every card below, shown on demand (`c` in the web app), never while you answer. Takes no directives and no card ID. [→ ch 3](03-the-deck-format.md#sections-and-sub-cards) |
| `###`/`####` front | card | A sub-card of the card one level shallower, withheld from review until that parent graduates. Nothing goes deeper than `####`. [→ ch 3](03-the-deck-format.md#sections-and-sub-cards) |
| `>` line | card | A note, shown after you answer. [→ ch 3](03-the-deck-format.md) |
| `<!-- -->` | anywhere | A comment with no recognized key: ignored. |
| `format-version` | deck | The deck **format's** version, not the deck's own. Written by `alix deck init` above `id`, stays `1`, and any other number is refused rather than guessed at. Mandatory once a deck has an `id`. [→ ch 3](03-the-deck-format.md) |
| `id` | deck | The frontmatter deck ID (`deck-<token>`) marks an initialized deck and authorizes maintenance of missing card IDs. Its `deck-` prefix is what tells alix's decks apart. [→ ch 3](03-the-deck-format.md) |
| `id` | card | The HTML-comment card ID (`card-<token>`) anchors review history. It is minted by `alix deck init` or a deck-creation workflow and maintained by alix, never hand-authored. After a card table it is the table's container ID; each row's card composes it with the row stamp (`r:`) in the row's first cell. [→ ch 3](03-the-deck-format.md) |
| `reveal` | deck · card | [How the answer is uncovered](04-review-modes.md): flip (default) or line. Cloze is triggered by `\blank{...}` markers, never by a `reveal:` value. |
| `order` | deck | Card order: `scheduled` (default) or `sequential`. [→ ch 5](05-scheduling.md) |
| `input` | deck · card | `draw`: answer on a canvas instead of typing. [→ ch 4](04-review-modes.md) |
| `direction` | deck · card | [Review direction](06-cloze-direction-images.md): forward, reverse, both. |
| `sampling` | deck · card | `on` (default) or `off`: whether a [card table](03-the-deck-format.md)'s rows may draw Recognize options from their own column. A table's value overrides the deck's in either direction. |
| `strictness` | workspace | [Exam](12-the-ai-exam.md) grading rigor for the members, in `alix.toml`'s `[defaults]` only: a learner setting, so a deck declaring it gets an unknown-key lint. |
| `requires` | deck | [Prerequisite deck](09-dependencies.md) that gates unlocks (repeatable). |
| `title` | deck | The deck's display name, a single non-empty line. Without it a deck is named by its condensed `trace:`, else by its filename stem; a `#` heading is never the name. [→ ch 3](03-the-deck-format.md) |
| `description` | deck | A short summary, shown in the web picker's deck drawer. [→ ch 3](03-the-deck-format.md) |
| `authors` | deck | Who made the deck: one value or a list. Holds people and any AI that helped, so there is no separate generated-by key. Yours to fill in; alix never rewrites it. |
| `license` | deck | The deck's licence, a single string, by convention an SPDX identifier. |
| `created-at` | deck | When the deck was made, a single string, by convention an ISO 8601 date. Stored verbatim and not validated. |
| `link` | deck | [tutor reference](10-tutor.md) URL, tutor-only (repeatable). |
| `source` | deck | [Exam ground truth](12-the-ai-exam.md): a YAML list of URLs, files, or directories (one entry is the norm), also a [trace](13-trace-decks.md)'s cited path and a tutor reference. It identifies evidence but never grants access to a wider local tree. A workspace `alix.toml` may declare a `source` too, as supporting context for its members. |
| `trace` | deck | What a [trace](13-trace-decks.md) walks; its presence makes the deck a trace. |
| `at` | card | A repeatable named-field locator into the `source` (`at: file:lines fingerprint: xxh64-...`, plus `asset:` once frozen; a range-less path or URL cites the whole source): a [trace checkpoint's](13-trace-decks.md) reveal target, or a [fact card's source citation](06-cloze-direction-images.md#source-citations) shown on reveal. |
| `given` | card | A [trace checkpoint's](13-trace-decks.md) off-screen symbol, as `name - meaning` (repeatable). |
| `blank` | card | Masks and asks: a region of the preceding image (`rect x= y= width= height= hidden="..."`, optional `[group]`) or a stored text blank in the answer block (`span hidden="..."`, optional `occurrence=`/`boundary=`). Carries a minted `b:` stamp (and `position:` anchor on a span), maintained by alix like ids. [→ ch 6](06-cloze-direction-images.md) |
| `cover` | card | Masks without ever asking, for legends and labels that give answers away: `rect` on the preceding image or `span` in the answer block. No group, no stamp, no card. [→ ch 6](06-cloze-direction-images.md) |
| `crop` | card | A viewport onto the preceding image (`rect x= y= width= height=`, at most one per image); region coordinates stay in full-source space. [→ ch 6](06-cloze-direction-images.md) |

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
