# 0031: A personal file copies nothing, and a note names its card first

- Status: Accepted
- Recorded: 2026-08-08
- Retrospective: No
- Supersedes:
  [ADR 0030](0030-personal-data-lives-in-a-sidecar-file.md)

## Context

ADR 0030 placed personal learning data in a sidecar file and fixed its shape
in the same record: a `personal-for:` frontmatter key, and note blocks
addressed as `<!-- for: card-<token> (darse cuenta) -->`, whose parenthetical
is a human hint alix keeps refreshed.

Building it put two of those choices in front of Alex, who rejected both
(2026-08-08).

The hint copies the deck's own words into a second file. Alex: "so what
happens if the author of the deck changes the question, and the personal one
thereby goes stale? I don't like to introduce this staleness problem."
Refreshing the copy does not answer that. It is wrong between the deck edit
and the next refresh, and it makes alix rewrite a file whose whole point is
that the user edits it by hand.

The marker's position was settled as trailing, mirroring the closing
`<!-- id: -->` of a deck card. A personal **hint** is planned as a third block
kind. With a trailing marker, neither a parser nor a reader can tell what a
block is until the block ends: two kinds can be buffered and classified at the
end, three cannot.

## Decision

0030's placement decision stands unchanged: one optional `<deck>.personal.md`
per deck, excluded from discovery and from share, carrying every note and card
that alix or the learner adds, so the authored deck is never written again.
This record replaces its file format only.

1. **The frontmatter link is `for: deck-<token>`.** The file name already says
   whose file this is, so the key does not repeat it.
2. **Nothing in a personal file copies anything in the deck.** The card id is
   the only link. No human hint, no seeded heading, no field alix refreshes:
   every such copy is a cache, and a cache in a hand-edited file goes stale.
3. **Identity closes a block; attachment opens it.** A card ends with
   `<!-- id: card-<token> -->`, exactly as in a deck, because the id names the
   card itself. A note begins with `<!-- note: card-<token> -->`, because a
   note is an attachment and has to declare its target before anything below
   it means anything.
4. **Each block kind is its own keyword**, rather than one marker carrying a
   kind field. The parser dispatches on a single token, so a misspelled kind
   fails as an unknown marker instead of as a valid marker with a bad field.
   `hint:` is reserved for the planned third kind and is not recognized today.
5. **An orphan note is kept and reported by card id**, since there is no hint
   left to quote.

## Consequences

Easier: a personal file has no derived content at all, so no edit to the deck
can falsify it, and alix never rewrites a line the user did not ask it to.
Reading it top to bottom, machine and human learn what a block is before they
read it, which is what admits further block kinds without another format
decision.

Harder: a note block no longer carries a human-readable trace of the card it
belongs to, so reading a personal file on its own tells you less than it did.
A reader who wants a label writes their own `## ` heading above the marker;
alix leaves it alone and never writes one.

Deliberately unsupported: with the marker leading, a note block cut into a
real deck attaches its `>` lines to whatever card precedes it, instead of
failing loudly as `card front without an answer`. Alex weighed that against
the third block kind and accepted it.

## Alternatives considered

**Keep the hint and refresh it.** Rejected on Alex's staleness objection; see
Context.

**One marker with a kind field**, `<!-- for: card-x note -->`. Rejected: a
misspelled kind still parses as a marker, so it fails as a valid block with a
bad field rather than as no block at all.

**Keep the trailing marker and buffer.** Works for two kinds by classifying at
the end of the block; does not survive the third.

## Compatibility

The project is pre-1.0, so no old shape is recognized. A file carrying
`personal-for:` fails as an unknown frontmatter key, and a file carrying
`<!-- for: ... -->` or the hinted form has no marker at all, so its `>` lines
are ordinary text. Neither gets a dedicated message or a suggested remedy.
The format was never released: v0.7.0 predates the sidecar, and the one
personal file in existence was rewritten by hand.

## Security

Unchanged. The file is local, never bundled by `alix share` in either
direction, and holds only what the user or their own tutor session wrote.

## Verification

- `src/parser/sidecar.rs`: the marker grammar and the block boundary, with
  `a_marker_carrying_anything_beyond_the_id_is_not_a_marker` and
  `a_marker_naming_a_kind_alix_does_not_write_yet_is_not_a_note` pinning
  points 3 and 4, and
  `without_notes_leaves_a_card_whose_front_follows_a_stray_marker` pinning the
  card/note ambiguity.
- `src/personal.rs::every_note_written_to_a_block_stays_below_its_opening_marker`
  pins what alix writes.
- `tests/api.rs::a_tutor_note_leaves_the_authored_deck_untouched_and_writes_the_sidecar`
  pins it end to end over the JSON API.
- `tests/cli.rs::doctor_reports_every_way_a_personal_file_can_be_wrong` pins
  point 5.

## Reversal

A block kind that cannot be named by a single keyword, or a reader study
showing an unlabelled note block is unreadable in practice, would justify
revisiting the marker. Reversing it means rewriting existing personal files,
which is an external one-off script over `*.personal.md`, not code in this
repository.
