# 0060: A retried phone card gets the kept card

- Status: Accepted
- Recorded: 2026-10-31
- Retrospective: No
- Evidence: fn mint_or_return_kept_card in src/store.rs
- Evidence: fn a_retried_mint_returns_the_kept_card_only_when_front_and_back_are_identical in src/store.rs
- Evidence: an identical retry (its reply was lost) gets the kept card in tests/api.rs
- Evidence: final class AddCardNoAnswer in mobile/alix/lib/server_client.dart
- Amends: 0057 (clause 1, for an identical retry)

## Context

Under 0057 the desktop mints a phone-drafted card and answers with its id
and block; that reply is the only way the card reaches the phone. When the
reply is lost (the network drops after the desktop appended the card), the
phone showed the same line it shows for a refusal and kept the draft open.
Tapping Add again sent the same card, which the desktop's dedup refused
with 422. The desktop then held the card and the phone did not, and nothing
later reconciles the two.

## Decision

1. `POST /api/remote/ask/card` answers an identical retry with the kept
   card: when the create is a duplicate and the deck's personal sidecar
   holds the request's trimmed front and back stored exactly as the plain
   card a create writes (`## front`, the back lines, the id line, nothing
   else), the answer is 200 with that card's id and those exact bytes, and
   nothing is appended. Any other duplicate stays 422: one of an authored
   deck card (checked first, even when the sidecar also holds the card),
   the same content in different text, or a sidecar card carrying anything
   more, such as a directive. Only this
   route does it; a local create keeps refusing every duplicate.
2. The phone tells three outcomes apart: the desktop kept the card, the
   desktop answered with a refusal, or no answer arrived. On no answer it
   says the card may already be added and that Add retries.
3. The draft's fields lock when Add is pressed and unlock only on a
   refusal (or Cancel), so a retry after no answer is byte-identical and
   reaches clause 1.

## Consequences

The ordinary retry after a lost reply leaves both devices with the same
card under the same id. The check is on content, so it survives a desktop
restart. Still diverging: Cancel after a lost reply leaves the card on the
desktop only. A deliberate second Add of a card already on both devices
now succeeds on the desktop and fails on the phone's own dedup, which
reads as a refusal there.

## Alternatives considered

- Match on the content fingerprint (markup and whitespace normalized): an
  edit that only changed markup would get the old card back, dropping the
  edit without a word.
- A request key the phone sends and the desktop remembers: new wire field
  and server state that a restart loses unless persisted; with the fields
  locked, the identical-content check already identifies the retry.
- Make Add a polled job like draft and note: a route change and slot
  handling on both sides for the same outcome.

## Compatibility

Pre-1.0: the 422 for an identical retry becomes a 200 with the same
response shape; no persisted format changes.

## Security

The retry answer returns only the id and block of a card whose content the
caller already sent; the 422 it replaces already confirmed that such a card
exists. Nothing new is written.

## Verification

The lib test mints a card, retries it identically (same id and block, one
card in the sidecar), retries the same fingerprint in different text (422)
and a deck card's content (422). The route test does the same over HTTP.
Phone tests cover the three reply kinds over a real socket (a dropped
connection is no answer), the lock on no answer, and the unlock on a
refusal.

## Reversal

If a later design gives Add a delivery guarantee (a request key or a
polled job), clause 1 can return to refusing every duplicate.
