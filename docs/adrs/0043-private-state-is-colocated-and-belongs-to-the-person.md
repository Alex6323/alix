# 0043: Private state is colocated, and any relocation belongs to the person

- Status: Accepted
- Evidence: pub fn store_path in src/workspace.rs
- Recorded: 2026-09-06
- Retrospective: No
- Refines:
  [ADR 0019](0019-workspace-artifact-layout.md) and
  [ADR 0022](0022-workspace-and-user-file-ownership.md)

## Context

ADR 0022 separated workspace-owned files (manifest, decks, assets,
augmentation) from user-owned files (progress, recent history, the local
manifest) on the axis of ownership and shareability. It kept a `store`
key in the workspace manifest that relocates the user-owned files to a
directory of the manifest's choosing, inherited from the commit that
introduced per-workspace stores (2026-06-21) without a recorded reason.

The one documented use, a deck folder on a cloud drive with progress kept
private to each device, makes the same person review the same cards on
each device. The use a maintainer could imagine, several people sharing
one content folder with private progress each, cannot be expressed by a
path in a manifest they all share: every member resolves the same store.
No workspace known to use the key exists. The paired
device sync (ADR 0042) would have had to promise, test, and document an
entry whose progress lives outside the entry.

## Decision

A workspace's store is the workspace directory. The manifest carries no
`store` key; `Manifest` rejects unknown keys, so a manifest that still
names one fails as ordinary invalid input, reported by `alix doctor`, and
is never silently read with the key ignored. The CLI `--store` override
is unchanged.

The direction that replaces the key, recorded now and built only when a
feature needs it: relocation of private state belongs to the person, not
to the workspace. A profile-level store root (the persistent twin of
`--store`) is the shape a shared-content, private-progress feature takes.
Identity lives with the progress: the sync root id sits at the root of
the store, which today is the served folder because the store is
colocated, and moves with the progress if a profile-level store exists.

## Consequences

- A workspace is again one folder for content and private state; moving
  or backing it up moves both, as ADR 0019 describes.
- Keeping private files out of a folder shared with other people is a
  property of the synchronization tool, not of the manifest: a Syncthing
  ignore list for `progress/`, `recent.json`, `alix.local.toml`, and
  `*.json.tmp`. Cloud drives without per-file ignore cannot express it
  until the profile-level store exists.
- The paired device sync resolves every deck's store as the colocated
  one; no entry holds its progress outside itself.
- ADR 0022's clause that a workspace `store` override cannot relocate
  augmentation is vacuous from this record on; its ownership axis stands.
