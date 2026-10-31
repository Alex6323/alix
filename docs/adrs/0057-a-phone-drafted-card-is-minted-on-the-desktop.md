# 0057: A phone-drafted card is minted on the desktop

- Status: Accepted
- Recorded: 2026-10-25
- Retrospective: No
- Evidence: fn a_phone_drafted_card_is_minted_in_the_desktop_sidecar_and_handed_back_with_its_id in tests/api.rs
- Evidence: fn remote_endpoints_never_write_the_server_store in tests/api.rs
- Evidence: fn record_minted_card_keeps_the_desktop_id_and_dedups_against_the_deck_and_the_sidecar in mobile/alix/rust/src/api/review.rs
- Amends: 0055 (clause 4, for card creates on a served deck)
- Amended by: 0060 (clause 1, for an identical retry)

## Context

ADR 0055 closed the gap for tutor notes: a note condensed for a paired
phone is appended to the desktop's sidecar, so the next pull carries it
back instead of replacing the phone's sidecar without it. A card drafted on
the phone had the same gap and was left open there: the phone minted it
alone, into its own sidecar and store, and no `/api/remote/*` route told
the desktop. The next pull of that deck mirrored the desktop's sidecar over
the phone's and the card vanished, leaving its progress entry orphaned.

Pushing the phone's sidecar through sync was rejected again for the reason
0055 gave: the sidecar has no revision and two appenders, so a push needs a
merge rule, and it opens a second content direction against 0042 rule 3.

## Decision

1. The phone sends an accepted draft to `POST /api/remote/ask/card` with
   the deck id, front and back. When the deck resolves against the served
   catalog, the server validates and dedups exactly as a local create does,
   mints the id, appends the stamped block to that deck's personal sidecar
   (`<deck>.local.md`), and answers with the id and the exact block.
2. The phone appends that block unchanged to its own sidecar and inserts
   the id into its own store. The desktop's sidecar is the source the next
   pull mirrors; the id reaches the desktop store through the phone's
   ordinary revision-checked progress push (0042 clause 2).
3. The server never writes its store for this call: a store insert would
   move the revision the phone's next push was pulled at. The write is
   bounded to the one sidecar; never the authored deck, never a
   phone-supplied path.
4. A deck the desktop does not serve answers 404 and writes nothing; the
   phone then mints the card alone, as before (the `card_only` case). A
   refused create (duplicate, malformed, 404, unreachable) mints nothing on
   the phone: the phone never holds a card the desktop did not keep.
5. The create runs on the jobs owner thread, where the remote note append
   runs, so the two remote appenders to one sidecar are serialized by
   construction.

## Consequences

A card drafted on either device survives the next pull. ADR 0055's clause
4 (every other remote endpoint writes nothing) now excepts this route as
well. A desktop review session with that deck open does not see the new
card until the deck is reopened, as with remote notes. A failed create
reads as one line on the phone whether the desktop refused the content or
could not be reached; the client's remote error surface does not separate
the two.

## Alternatives considered

- Push sidecars from the phone in sync: a merge rule for a two-writer file
  and a second content direction (0042 rule 3); far larger than the gap.
- Mint on the phone and send the block to the desktop afterwards: the
  desktop would have to accept a phone-minted id and dedup after the fact,
  and a refused block would already be on the phone.
- Insert the id into the desktop store at create time: moves the desktop
  revision under the phone's pulled document, so its next push is refused.

## Compatibility

A new route and response (`RemoteCreateCardResp`, snapshot-pinned); the
sidecar grammar and the store are unchanged.

## Security

A pairing-token holder can append a card of its own wording to a served
deck's sidecar, bounded by the local create's validation (two non-empty
single lines, no duplicate of the deck or sidecar). The bearer token
already authorizes the remote tutor and the note append; nothing else on
the server becomes writable.

## Verification

The first evidence test posts a create for an unserved deck (404, no file),
a served deck (200, the block in the sidecar under the returned id), the
same content again (422, sidecar unchanged), authored content (422) and an
empty front (422). The second keeps the store byte-identical across a
successful create and every other remote call. The third records a block
on the phone core under the desktop's id and refuses the deck's and the
sidecar's duplicates.

## Reversal

If sync one day pushes the phone's sidecar with a merge rule, the route
becomes redundant and the phone can mint alone again.
