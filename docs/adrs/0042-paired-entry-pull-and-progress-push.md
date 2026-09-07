# 0042: A paired device pulls entries and pushes progress over the pairing

- Status: Accepted
- Evidence: sync_wire_shapes in src/serve/contract.rs
- Evidence: root_identity_mints_once_and_preserves_other_toml_keys in src/sync/server.rs
- Evidence: accepted_sync_push_is_the_base_of_the_next_desktop_grade in tests/api.rs
- Evidence: sync_push_handler_commits_once_on_the_study_owner_thread in src/serve/tests.rs
- Evidence: a_first_pull_lands_every_entry_shape_byte_for_byte in src/paired.rs
- Evidence: a_crash_at_any_step_leaves_the_old_or_the_new_entry in src/paired.rs
- Evidence: a_deleted_member_with_unpushed_progress_keeps_its_files_and_document in src/paired.rs
- Evidence: pulled_entries_count_the_phone_saves_since_the_last_push_and_their_time in src/paired.rs
- Evidence: SyncController in mobile/alix/test/sync_controller_test.dart
- Evidence: testWidgets in mobile/alix/integration_test/sync_e2e_test.dart
- Recorded: 2026-09-03; revised 2026-09-05 (identity per served folder);
  accepted 2026-09-07 with the implementation
- Retrospective: No

## Context

The Android client embeds the lean core and reviews offline, and since ADR
0010 it may borrow desktop AI over a LAN pairing whose bearer token guards
`/api/*`. Getting decks onto the phone is still manual (a shared folder or
the generator), and progress earned on the phone never reaches the desktop:
ADR 0010 fixed that the remote surface never writes the server store and the
phone owns its state. The 2026-07-28 sync spec chose Syncthing for
convergence, but its Android background-service proof is open, and if the
engine cannot stay alive outside the foreground, sync degrades to
sync-at-open, which a bounded pull/push over the pairing delivers without an
engine, a second app, or public discovery.

Alex ruled on 2026-09-03: phase 1 of 0.9 is that pull/push; one writer per
deck per sync cycle with a loud refusal; the unit is any picker entry
(workspace or loose deck) with content flowing one way and a manifest-scoped
mirror; Android only. The spec is
`docs/specs/2026-09-03-entry-pull-spec.md` (local).

## Decision

1. A new `/api/sync/*` family, behind the existing bearer guard: `entries`
   (what is pullable), `pull` (the share staging of one entry plus its
   progress documents and a manifest, as one zip), `push` (one progress
   document, carrying the root id it was pulled from and the revision it
   was pulled at; accepted only when the server serves that root and the
   desktop's revision equals the pulled one; otherwise 409 and nothing
   written). Share keeps its
   contract untouched: progress never travels in a share and landing never
   overwrites.
2. Progress ownership moves per deck per cycle: a pull hands the desktop's
   document to the phone with its revision; a push hands it back through the
   store's ordinary stale-revision check, so the desktop document advances
   exactly one revision per accepted push and carries the phone's writer
   marker. The push executes on the Study/Progress owner thread (ADR
   0027), so it is serialized with desktop saves by construction; the
   check itself has no lock. Between cycles the phone owns its state, as ADR 0010 says; this
   record supersedes 0010's clause that the phone never writes the desktop
   store, for this family only.
3. Content flows one way, desktop to phone. The phone applies a pull as a
   manifest-scoped mirror applied as one directory swap: every path the
   desktop sent replaces or removes its previous copy; anything else under
   the entry (a deck generated on the phone) is kept and reported; a member
   with unpushed phone progress is never removed.
4. Every served folder carries a stable root id, minted once, kept in the
   folder's own `.alix/sync.toml`, and exposed to paired clients; the phone
   keys its pulled directories and manifests by it, so token rotation, a
   folder move, and a repointed profile never orphan progress, and a
   second served folder never shares a progress directory with the first.
   The id is per folder, not per profile: a profile is a launch recipe and
   is never served. The id's grammar and config key are freeze-forever
   choices recorded in the spec's decision list for Alex, not chosen here.
5. The existing bearer token authorizes the family. Rationale: it already
   authorizes reading every deck and grading against the desktop store
   through the study endpoints; a revision-checked single-document push
   widens nothing in kind. The 2026-07-28 rule that the AI token must not
   authorize filesystem synchronization stands for Syncthing, whose
   convergence includes deletes and arbitrary files.

## Consequences

- The web JSON API grows one family; docs/API.md, the contract snapshots,
  and the CHANGELOG move with the code.
- The desktop store gains a second writer class (paired devices) with the
  same atomic write, revision, and writer-marker rules as the desktop's own
  saves. A conflict surfaces at two moments of truth and nowhere earlier:
  the phone's push (409 and an explicit choice that names what it
  discards; the loser is discarded whole, and an accepted push leaves the
  desktop's previous document as the deck's `.bak`, which `alix deck
  restore` swaps back until the next push) and the desktop's next save
  after a foreign write (`save_error`). No pre-warning on either side.
- The phone's shared-folder root is removed in the release that ships this
  design (Alex, 2026-09-06): the paired root replaces it,
  `MANAGE_EXTERNAL_STORAGE` leaves the manifest, and the phone's
  foreign-writer notice goes with its only user. Desktop-to-desktop folder
  sync is untouched. The bet is that pull and push cover the need; user
  requests reopen it.
- The phone stores and dials the scheme its pairing URL carried (spec
  decision 17, ruled 2026-09-06); alix itself does not terminate TLS in
  this phase. A reverse proxy with a CA certificate or a VPN mesh is the
  operator's route; alix-terminated TLS is a separate roadmap item.
- The phone gains a per-root directory and a sync report; no background
  service, no retry loop.
- The root id is a new persisted value in the served folder's
  `.alix/sync.toml` (`root_id`), written at serve start when absent;
  share already strips that file.
- Phase 2 (Syncthing) inherits the single-writer contract and the identity;
  it replaces the transport, not the rules.

## Alternatives considered

- Syncthing first (2026-07-28 spec): deferred behind the background-service
  proof; the pull/push is what the engine degrades to if the proof fails.
- Widening `/api/share`: rejected; share's promises to strangers must stay.
- Per-card merge instead of single-writer refusal: deferred as a named
  roadmap item; the scheduler's per-deck state has no ground truth to merge
  against.
- A separate sync credential: recorded by Alex as not chosen (spec
  decision 3); the bearer token already grants strictly more through the
  study endpoints.

## Compatibility

Pre-1.0: no compatibility machinery. The progress document format is
unchanged (`DECK_DOCUMENT_VERSION` stays 1); the manifest and the sync DTOs
are new documents that start at version 1. A phone that never pulled is
unaffected. The root id is a new file, `.alix/sync.toml`, in the served folder; a
folder without one mints it at the next serve.

## Security

- Trust boundary: the paired token already reaches every deck and the study
  endpoints' store writes; the family adds a bulk export and a single
  revision-checked document write. The byte policy is the spec's decision 14 (streamed pull, announced
  sizes, a push body cap); cardinality alone bounds nothing. The trust
  boundary is ADR 0010's: a trusted network or an operator-managed proxy,
  never internet-grade identity.
- The push validates the document like a local load (version, deck id,
  unknown fields) before the store's save path runs; a malformed body is
  400, a stale one is 409, and neither writes.
- Pull staging reuses share's sanitizer and then re-adds only what the same
  person owns: the entry's progress documents, personal sidecars, and
  `alix.local.toml`. A pull never carries the desktop's recent state, the
  root id file, backups, or conflict copies.
- `docs/security/README.md` gains the family's regression evidence when it
  lands.

## Verification

- `mod contract` pins for `VersionDto` (with `root_id`), `SyncEntriesDto`,
  `SyncPullManifest`, `SyncPushDto`, `SyncConflictDto`, `SyncRootDto`.
- API tests: pull byte-equality except the manifest; push accepted at equal
  revision and refused at a moved one with the desktop untouched; a second
  served folder's directory separate from the first; a moved folder keeps
  its pairing.
- The spec's falsification table, run against the implementation; the
  phone side's end-to-end run (real binary, real app: pull, review, summary
  push, conflict choice) lives in `mobile/alix/integration_test/`.

## Reversal

Remove the family and the phone's sync surface; pulled roots on the phone
remain ordinary decks with ordinary progress documents, reviewable offline.
Reversing phase 1 alone keeps `.alix/sync.toml`, which phase 2 consumes;
reversing both drops it, cleaned up by disposable tooling outside the
repository (pre-1.0, no migration code). ADR 0010's superseded clause returns to
force.
