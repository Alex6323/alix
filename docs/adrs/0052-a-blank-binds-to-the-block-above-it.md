# 0052: A blank binds to the block above it, and a blank card has no answer section

- Status: Proposed
- Recorded: 2026-10-11
- Retrospective: No

## Context

A card has a question side, shown before Reveal, and an answer side, shown
after. On a plain card the `##` line is the question and the lines below it
are the answer. A card with a blank (a `<!-- blank: span ... -->` hiding text,
or a `<!-- blank: rect ... -->` hiding part of an image; ADR 0034, ADR 0039)
did not follow one rule:

- A text blank searched the card's answer lines, wherever its directive sat.
  Those answer lines were then shown before Reveal as the question's context,
  with the blanked words masked. So a text cloze needed a `##` question above
  a sentence that asked the same thing again:

      ## What does a committee chain walk usually start from?
      The trust root of a committee chain walk is typically the genesis committee.
      <!-- blank: span hidden="genesis committee" -->

- An image blank bound to the nearest image above it on the same side of
  `---`. Its card's answer lines were also shown before Reveal, but an image
  blank masks no text, so an answer written below `---` appeared before
  Reveal. Measured 2026-10-11 on real decks: 9 of 12 image-blank cards
  showed their answer this way.

On reveal, a cloze card kept the blanked sentence (`... the ⍰.`) and showed
the hidden words in a separate answer area below it.

A span names its own answer: `hidden="..."` is required on it. A rect may
omit it, and such a card asks with no answer text, so a typing check opened
on it can never pass. Answer lines on a blank card that names its answer
never carry it; they can only repeat it or explain it, and a note already
explains.

The region grammar (`span`, `rect`, `cover:`, `crop:`, `hidden=`,
`occurrence=`, the minted `position:` and `b:` stamp, named groups) and the
canonical maskable stream are defined in ADR 0034; this record uses those
terms unchanged. The deck format is permanent, so where a blank binds and what a blank card
shows before Reveal are decided here, before decks depend on them.

## Decision

**1. A region directive binds to the block directly above it.** A region
directive is a `blank:`, `cover:` or `crop:` line. It applies to the nearest
block above it; comment lines (`<!-- ... -->`) and link definitions
(`[name]: url`, which alix lifts out as deck metadata) between the two do
not count, so a card's other directives may sit in between in doctor's
canonical comment order. A blank line anywhere between a directive and its
block, among those lines included, breaks the binding. The inside of a code
fence or a `$$` block is never another block. A block is, in this order of
recognition:

- the `##` line, measured over its visible text (trailing comments
  stripped, a closing run of `#`s stripped only when a space precedes it,
  `\#` unescaped); when it holds only media it is also a media line;
- a media line (a line holding only an image, audio or video);
- a code fence, opener to closer;
- a `$$` display-math block, opener to closer;
- otherwise a maximal run of consecutive non-blank content lines. A
  comment line ends the run, as it ends a paragraph in CommonMark; a link
  definition does not, since it is not content. A list
  with blank lines between its items is several blocks, whatever the
  indentation; a media line ends the run above it; an unbadged `>`
  quotation is content and joins its run; a note (a `>` block whose first
  line is a `[!NOTE]`-style badge) ends the run and is not a block. The
  `---` divider is not a block.

Several directives may stack under one block and all bind to it. No blank
line may separate a directive from its block. A span is matched in the
block's canonical maskable stream (ADR 0034). A directive whose block
cannot match it is a hard deck error: a span (`blank:` or `cover:`) whose
hidden text is not in the block, a rect (`blank:` or `cover:`) or a crop
under a block that is not a media line, a directive under a note, directly
under `---`, in section prose outside a card, or in a card
table (`<!-- cards -->`) including under its title line. `occurrence=` and
the minted `position:` count within the block. A named group's members may bind to
different blocks.

**2. A card with a blank has no answer section.** A card with a blank is a
card on which a `blank:` binds; `cover:` and `crop:` alone do not make one.
On a card with a blank, everything above the notes is the question and is
shown before Reveal: the card's own blanks as the asked marker, the
block's other blanks and its covers as the hidden marker, and neither
marker carries text. A `---` divider on such a card is a hard deck error ("a cloze card
has no answer section; put explanations in a note"). Notes are unchanged:
they trail the card, show after Reveal, and content after a note stays an
error. A card without a blank keeps its
front and answer split, and its covers and crops bind by rule 1.

**3. A card with a blank needs no answer lines.** The text cloze above
becomes one statement of the fact:

    ## The trust root of a committee chain walk is typically the genesis committee.
    <!-- blank: span hidden="genesis committee" -->

A card without a blank still needs answer lines ("card front without an
answer"), unless it carries `<!-- ignore -->`, which keeps exempting a
draft card as before. A `##` line that is itself a media line, so that an image with its
rects is the whole question, falls under the same rules once the parser
accepts it; it needs no further binding rule.

**4. Every cloze reveal fills the card's own blanks in place.** The library
supplies the revealed question (the card's own spans shown and marked as the
answer, sibling spans still hidden; an image blank's region uncovered, as
before), and clients render it in place of the masked question. No separate
answer area repeats the hidden text. Grading is unchanged: typed and
recalled answers are checked against the hidden text. In line-by-line
reveal, each step fills the card's next own blank (a span, or a rect group)
in the order its directives appear.

**5. An unrevealed text blank has a fixed width.** The pre-reveal view
carries a marker, never the hidden text or its length, and clients draw
every unrevealed text blank, asked or hidden, at the same width, so the gap cannot
narrow the answer down. An image blank's box is the region it covers.

**6. Card and blank identity is unchanged.** Cards keep their minted
`card-<token>`, each blank its `b:` stamp, a named group its derived
`-g<hash>` (ADR 0034). Content fingerprints are not identity and do change
(see Consequences).

**7. A `direction:` directive on a card with a blank is a hard deck
error.** Reversing a cloze has no meaning, so `both` and `reverse` alike
fail on the card. A deck-wide `direction:` in frontmatter keeps skipping
cards with a blank, so one cloze card never fails such a deck.

**8. The deck generator and the exam's remediation cards write text cloze
cards without a separate question**, the sentence on the `##` line. The
tutor's card draft is out of scope here.

**9. Doctor warns when another block of the question shows a blank's
hidden text.** It compares against what the question displays, after
masking. The blank's own block is masked and never triggers it; a
different block shown before Reveal, such as a `##` question repeating the
hidden words above the blanked sentence, does. The card's section prose
(its `#` heading and the prose under it) counts too: clients open it
before the question on a card's first introduction.

**10. Every blank names its answer.** A `hidden="..."` that grading can check is
required on a `blank:` of every shape, so a rect without it is a hard deck error like a
span without it. Typing, choices and the exam then grade every blank against
its text. A `cover:` asks nothing and keeps `hidden=` optional and inert.

### Amends ADR 0034

This record replaces, in ADR 0034:

- "Binding is positional, and shape-specific": geometric shapes binding to
  the nearest preceding media element on the same side of `---`, and `span`
  and `cover: span` binding to the card's answer block. All region
  directives now bind by rule 1, and a blank card has no `---`.
- `occurrence=N` counting "over the answer block's lines", and `position:`
  indexing "the block's canonical maskable stream" where the block was the
  answer block: both now count within the bound block.
- The doctor lint "the hidden text occurring more than once in the block":
  rule 9 takes its place, since an uncovered second occurrence stays
  visible in the masked question.
- `hidden="..."` "optional on a geometric `blank:`": it is now required on
  every `blank:` (rule 10), which retires the unlabelled image region that
  asked with no answer text.

The rule stays stated over media lines, so audio and video inherit it.

## Consequences

Easier: one binding rule for every region directive, readable without
counting; one statement per fact; no answer can appear before Reveal by
sitting in a line the format calls answer; a reveal that reads as the
completed question; a span's search area shrinks to one block, so a word
repeated elsewhere in the card is no longer ambiguous.

Harder, and accepted:

- The client contract changes twice: the card view's existing `front`
  becomes the masked question for a blank card (its runs, units and plain
  text never carry hidden text), and a revealed question is added. Both
  clients, the hand-mirrored mobile bridge struct, the contract snapshots,
  browse and `alix list` (which show the masked question) follow.
- Rewriting decks changes the content fingerprint of every rewritten cloze
  card, so cached AI augmentations for those cards are recomputed and
  previously generated remediation cards stop matching them for dedup.
- Decks must be rewritten with disposable tooling outside the repository,
  since the format keeps no backwards compatibility before 1.0: directives below a note, after a blank line, above
  their text, or separated from their image by prose; text after `---` on a
  blank card. Measured 2026-10-11: about 250 directive lines in real decks,
  counting only the first two shapes. The tooling deletes minted
  `position:` values and lets the stamper mint them again; it never writes
  a position itself.
- Tracked decks and fixtures are rewritten by hand in the same change: the
  phone's bundled tutorial deck, the book, the README example, the slides,
  and the test fixtures.
- The phone release that carries the new parser ships before the desktop
  release, so a rewritten deck never reaches a phone that cannot read it.
- The generator prompts and the shared card-shape guidance
  (`docs/include/card-shapes.md`) change, so the costed `make shape-eval`
  and `make docs-audit` rerun before the release.

Deliberately unsupported: a blank separated from its block; any answer
section on a blank card. Follow-ups, not decided here: a quotation inside a
note, which is today a hard error and was the only use of an answer section
on a cloze card that a note could not take over; the tutor's card draft
choosing its shape from the shared card-shape guidance.

## Alternatives considered

**The front ends at `---` on every card.** One rule, but an image card with
nothing to add as answer would need a `---` with nothing below it. Rejected
by ruling for the blank-card rule above.

**A blank card keeps an answer section after `---`.** Shown after Reveal,
never graded, since every blank carries its own answer. A note does the
same job; the only content a note cannot hold today is a quotation, which
is better solved inside notes. Rejected by ruling.

**Directives may trail the card, a text blank searching the whole front.**
Keeps today's directive placement, but an image blank then needs a second
addressing rule, and a word repeated in the front is ambiguous. Rejected by
ruling in favour of binding to the block above.

**A literal "directly above", with comment lines counting.** Doctor's
canonical comment order places card directives such as `reveal:` between a
block and its blank, so its own repair output would fail. Changing the
canonical order instead would rewrite every stamped deck's layout.
Rejected.

**An authoring rule only** (keep the forced question, require a new angle).
Rejected: it leaves the duplication to every author and the generator, and
fixes no leak.

## Compatibility

Pre-1.0, so no old shape is recognized. The shapes listed under
Consequences fail as ordinary deck errors after this change, with one
exception: a directive above its text whose block above happens to contain
the hidden text binds there, and the text below then shows the hidden words
in the question, which rule 9 flags. External tooling moves every listed
shape before the release that carries it. The API change is
recorded in `docs/API.md` and the contract snapshots.

## Security

Unchanged in kind. Binding reads the same authored text through the same
stream builder; the `##` line becomes a new input to it.

## Verification

To be written with the implementation; the record names its evidence
before it is Accepted. Expected laws:

- every region directive binds to the block directly above it, comment
  lines skipped, and every other placement is a hard error, for span, rect,
  cover and crop;
- each block kind in rule 1 bounds a span's search, including a media line
  splitting a run and a `$$` block containing blank lines;
- a card with a blank and a `---` fails; a card with a blank and no answer
  lines parses; without a blank it fails;
- no pre-reveal field of a blank card's view carries its hidden text;
- every cloze card view carries a revealed question with only its own
  blanks filled, on both clients;
- `direction: both` on a blank card fails, and a deck-wide `both` still
  parses its blank cards;
- the narrowed lint fires on a question block repeating hidden text and
  never on the blank's own block;
- the generator's and remediation's cloze shape parses with no answer
  lines.
- a `blank:` of any shape without `hidden=`, or with an empty one, fails,
  and a `cover:` without it parses.

## Reversal

Evidence that would justify revisiting: authors measurably needing to bind
a blank away from its block, blank cards proving hard to read without a
separate question, or a graded need for content after the question on a
cloze card. Reversal means rewriting decks with external tooling, under a
new record.
