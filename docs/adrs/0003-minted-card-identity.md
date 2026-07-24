# 0003: Minted card identity

- Status: Accepted
- Recorded: 2026-07-24
- Retrospective: Yes

## Decision history

The current identity model shipped on 2026-07-19. Commit `83cc6b9` froze the
token grammar, `1063905` added insertion-only stamping, `a6e6d63` made tokens
authoritative in the Markdown parser, and `7f32ead` removed the previous
numeric identity. Commit `907f1c3` made wholesale identity replacement an
explicit destructive operation guarded by `--force`.

Earlier Markdown-format design explored content-derived identities. The
implementation deliberately pivoted away from that model before release.

## Context

Review schedules and generated sidecar data must continue to identify the same
card when a learner edits wording, moves a card, renames a deck, or reorganizes
a workspace. Identity derived from mutable content makes ordinary authoring
look like deletion plus creation and silently disconnects progress.

Cloze cards and reversible cards also create several schedulable views from
one authored card block. Their derived identities must be stable without
duplicating machine stamps throughout the source.

## Decision

Each authored card block has one opaque token stored in an
`<!-- id: ... -->` directive. Alix mints canonical tokens from 128 random bits,
encoded as 26 lowercase Crockford-base32 characters. The stored token remains
authoritative across content edits and file moves.

One stamped token produces schedulable sub-identities:

- a normal card uses the token itself;
- cloze hole `n` uses `<token>-<n>`;
- the reverse direction uses `<token>-r`.

Stable identity is distinct from the content fingerprint. Fingerprints may
detect stale augmentation output and help realign changed cloze holes, but a
fingerprint never replaces the authoritative token.

Normal stamping is insertion-only and idempotent. It mints missing deck and
card tokens without changing existing tokens. Duplicate tokens are detected
loudly. Replacing one token is explicit; wholesale replacement, which
disconnects progress, requires `--force`.

## Consequences

- Ordinary editing and renaming preserve review history.
- Deck files contain small machine-maintained directives.
- Token grammar and sub-identity framing are frozen compatibility surfaces.
- Copying a stamped card also copies its identity, so duplicate detection is
  required.
- Content changes can invalidate derived artifacts without resetting the
  learner's schedule.

## Alternatives considered

### Content hashes

Hashes deduplicate identical text but change whenever wording changes. They
conflate identity with staleness and would make routine editing reset progress.

### File path and heading position

Paths and positions are convenient lookup keys but change during normal deck
organization and insertion.

### Answer-derived identity

Answers are authored content and are frequently refined. Cloze and reverse
forms also make answer-based framing ambiguous.

### Central identity database

A database could map mutable content to stable IDs, but it would make a deck's
identity depend on state outside the portable file and conflict with ADR 0001.

## Compatibility

Tokens in deck directives, the accepted lowercase-alphanumeric token charset,
and `-r` and decimal cloze suffixes are persisted compatibility surfaces.
Canonical minting may stay narrower than parsing, because hand-authored and
third-party lowercase-alphanumeric tokens are accepted.

An identity migration must remap progress and every token-keyed sidecar before
rewriting a deck.

## Security

Tokens are identifiers, not secrets or authorization credentials. Their
randomness prevents accidental collision; it must not be treated as access
control.

## Verification

- `src/token.rs` freezes minting, validation, sub-ID composition, and parsing.
- `src/stamp.rs` tests insertion-only, byte-preserving, atomic, and idempotent
  stamping.
- `src/parser/canonical.rs` and `src/parser/cloze.rs` assign tokens and derived
  IDs to parsed cards.
- `src/card.rs` separates identity from content fingerprints.
- Doctor and deduplication tests report duplicate or malformed identities.

## Reversal

Replacing this model requires evidence that stored tokens cannot meet a real
identity requirement, plus a collision-safe mapping for every deck, progress
entry, virtual card, and sidecar. The migration must be backed up, reversible,
and explicit to the learner.
