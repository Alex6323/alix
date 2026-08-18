# 0034: Image region grammar and derived group identity

- Status: Accepted (NOT yet implemented; the grammar and identity were ruled
  frozen 2026-08-18 and the base build is next, so this record is binding
  while its evidence lands with that build)
- Recorded: 2026-08-18
- Retrospective: No

## Context

Alix hides spans of an answer with `\blank{...}` and asks the learner to recall
them. The same idea over a picture (hide a region of an image and ask what is
under it) is what other tools call image occlusion. It needs a way to write
regions into a Markdown deck.

Two properties make these choices durable rather than routine.

**They are permanent from the first deck that uses them.** A deck is the user's
plain-text file. A coordinate form, a key name, a unit, a delimiter or an
escaping rule that ships is in files alix does not own, so accepting a form now
and rejecting it later is a format break, and pre-1.0 freedom to break does not
make a deck already on disk readable.

**Region identity is card identity.** A masked region is a card. Whatever
addresses a region addresses a card's review history, so an identity choice that
looks like a detail decides whether editing a picture silently resets what the
learner has learned. This is the same constraint ADR 0003 records for cards and
ADR 0026 for self-describing ids, applied to a new addressable thing.

The full design, its rejected alternatives and six adversarial review passes are
in local working material. This record carries only the parts that must outlive
it, because those exist nowhere else that is tracked.

## Decision

### Regions are directive comments

A region is an HTML comment line beneath the media element it marks:

    ## Name the carpal bones
    ![](hand.png)
    <!-- blank: rect x=240 y=160 width=600 height=400 b:a1b2c3 -->

Regions must be invisible to other Markdown renderers, which rules out fenced
blocks (visible as code) and alt-text (breaks accessibility). Directive comments
are what the format already uses for `id:`, `reveal:` and `direction:`.

Three keywords, and no fourth is implied:

- **`blank:`** masks a region and asks about it. Named for the inline marker it
  matches, not for the industry word `occlude:`. A blank over text and a blank
  over a picture are one concept, so a second keyword would spend conceptual
  surface saying it twice. Two further reasons rule out `occlude:` specifically:
  it means the same thing as `cover:` below, so the pair would be two words for
  "hide" distinguished by nothing in the words, while the distinction that
  matters is asked versus never-asked; and it is false for text, where
  `blank: span` blanks a word in a sentence rather than occluding it.
  `occlude:` is therefore released rather than reserved: it stays an ordinary
  unknown directive name, because a name is reserved to keep it available for
  future meaning and this one will never have meaning here.
- **`cover:`** masks a region and never asks. It exists because the absence of an
  answer was overloaded: an unlabelled region has no natural text and still
  wants asking, while a legend that gives answers away wants hiding and never
  asking. A cover is scoped to the media element rather than to a card, takes no
  group name, carries no stamp, and never reveals, because its content gives
  away other cards on the same picture and that reason does not expire when one
  card is answered.
- **`crop:`** is a viewport onto the media element, so one large source serves
  many cards without being cut into files. Region coordinates remain absolute in
  full-source space, never crop space, so adjusting a crop does not invalidate
  every region on the picture.

### Binding is positional, and shape-specific

**Geometric and temporal shapes** (`rect`, `ellipse`, `polygon`, `clip`) bind to
the nearest preceding media element **on the same side of the card**, never
across the `---` divider. Having no preceding media element on their own side is
a parse error. The rule is stated over media rather than images so that audio
and video inherit it unchanged.

**`span` binds to the card's answer block**, which is implicit because a card has
exactly one, and requires no media element. A stored text blank is a `span`
living in answer text, so a single media-binding law would make every text blank
a parse error. `cover: span` is likewise scoped to the answer block rather than
to a media element, hiding its span for every card in that passage.

An explicit `image="hand.png"` key was rejected: the same source appearing twice
on one card stays ambiguous under it, while adjacency resolves that for free.

### Coordinates are named fields with SVG names

    rect     x=240 y=160 width=600 height=400
    ellipse  cx=500 cy=300 rx=100 ry=60
    polygon  points=10,10 90,10 50,90
    span     hidden="der" word=2
    clip     from= to=

Named fields make order irrelevant, make a transposed pair impossible rather
than silent, and extend to a new shape by adding names instead of changing the
grammar. Shape and attribute names follow SVG where SVG has the shape, full
words where it does not, so `rect` keeps SVG's own asymmetry with `ellipse`
rather than inventing a consistent-looking spelling.

**Time is a dimension carried by keys, never by a shape word.** A video region
is a rectangle *and* a time range, so a medium-bearing shape word would need
`videorect` beside `rect` and would multiply with every medium. `rect ... from=
to=` covers it with no new shape.

**One key carries what is under the mask: `hidden="..."`.** It is the locator on
a `span` (which is found *by* its text, so the covered string is simultaneously
anchor and answer) and the expected answer on a geometric shape. It is required
on `span`, optional on a geometric `blank:`, and retained but inert on a
geometric `cover:` so that an authoring tool toggling ask-versus-hide never
destroys the author's text.

**A `span` carries `hidden=` and exactly one of `word=` or `char=`**, a 1-based
positive canonical integer. Neither key is a parse error, both together is a
parse error, and `0`, a negative and a decimal are each rejected. Without this,
an edit moving a span between a whole word and a sub-word leaves one
implementation choosing `word=`, another `char=`, and a third discarding both
and searching.

**The v1 accepted shape set is `rect` and `span`.** `ellipse`, `polygon` and
`clip` are reserved words the parser rejects until they are built. `clip` is
additionally reserved because its keys and units are not yet ruled, and a shape
with an undecided key set cannot sit inside a closed grammar.

### Units are per media element

Bare numbers are pixels in the source's own coordinate space, because that is
what a paint tool hands the author. A `%` suffix opts a field into percentages
of the full source.

**Every geometric region and the `crop:` on one media element carry the same
unit, or the deck is rejected.** Not merely per region: a pixel crop beside a
percentage blank cannot be compared without the source's pixel dimensions, which
the parser never has, and containment against the crop is a parse-time rule.

This is deliberately the strict direction. On a frozen format, relaxing a rule
later keeps every deck ever written valid, while tightening later breaks decks
already on disk, so under uncertainty the strict rule is the recoverable one.

### The numeric and bounds domain

Enforced by the parser, which needs nothing but the line:

1. Digits with an optional decimal fraction. No exponent, no leading `+`, no
   separators, which makes `NaN` and `inf` unparseable rather than special.
2. No negatives, on any shape.
3. Every size component is strictly positive. The keys this binds today are
   `width`, `height`, `rx` and `ry`; a future shape inherits the law rather than
   waiting for the list to be extended.
4. Percentages run 0 to 100, sizes strictly above 0.
5. An asked region must keep **positive area in the visible viewport**, which is
   the crop where there is one and the media otherwise. "Wholly outside" means
   no positive-area intersection, so touching at an edge is outside. A `blank:`
   failing this is a parse error, including when only one member of a group
   fails, because the learner cannot see what they are asked. Partial overlap is
   legal and clipped. A `cover:` is ignored when empty, since it creates no card.

Enforced by the client, the only layer holding the source:

6. A region extending past the source edge is clipped, not an error. No layer
   that can see the file is positioned to reject a deck.

Bounds against the *source* can never be a parse error, because the deck only
references a file; bounds against the *crop* can, because the unit rule
guarantees both are authored numbers in one space. Percentage geometry is
therefore decidable by the parser, since 0 to 100 bounds the source. Pixel
geometry that clips to nothing is not, and becomes a loud load or render failure
at the layer holding the media. That is the single exception to rule 6's
clip-never-fail, and it exists because a card asking about nothing visible is a
broken question rather than a rendering nicety.

### Key sets are closed and unknown keys are hard errors

Each directive's key set is closed. An unknown, duplicate or inapplicable key is
a parse error, never a lint and never silently ignored.

|                 | `blank:`                               | `cover:`                                | `crop:`          |
| --------------- | -------------------------------------- | --------------------------------------- | ---------------- |
| shape word      | `rect`, `span` (rest reserved)         | same                                    | `rect` only      |
| geometry        | per shape, SVG names                   | same                                    | `x y width height` |
| `[name]` group  | yes                                    | no                                      | no               |
| `hidden="…"`    | required on `span`, optional elsewhere | required on `span`, inert elsewhere     | no               |
| `b:<stamp>`     | yes                                    | no                                      | no               |
| `from=`/`to=`   | yes, on video                          | yes, on video                           | no               |
| unit            | one per media element                  | same                                    | same             |

The strictness follows ADR 0026, which already makes an unknown key inside the
`at:` locator a hard error. It does **not** inherit that record's canonical
ordering: named fields are order-irrelevant here. A crop carries no `hidden=`,
no group name and no stamp because a viewport has nothing under it to answer, no
card to join and no history to own; the stamp exclusion is load-bearing rather
than tidy, for the reason in the identity section below.

`crop:` additionally allows at most one per media element (a second is a parse
error, since two viewports imply a layout language this format does not have)
and no `from=`/`to=` (a crop with time keys would move the frame mid-card, which
is a pan or a zoom, and that is video editing).

### Quoted values escape with backslash, strictly

A value is delimited by `"` and is single-line. The reader unescapes `\"`, `\\`
and `\>`, and any other `\x` is a parse error. The writer escapes `"` as `\"`,
`\` as `\\`, and a `>` that a `--` precedes as `\>`.

Alix itself writes these values, and `\blank{"Ja"}` is legal today, so
forbidding quotes would leave alix unable to represent a legal deck. Backslash
is chosen because the format already escapes with backslash in this exact role
(`\##`, `\>`, `\---`, `\{`). Unknown escapes are rejected to keep `\n` available
if values ever stop being single-line.

**Two sequences are dangerous, and a lone `>` is not.** An HTML comment ends at
the first `-->`, and the tokenizer's comment-end-bang state also ends one when
`>` follows `--!`. A value containing either would close the comment early in
every other Markdown renderer and spill the rest of the machine line as visible
page text, breaking the format's promise that regions are invisible. So the
writer escapes the `>` after `--` and after `--!`, and leaves every other `>`
bare. Both terminators matter in practice: a deck teaching HTML can blank this
literal syntax.

### Identity: per-region stamps, derived group ids

**Every region carries its own minted stamp**, written as `b:<stamp>` to match
the existing `key:value` shape for values alix minted, while author-written
coordinates stay `key=value`. A single region's card id is the literal
`card-<token>-b<stamp>`.

**A stamp is exactly six characters from the frozen lowercase Crockford set**
`0123456789abcdefghjkmnpqrstvwxyz`, which excludes `i`, `l`, `o` and `u`. A
wrong length, an uppercase letter or an excluded letter is a parse error. Strict
rather than "lowercase alphanumeric" because these are machine-minted
identities and the minter can only emit this set, so a wider grammar would
accept ids alix cannot produce.

**Uniqueness is scoped to the parent card.** The id proves the scope rather than
taste choosing it: the parent card's token prefixes the stamp, so identical
stamps collide only within one parent card and are harmless across different
ones. A group-only check would silently fuse two ungrouped regions under one
parent; a deck-wide check would re-mint a safely copied stamp and reset that
card's identity for nothing.

**A group's card id is DERIVED from the set of its members' stamps** and is
never written in the file:

    card-<token>-g<13>

Frozen derivation, in order:

1. Take each member's stamp.
2. Sort them in byte order.
3. Join with `-`.
4. Hash with XxHash64, seed 0 (the `hash64` the codebase already uses).
5. Render as 13 Crockford base32 characters, most-significant-first, reusing the
   existing token rendering convention.

Known vector, computed independently twice: `"a1b2c3-d4e5f6"` hashes to
`0xc8e57f0916150c1d` and renders as `chsbz14b1a30x`. Sorting is load-bearing:
the reversed preimage gives `0x63bc3f0d3cc8e88f`, a different id.

Two members of one parent card may never carry the same stamp; a stamper
re-mints on collision, so copy-pasting a region line cannot silently fuse two
members. This is also why a `crop:` may not carry a stamp: a stamp on the same
media element is one careless "collect this source's stamps" implementation away
from changing a card's id.

**Why derived rather than written or shared.** A written group id would put
identity outside the grammar. A shared stamp across members (an earlier ruling,
reversed) makes a group one card but cannot see a membership change, so adding
or removing a region silently keeps history that no longer describes the
question. Deriving from the set means membership *is* the identity: change the
members and the id changes, which is the honest outcome, and ADR 0032's
fresh-on-merge behaviour falls out by construction instead of as a special rule.

**An unstamped region is legal author input, not a canonical stored region.** A
stamping pass mints into it. This is forced rather than preferred: the stamper
parses a deck before it computes or writes any mint, so rejecting unstamped
input would reject the only input the stamper exists to fix, and every ordinary
creation path (a drawing tool, a copied example, a hand-written region) starts
unstamped. A region left unreconciled has no usable card id and is excluded from
the session rather than reviving positional identity.

## Consequences

A deck gains addressable picture regions using the concepts it already has:
directive comments, group names, minted stamps. No new file, no sidecar, no
second identity model.

Editing a picture's regions no longer silently resets progress, because identity
is minted rather than derived from content. Changing a group's membership *does*
change that group's id, deliberately: it is a different question.

Every client that reviews cards must draw masks, distinguish the regions a card
asks from other cards' masks and from covers, and reveal on answer. There is no
client-capability negotiation: a client renders what the library emits, and no
release may ship a client that receives a gradable card it cannot draw.

The parser gains a closed-key named-field grammar and a numeric domain it did
not have. Unknown keys become hard errors, which is stricter than the lint that
unknown *directive names* receive.

## Alternatives considered

**A separate `occlude:` keyword.** Rejected: a blank over text and a blank over a
picture are one concept, and a second keyword spends conceptual surface to say
so twice.

**Positional coordinates (`x,y wxh`).** Rejected: it changes separator
mid-expression, uses `x` as both a letter and an axis, and cannot express a
polygon at all.

**Two keys, `text=` for a span's locator and `answer=` for a region's expected
answer.** Rejected as complementary distribution rather than two concepts. The
cost is that covering one string while grading another becomes inexpressible;
nothing is lost today, since grading is exact match after normalization.

**A written group id, and a stamp shared by group members.** Both rejected under
identity, above.

**An explicit `image=` binding key.** Rejected under binding, above.

## Compatibility

Pre-1.0, so no migration path and no old-format recognition. Decks predating
this grammar contain no region directives at all, so nothing needs converting.

Taking `blank:` means the parser must stop treating a `blank:` directive as
unknown. `occlude:` keeps behaving as an ordinary unknown directive name, which
is a decision rather than an omission: it is released, not reserved.

The review data transfer object grows a region list, which is a documented API
contract change and a contract snapshot change, and every client moves with it.

## Security

No trust boundary changes. Regions are parsed from deck files, which are already
untrusted input parsed by this parser, and the numeric domain narrows what is
accepted rather than widening it. Quoted values are escaped so that a value can
never terminate its own comment and spill author-controlled text into rendered
output. No new file is written and nothing crosses a process or network boundary
that did not before.

## Verification

- Parser tests per directive asserting the exact accepted key set, and rejecting
  an unknown, a duplicate and an inapplicable key for each.
- A test that a reserved shape word (`ellipse`, `polygon`, `clip`) is rejected.
- A test that a `span` binds without any media element, and that a geometric
  shape without one is rejected.
- A `span` key matrix: `word=` alone accepted, `char=` alone accepted, neither
  rejected, both rejected, and `0`, a negative and a decimal rejected.
- A numeric-domain sweep: exponent, leading `+`, negative, each zero size
  component, out-of-range percentage.
- A unit test that a crop and a region in different units are rejected on one
  media element.
- Bounds tests: a region clipping to zero visible area rejected (including with
  no crop present, e.g. `x=100% width=10%`), a region touching a crop edge
  rejected, partial overlap accepted, an empty cover ignored.
- Escaping round-trip tests, including `\blank{"Ja"}` becoming `hidden="\"Ja\""`,
  an unknown escape rejected, and values containing `-->` and `--!>` neither
  truncating the directive nor rendering as visible text.
- A stamp-grammar test: six characters accepted, five rejected, uppercase
  rejected, and each of `i`, `l`, `o`, `u` rejected.
- A uniqueness test that two regions on one parent card cannot share a stamp
  while two regions on different parent cards may.
- Identity tests: every group member has a distinct stamp; the group id is
  invariant under member order; adding or removing any member changes it; a
  single region keeps its literal `b` form; and the known vector above is pinned
  as a literal.
- A stamping test that an unstamped region is minted into on the first pass, and
  that an unreconciled card has no usable id and is excluded from the session.

## Reversal

Evidence that would justify replacing this: an authoring case that a closed key
set or the per-media-element unit rule cannot express, which would widen the
grammar rather than replace the record; or a demonstrated need for two regions
on one source in different units. Widening is safe because it keeps existing
decks valid.

The derivation itself is not reversible in the same way. Changing the hash, the
seed, the separator, the sort or the rendering changes every group card id and
severs every group's review history, so a replacement would have to accept that
loss explicitly rather than treat it as an implementation detail.
