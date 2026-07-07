# 7 · Directives reference

Every card marker and `% key: value` directive in one place. **Scope** is where
each may appear — *deck* = the header (before the first card), *card* = after a
card's front, *anywhere* = both. Each links to the chapter that explains it in
full.

| Token | Scope | What it does |
| --- | --- | --- |
| `#` front | card | Starts a card at column 0; the indented lines below are the answer. [→ ch 3](03-the-deck-format.md) |
| `!` line | card | A note, shown after you answer. [→ ch 3](03-the-deck-format.md) |
| `%` line | anywhere | A comment — ignored, unless it's one of the directives below. |
| `% reveal:` | deck · card | [How the answer is uncovered](04-review-modes.md): flip (default), cloze (`{{spans}}`), line. |
| `% order:` | deck | Card order: `scheduled` (default) or `sequential`. [→ ch 5](05-scheduling.md) |
| `% direction:` | deck · card | [Review direction](06-cloze-direction-images.md): forward, reverse, both. |
| `% img:` / `% img-back:` | card | [Image](06-cloze-direction-images.md) on the front / back. |
| `% img-dir:` | deck | Base directory image filenames resolve against. [→ ch 6](06-cloze-direction-images.md) |
| `% title:` | deck | [Display name](08-workspaces.md) shown instead of the file name. |
| `% requires:` | deck | [Prerequisite deck](09-dependencies.md) that gates unlocks (repeatable). |
| `% link:` | deck | [tutor reference](10-tutor.md) URL — tutor-only (repeatable). |
| `% source:` | deck | [Exam ground truth](12-the-ai-exam.md) (URL/file, repeatable); also a [trace](13-trace-decks.md)'s path origin, and a tutor reference. |
| `% origin:` | deck · card | Live source root a [frozen deck](14-explore.md)'s snapshots came from (set in a workspace's `alix.toml`); enables [tutor](10-tutor.md) grounding and [`alix deck check`](17-command-reference.md) drift detection — `% source:` itself points at the frozen `assets/`. |
| `% strictness:` | deck | [Exam grading rigor](12-the-ai-exam.md): strict, balanced, lenient. |
| `% trace:` | deck | What a [trace](13-trace-decks.md) walks; its presence makes the deck a trace. |
| `% at:` | card | A locator into the `% source:` (`file:lines`): a [trace checkpoint's](13-trace-decks.md) reveal target, or a [fact card's source citation](06-cloze-direction-images.md#source-citations) shown on reveal. |
| `% given:` | card | A [trace checkpoint's](13-trace-decks.md) off-screen symbol, as `name — meaning` (repeatable). |

## `% link:` vs `% source:`

Two that look similar but aren't. Both point at material a deck is about, but
`% source:` is the **exam's ground truth** — questions are generated from it and
answers graded against it — and a URL source doubles as a tutor reference.
`% link:` is **only** a tutor reference and never becomes exam material; use it for
supplementary reading the exam should ignore. The implication runs one way: a
`% source:` URL is offered to the tutor, but a `% link:` is never promoted to a
source.

## Precedence

Where a directive can come from several places, the more specific wins:

> CLI flag > card `%` directive > deck `%` directive > workspace `[defaults]` > built-in default

So a card's `% reveal:` overrides the deck's, which overrides a workspace's
`[defaults]`, which overrides `alix`'s default (`flip`); where a CLI flag exists
for a directive (e.g. `--strictness` for `% strictness:`) it wins over all of them.

The session depth (Recognize/Recall/Reconstruct) is **not** in this chain
either — it isn't config or a deck directive at all. It's chosen per session
(`--depth`, or the picker's Learn ▾ menu), the same way for every deck (see
[Reveal & session depths](04-review-modes.md)).
