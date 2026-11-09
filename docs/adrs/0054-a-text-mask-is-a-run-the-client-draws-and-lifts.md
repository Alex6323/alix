# 0054: A text mask is a run the client draws and lifts

- Status: Proposed
- Needs rewrite: later rulings on text masks (sibling blanks shown, masks
  lifted client-side, `front_steps` retired) change this design; do not
  build from this text.
- Recorded: 2026-10-20
- Retrospective: No
- Amends: [ADR 0052](0052-a-blank-binds-to-the-block-above-it.md) rule 5

## Context

A `cover:` directive hides part of a card and asks nothing (ADR 0034). A
`blank:` hides part of a card and asks for it. Both bind to the block
directly above them (ADR 0052 rule 1). On an image, the server sends the
whole picture and a list of mask regions; each region carries a
`reveal_on_answer` flag, and the client draws the masks and lifts the
flagged ones when the answer shows.

Text works differently. The library bakes text masks into the question
string before any client sees it: the asked blank becomes `⍰`, every other
hidden span `⬚`, and the hidden text is gone (ADR 0052 rule 5). A blank
card reveals through `front_steps`, server-built copies of the question
with the card's own blanks filled in (ADR 0052 rule 4).

A plain card (one with no `blank:`) has no `front_steps`, so nothing could
lift a text mask on it. Its covers are bound and minted but never applied:
a `cover: span` on a plain card's question shows its hidden text before
Reveal (reproduced in `alix list` on 99255570; the card view projects the
same unmasked `front`).

## Decision

**1. A text mask travels as its own inline run.** An `InlineRun` gains
`mask: {role, reveal_on_answer}`, with `role` one of `"asked"`, `"mask"`,
`"cover"` and the same meaning as on an image region. The run's `text` is
the hidden text when the client lifts the mask itself (rule 3), and empty
otherwise. The wire carries no character offsets.

**2. Another card's answer never reaches the client.** A sibling blank's
mask (`role: "mask"`) and a cover on a blank card carry empty `text`:
their hidden text is another card's answer, or could give one away, and no
client ever shows it.

**3. A plain card's question covers lift on Reveal.** On a card without
`front_steps`, a cover in the question arrives with `reveal_on_answer:
true` and its hidden text, and the client lifts it when the answer shows.
A blank card keeps revealing through `front_steps` (ADR 0052 rule 4); the
masks in its runs show where the gaps are.

**4. A mask gives no length hint, before or after Reveal.** Clients draw
every mask at one fixed width. A lifted mask shows its text centered at
that width; a text wider than it takes the next coarse width step, so the
revealed width still does not measure the answer. The same holds for a
filled blank in `front_steps`.

**5. Plain-text fields stay masked.** `front`, `alix list`, browse, and
every prompt built from a card's question show `⍰` and `⬚` and never the
hidden text, on plain cards as on blank cards. A run sequence with masks
therefore no longer concatenates to `front`.

**6. Masks inside math and code stay drawn by the server.** A span inside
inline or display math is rendered into the formula's SVG, and a span
inside a code fence stays a marker in the fence's lines, as today. Such a
cover on a plain card hides before and after Reveal.

### Amends ADR 0052

Rule 5 ("the pre-reveal view carries a marker, never the hidden text or
its length") now holds for every plain-text field and for masks the client
does not lift; a cover a plain card's client lifts carries its text
(rule 3). The fixed-width rule extends to the revealed state (rule 4).

## Consequences

Easier: a cover hides on a plain card's question, which today it does
not; one mask vocabulary for images and text; no text offsets to keep
consistent across Rust bytes and Dart or JavaScript UTF-16.

Harder, and accepted:

- The hidden text of a liftable cover is on the wire before Reveal, as an
  image's covered pixels already are. A user reading the response can see
  it. A cover is a study aid, not a secret.
- The client contract changes: `InlineRun.mask`, the empty-text rule, and
  the lift rule. Both clients, the mobile bridge's mirrored structs, the
  contract snapshots and `docs/API.md` change together.
- Two reveal mechanisms remain: blank cards by `front_steps`, plain cards
  by lifting. Clients choose by whether `front_steps` is empty, never by
  card kind.

Deliberately unsupported, named for the roadmap:

- a liftable cover inside math, a code fence, a task list, or a table on a
  plain card's question (it stays hidden after Reveal);
- a revealed text wider than the line, which a fixed-width inline mask
  cannot wrap.

## Alternatives considered

**Plain text plus offset ranges.** Every client must agree on a unit
(UTF-8 bytes, UTF-16 units, graphemes), and an off-by-one shows the
wrong letters. Rejected by ruling (2026-10-20) for the run.

**Send every mask's hidden text, sibling blanks included.** The literal
reading of "the server sends the full text". It ships other cards'
answers that no client ever displays. Rejected.

**Lift every mask on the client and drop `front_steps`.** One reveal
mechanism, but masks inside math SVG and code fences cannot be lifted by
a client, and line-by-line reveal needs a step order per mask. Rejected
for now.

**Server-built revealed question for plain cards** (a `front_steps` of one
step). Keeps one mechanism, but contradicts the ruling that clients draw
and lift masks as they do on images. Rejected by ruling (2026-10-15).

## Security

The hidden text of a liftable cover moves from never-sent to sent before
Reveal. It is the card's own content and visible to the same user after
Reveal. No other card's answer is sent (rule 2).

## Verification

To be pinned by tests before acceptance:

- for every mask role on every card kind, the run's `text` is empty
  exactly when rule 2 says so;
- a plain card's question cover arrives with `reveal_on_answer: true` and
  its text; `front`, `alix list` and prompts carry `⬚`;
- both clients draw a mask at fixed width and lift a plain card's cover on
  Reveal, keeping the width or taking the next step.

## Reversal

Revisit if users need masks that lift inside math or code, or if keeping
two reveal mechanisms causes client bugs. Reversal is a new record.
