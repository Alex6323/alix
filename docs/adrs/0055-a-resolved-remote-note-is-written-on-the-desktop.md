# 0055: A resolved remote note is written on the desktop

- Status: Accepted
- Amended by: 0057 (clause 4 for card creates on a served deck: a
  phone-drafted card is minted into the desktop's sidecar)
- Recorded: 2026-10-25
- Retrospective: No
- Evidence: fn a_grounded_remote_note_lands_in_the_desktop_sidecar_and_a_card_only_one_writes_nothing in tests/api.rs
- Evidence: fn remote_endpoints_never_write_the_server_store in tests/api.rs
- Amends: 0042 (clause 3, for tutor notes only)

## Context

ADR 0042 rule 3 makes content flow one way, desktop to phone: a pull mirrors
every path the desktop sent over the phone's copy. A paired phone's tutor
condenses a note through `POST /api/remote/ask/note`, and under ADR 0010's
iron rule the server only handed the lines back; the phone appended them to
its own copy of the deck's personal sidecar. Sync pushes progress documents
only, so that sidecar never reached the desktop, and the next pull of the
deck replaced the phone's sidecar with the desktop's, or removed it when the
desktop had none. A note made on the phone existed in no desktop file and
was lost at the next pull, silently.

## Decision

1. When a remote note's card resolves against the served catalog, the
   server appends the condensed lines to that deck's personal sidecar
   (`<deck>.local.md`) before handing them back, exactly as a local tutor
   note is written. The phone still applies the same lines to its own copy
   at once; the desktop's file is the source the next pull mirrors.
2. The write is bounded to that one file: never the authored deck, never
   the store, never a phone-supplied path. A card the desktop does not
   serve (`card_only`) writes nothing on the server.
3. A failed sidecar write returns as the call's `error` with no note, so
   the phone never shows a note the desktop did not keep.
4. Every other `/api/remote/*` endpoint keeps ADR 0010's rule: it writes
   nothing on the server.

## Consequences

A note made on either device survives the next pull. The one-way rule of
ADR 0042 holds for everything but tutor notes; the sidecar is content the
desktop now also receives from a remote call. A card drafted on the phone is
still minted on the phone only and has the same gap; that is a separate
decision.

## Alternatives considered

- Push sidecars from the phone in sync: a second content direction with a
  merge rule for a file both devices append to; far larger than the gap.
- Report the loss instead of fixing it: the note is model output the
  desktop already produced; keeping it costs one file append.

## Compatibility

No format or wire change: `RemoteAskDto` is unchanged, the sidecar grammar
is unchanged. Older phones gain the fix without an update.

## Security

A pairing-token holder can append model-condensed text, steered by the
history it sends, to a served deck's sidecar. The bearer token already
authorizes the remote tutor; the written file is the one a local tutor note
writes, and nothing else on the server becomes writable.

## Verification

The two evidence tests: the first sends a card-only note and a resolved note
through the fake backend and asserts no sidecar for the first and the
addressed block for the second; the second keeps every other remote call
write-free.

## Reversal

If the phone one day pushes its sidecar through sync with a merge rule, this
append becomes redundant and the clause can return to 0042's one-way shape.
