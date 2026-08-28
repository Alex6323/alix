# 0002: Markdown-native decks

- Status: Accepted
- Evidence: a_card_runs_from_its_h2_to_the_next_h2_or_eof in src/parser/mod.rs
- Recorded: 2026-07-24
- Retrospective: Yes
- Refined by:
  [ADR 0018](0018-explicit-deck-initialization.md), which makes deck
  initialization explicit, and
  [ADR 0026](0026-self-describing-ids-and-named-locator-fields.md), which
  replaces the `alix-id` key with `id: deck-<token>` while preserving the
  Markdown-native format.
- Details evolved 2026-08-24: a blockquote opens a note only when its first
  line is one of GitHub's five alert badges; an unbadged blockquote is a
  quotation that belongs to the answer and reveals with it. Evidence in
  `src/parser/mod.rs`; the Markdown-native decision is unchanged.

## Decision history

The Markdown format shipped on 2026-07-19. Commit `8c67cb6` introduced the
parser, `bb6cd21` moved consumers to `.md`, and `86f2af3` removed the previous
custom text-format scaffolding. Commit `624889e` is the first release-line
commit after the conversion.

Early design work considered deriving identity from content. The shipped
format instead stores minted identity tokens, as recorded separately in ADR
0003.

## Context

Alix decks must remain useful as ordinary files while expressing card
boundaries, notes, cloze holes, configuration, and machine-maintained identity.
Standard Markdown supplies portable prose and formatting, but its semantic
document model does not define Alix's card grammar.

The previous custom `.txt` format made Alix-specific structure visible, but it
created a private interchange format and duplicated conventions already
understood by editors and publishing tools.

## Decision

`.md` is the canonical deck extension and Markdown is the authored interchange
format.

Alix applies a line-oriented structural grammar around Markdown:

- YAML frontmatter holds deck metadata, including the namespaced `alix-id`
  identity marker.
- A level-two heading starts a card and supplies its front.
- The following block supplies the answer and optional context.
- Blockquotes represent notes.
- Fenced code remains literal while structural scanning is active.
- HTML comments carry machine directives such as identity and source
  locations without changing the rendered prose.

The parser keeps authored text, normalized content, and display projection
separate. CommonMark-like inline formatting may change how a client displays a
card, but it does not redefine the card boundary, grading content, or identity.

The pre-1.0 custom text format was removed cleanly. Alix does not carry a
permanent compatibility reader for it.

## Consequences

- Decks work naturally with editors, version control, diff tools, and Markdown
  publishing.
- The structural subset must be specified more precisely than "valid
  Markdown."
- Parser changes can affect persisted content and require compatibility tests.
- Display renderers may evolve without changing the canonical authored text.
- Users of the removed pre-1.0 format must convert their decks rather than rely
  on an indefinite dual parser.

## Alternatives considered

### Keep the custom text format

This preserved the old parser but kept Alix content in a private format and
made Markdown tooling less useful.

### Treat decks as unrestricted semantic Markdown

A pure document-tree interpretation cannot precisely preserve Alix's
line-addressed card boundaries, directives, fenced-code exclusions, and
machine stamping. The product needs a documented Markdown profile rather than
an arbitrary CommonMark document.

### Store rendered HTML as the canonical format

HTML would make authoring and review diffs noisier, weaken plain-text
readability, and mix content with a particular presentation.

## Compatibility

The grammar documented in `docs/book/src/03-the-deck-format.md` is a persisted
file-format boundary. Existing `.md` decks must continue to parse with the same
card boundaries and directives, or a change must provide a converter and an
explicit compatibility plan.

## Security

Markdown input is untrusted content. Display projection must continue to
control which HTML-like constructs and generated artifacts reach clients.
Machine directives must not become an escape hatch for arbitrary executable
content.

## Verification

- `src/parser/` owns structural parsing and its fixtures.
- `src/inline.rs` separates normalized content from display runs.
- `src/stamp.rs` inserts machine directives without rewriting authored card
  content.
- `docs/book/src/03-the-deck-format.md` documents the public grammar.
- Parser, projection, and stamping tests cover fences, directives, notes,
  frontmatter, and byte-preserving insertion.

## Reversal

A replacement format requires a deterministic converter for existing decks,
round-trip tests for every supported construct, a migration and rollback
story, and a stated interchange format. A second permanent parser is not the
default migration strategy.
