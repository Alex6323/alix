# 0030: Personal learning data lives in a sidecar file

- Status: Accepted
- Recorded: 2026-08-07
- Retrospective: No

## Context

Alix holds two kinds of material about a deck: what its author wrote, and
what a learner accumulated against it. Today they are stored by
accident rather than by decision, and both placements are wrong.

Tutor notes are appended straight into the deck file by three routes
(`/api/ask/note`, `/api/remote/ask/note`, `/api/walk/ask/note`, all via
`deck::append_note`). The file a user authored, or received from someone
else, is edited by alix without being asked. Remediation and tutor cards
go the other way: they live in the progress store as
`VirtualCard { id, kind, deck, text, created_ms }`, where `text` is
Markdown held inside JSON, so it is neither editable as a file nor
queryable as data.

Alex stated the requirement as two goals that currently bite each other
(2026-08-06): a `deck.md` stays clean of personal learning data, and
personal learning data is edited the same way a typo in the deck is,
by opening the file and changing the line. Writing into the deck breaks
the first. Storing Markdown in JSON breaks the second.

This is a placement decision for user data with no obvious reversal, so
it is recorded rather than left to the implementation.

## Decision

Each deck may have one **personal sidecar**: an ordinary Markdown file
beside it, `spanish.md` and `spanish.personal.md`, holding everything
personal about that deck.

1. **Alix never writes the authored deck again.** Every route that
   appended to it writes the sidecar instead. Deck files change only by
   their author's hand, or by the id stamping that already exists.
2. **The sidecar is a deck file in format**, so it reuses the parser, the
   `<!-- id: card-... -->` anchor, stamping, `>` notes, and doctor. It is
   never listed in the picker and is never reviewable on its own.
3. **Two independent mechanisms carry discovery and linkage.** The
   `*.personal.md` suffix decides discovery, so the picker excludes a
   sidecar without reading any file. A `personal-for: deck-<token>`
   frontmatter key carries the link, so renaming either file keeps the
   pair intact and doctor can prove it.
4. **Notes address cards by id**, with a human hint alix refreshes:
   `<!-- for: card-<token> (darse cuenta) -->`. The id is the link and
   survives every edit to the card; the parenthetical is display only.
   A sidecar note renders after the deck's own note, never merged into
   it or reordered.
5. **Orphans are kept and reported.** When the addressed card is gone the
   note stays and `alix doctor` names it, quoting the hint. Alix does not
   destroy a user's writing because a different file changed.
6. **The sidecar is personal, so it never travels.** `alix share` sends
   the authored deck, its assets, and its augmentations, and excludes the
   sidecar exactly as it already excludes `progress/`.
7. **The store stops carrying learning content.** `VirtualCard` leaves
   it, and `promote_virtual` disappears with it: there is nothing to
   promote, because a personal card is already in a file. Moving one into
   the authored deck is a manual edit, like any other authoring act.

## Consequences

Easier: personal notes and cards are editable, greppable, diffable, and
backed up by whatever already backs up the user's decks. A deck can be
shared or replaced without carrying or losing anything personal.

Harder, and accepted: a deck that accumulates personal data becomes two
files. Creation is lazy, so decks that never accumulate anything stay
single files, but a heavy user ends up with a folder of pairs. That is
the honest cost of the second goal.

Deliberately unsupported: multiple sidecars per deck, workspace-level
sidecars, editing the sidecar through the web UI (the file is the
editor), an archive of discarded notes, and per-note provenance beyond
the marker.

**Alix co-owns one file.** This record deliberately creates a file that
both alix and the user write: alix appends notes and refreshes the
display hint, the user edits freely. That is the situation an authored
deck is already in through id stamping, so the machinery and its failure
modes are known rather than new, and it is strictly better than the
status quo where alix writes the file the user considers canonical.

## Alternatives considered

**Virtual notes in the progress store, plus an explicit adopt action
writing them into the deck** (`docs/specs/2026-08-05-deck-purity-spec.md`,
superseded). Rejected by Alex on both halves: adopting reintroduces
exactly the deck writing that purity forbids, and JSON storage leaves
personal data uneditable, which was the second goal all along.

**One file, with personal material marked by convention.** Keeps a single
file per deck, but a shared or replaced deck then carries or loses
personal data, and alix must keep writing the authored file.

**A database or structured store for personal material.** Queryable, but
it fails the second goal exactly as the JSON store does, and it
contradicts local-first plaintext.

## Compatibility

Pre-1.0, so no migration. Existing `VirtualCard` entries are **dropped,
not converted** (Alex, D6): remediation cards regenerate the next time an
exam finds the same gap, and a tutor card can be asked again. Production
code therefore never learns the old shape.

One consequence must be handled rather than assumed: the store's
`CardState` entries keyed by those dropped virtual ids become orphans, so
existing orphan pruning must cover them instead of leaving dead progress
behind.

The sidecar's own format carries no version. It is a deck file, and it
follows the deck format's rules.

## Security

The sidecar is plaintext personal data at rest, beside the deck, with the
same permissions as the deck. That is not a new exposure: the deck, the
progress store, and the augment cache are already plaintext in the same
tree.

What is new is an exclusion that must hold: `share` and `receive` move
decks between people, and the sidecar must never be in a bundle. Point 6
is the constraint, and it needs its own regression test rather than
inheriting confidence from the `progress/` exclusion, which works by
directory while a sidecar sits beside the deck.

## Verification

- The three note routes leave the deck file byte-identical: hash it
  before and after each route, expect no change.
- Doctor reports all four pairing findings (missing parent, mismatched
  parent, a card id in both files, and `personal-for:` on a file without
  the suffix) plus the orphaned-note warning.
- A sidecar never appears in the picker, through every lister.
- A sidecar never enters a share bundle.
- Orphan pruning removes `CardState` entries for ids no file carries.
- The pairing and merge rules are property-tested as pure functions
  before they are wired to the filesystem
  (`docs/plans/2026-08-07-personal-sidecar-plan.md`).

## Reversal

Evidence that would justify replacing this record: users routinely losing
or failing to find their sidecars, or the two-file cost proving worse in
practice than the deck-writing it replaced.

Reversal is not free. Personal material would have to be moved back into
some other store, and by then sidecars will contain writing that exists
nowhere else, so any replacement must migrate rather than drop, which is
the opposite of this record's own compatibility stance and is only
available after 1.0.
