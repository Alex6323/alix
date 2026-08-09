# 0032: A cloze hole is addressable by name, and grouping starts fresh

- Status: Accepted
- Recorded: 2026-08-09
- Retrospective: No

## Context

A `>` note attaches to the authored block, not to the card under review. Every
cloze sub-card of one sentence therefore carries the same note, so reviewing a
later hole can display an earlier hole's answer before the learner has met it:

    ## The test pyramid, bottom to top
    \blank{Unit}, \blank{integration}, \blank{end-to-end}
    > Unit tests sit at the base because they are fastest and most numerous.

Measured on the live corpus 2026-08-06: 63% of 144 cloze blocks have two or
more holes, and 30 of the 89 multi-hole blocks carrying a note (34%) contain a
hole's answer verbatim. `feat(doctor): report a note that spells out one hole's
answer` now warns about it, but a warning cannot fix a deck that wants both a
shared note and a per-hole one.

Three separate roadmap items ({#cloze-hole-hints}, {#cloze-grouped-holes}, and
per-hole notes) are the same missing capability seen three times: **there is no
way to address one hole**. Solving them separately would mean three grammars
and three things an author must learn for one concept.

This record fixes the addressing syntax, which is written into deck files and
therefore permanent, and the review-history consequence of grouping, which
cannot be discovered later without corrupting schedules.

## Decision

**1. A hole is addressed by an optional name: `\blank[name]{answer}`.**

The name is drawn from `[a-zA-Z0-9_-]+`, contains no whitespace, and is closed
by `]`. A malformed name stays the existing loud `ClozeBracketReserved` parse
error rather than degrading to an unnamed hole, so a typo cannot silently
detach a payload from its hole.

The name addresses the hole for every payload the format grows: the per-hole
note now, the grouping key next, and a hint if and when the one hint story
resolves. One addressing idea, learned once.

**2. A name is an address, not an identity.**

Card identity remains the minted `card-<token>` plus the positional or remapped
sub-id (`-N`). A name is scoped to one authored block and carries no meaning
outside it. Renaming a hole must not be understood by anyone, author or
implementer, as renaming a card. The book must say this where it introduces the
syntax.

**3. Grouping two holes under one name starts that card's history fresh.**

Two holes sharing a name are drilled as a single sub-card asking both spans.
Recalling two spans together is a harder question than either alone, so
inheriting either hole's stability would overstate what the learner knows and
schedule the merged card too far out. The merged card therefore takes no
inherited schedule.

Holes that were not merged still remap through the existing fingerprint
cascade: `store::realign_holes` matches `HoleFingerprint { text_fp, line_fp }`
pairwise, so a positional shift caused by a merge elsewhere in the block does
not reset unrelated holes. Only the merged pair resets.

## Consequences

Easier: one grammar serves notes, grouping and any later per-hole payload;
authors who need none of it write exactly what they write today, because the
name is optional and the block-level `>` note keeps its current meaning.

Harder, and accepted: grouping costs the merged card's review history, once,
at the moment the author groups. That is a real loss and it is deliberate,
because the alternative is a schedule that lies.

Deliberately unsupported: names as cross-deck or cross-edit identifiers, names
on anything other than a cloze hole (per-direction and table-level notes are a
separate record, {#note-addressing}), and any payload that is not literal text.
This is not a templating language: no conditionals, no variables, no computed
text.

## Alternatives considered

**Indexed note lines** (`> 2: text`). Cheapest, and rejected twice over: it
collides with a note that legitimately begins `2:`, and positional indices
break when a hole is inserted, which is the same fragility that retired
content-hash card ids.

**Delimiters inside the marker** (`\blank{answer | hint}`). Compact and immune
to renumbering, but it makes every answer's text delimiter-sensitive and grows
the one construct authors already know into a small language.

**A per-hole directive comment** (`<!-- hole: 2 note: ... -->`). No delimiter
collisions, but verbose and still positional.

`\blank[name]{...}` is the only sketch the format already anticipated:
`\blank[` has returned `ClozeBracketReserved` since before this problem was
stated, minted for exactly this future.

## Compatibility

Pre-1.0, so no old shape is recognized. Nothing needs migrating: `\blank[`
currently fails as a parse error, so no deck in existence can contain a named
hole, and no reader has to guess whether a name means something older.

The block-level `>` note is unchanged in meaning and remains the default for
every hole, so decks that use no names behave identically.

## Security

Unchanged. A name is inert text in a local file, is never executed, and is not
a path, an id, or a lookup key outside its own block.

## Verification

- `\blank[base]{Unit}` parses and the sub-card carries the name; `\blank[]{x}`
  and `\blank[a b]{x}` stay `ClozeBracketReserved`.
- A per-hole note replaces the block note for its hole and leaves the other
  holes' notes untouched: the pyramid fixture stops leaking, and the doctor
  warning added in `94ec122` goes silent on it.
- Grouping two holes yields one sub-card with no inherited schedule, while an
  unmerged hole in the same block keeps its history through
  `store::realign_holes`.

## Reversal

Evidence that would justify replacing this: authors treating a name as a stable
id despite the documentation, or grouping proving rare enough that its identity
cost is not worth the mechanism. Reversing the syntax after decks contain named
holes means rewriting those decks, which is external tooling over `*.md`, not
code in this repository.
