# 0021: Deck-owned frozen assets

- Status: Superseded by
  [ADR 0026](0026-self-describing-ids-and-named-locator-fields.md)
- Recorded: 2026-07-27
- Retrospective: No
- Supersedes:
  [ADR 0015](0015-frozen-source-snapshots.md)
- Refines:
  [ADR 0017](0017-per-deck-state-documents.md),
  [ADR 0019](0019-workspace-artifact-layout.md), and
  [ADR 0020](0020-source-excerpt-integrity.md)
- Refined by:
  [ADR 0023](0023-semantic-workspace-update.md)

## Context

ADR 0015 preserves generated workspace evidence by copying cited excerpts into
a shared `assets/` directory after generation. ADR 0020 preserves live source
for development decks while detecting locator drift. That boundary leaves an
initialized workspace dependent on live source until a later generation or
publishing step. The source may no longer exist when that step occurs.

The shared flat asset directory also lacks an ownership rule. A tool cannot
move, share, replace, or remove one deck's assets without scanning the complete
workspace reference graph. By contrast, ADR 0017 gives progress and
augmentation one document per stable deck ID.

Source evidence is part of the portable learning artifact. Its availability
and ownership must not depend on when a workspace is eventually published or
on inference across unrelated decks.

## Decision

Every initialized source-backed workspace member uses frozen evidence as its
runtime source. Freezing occurs at the explicit write boundary that creates or
initializes the member, before the operation reports success. Review,
discovery, tutor opening, sharing, and ordinary doctor remain read-only.

Each stable deck ID owns one asset directory:

```text
assets/<deck-id>/
```

Every source excerpt, image, or other file Alix ingests, copies, freezes, or
generates for that deck is stored below this directory. A deck does not depend
on another deck ID's asset directory. Identical bytes used by several decks
are copied into each owning directory rather than shared.

Alix-managed deck assets use an exact-byte content address:

```text
sha256-<digest>.<extension>
```

The lowercase SHA-256 digest covers the exact stored bytes. The prefix names
the algorithm and the extension remains a media or syntax hint. The digest is
an immutable object address, not an authenticity signature.

Citation integrity remains a separate layer. The `xxh64` citation fingerprint
from ADR 0020 covers normalized displayed text so an unchanged excerpt can
move. A frozen citation retains both its content-addressed asset path and its
live provenance:

```markdown
---
source: assets/<deck-id>/sha256-<digest-a>.rs + sha256-<digest-b>.md
origin: /path/to/live/source
---

<!-- at: sha256-<digest-a>.rs @ xxh64:<fingerprint> from src/lib.rs:20-31 -->
```

The first source path is workspace-relative and subsequent ` + ` paths are
relative to its directory. An explicitly named regular text file is frozen in
full because the author already selected it as bounded exam ground truth. A
directory source is reduced to its cited excerpts and must contain at least one
citation before initialization succeeds. The resulting files form the source
used for offline exam generation and grading, not a recursive copy of the live
origin. Images remain workspace-relative references under the same deck-owned
directory but are not automatically exam evidence.

The frozen asset is authoritative evidence for review, grading, and offline
clients. The tutor always receives that exact evidence. When `origin` is a
usable local path or URL, the tutor also reads it for broader current context
and reports contradictions as possible card staleness instead of silently
substituting current bytes. Without usable origin context, tutoring continues
from the frozen evidence and the client shows that limitation. `from` preserves
the authored location of an excerpt.

An update is semantic, not a snapshot refresh. ADR 0023 defines
`alix workspace update` as a staged proposal of card changes and new frozen
evidence. It preserves valid identities, writes new assets before references,
atomically replaces deck files, and leaves old assets in place until the
complete update succeeds. It does not silently refresh evidence while
retaining an unreviewed card.

Direct files such as a workspace icon may remain workspace-owned under
`assets/`. Deck-owned paths are reserved for the member named by their deck ID.
Sharing one deck includes its complete asset directory and matching
augmentation, while private progress is included only in an explicit private
move.

## Consequences

- A source-backed workspace is self-contained as soon as it is initialized.
- Its exam corpus is bounded by explicit files and reviewed directory excerpts
  rather than the complete live origin.
- Publishing and sharing no longer depend on the continued availability of
  live source.
- One deck can be moved or bundled without a workspace-wide reachability scan.
- Identical assets may be duplicated across deck directories.
- Asset paths become longer but stable and inspectable.
- Runtime evidence behavior becomes consistent across desktop, web, and mobile.
- Editing a managed asset creates a new address instead of mutating an existing
  object.
- Live development work moves from implicit source reads to an explicit update
  workflow.
- URL-backed portable evidence remains incomplete until remote capture has its
  own trust and policy design.

## Alternatives considered

### Retain generated-workspace-only freezing

This keeps development decks convenient but leaves a portability window in
which the evidence can disappear before publication. Publication is too late
to establish reproducibility.

### Keep a flat content-addressed store

A flat store deduplicates bytes but requires reachability analysis or reference
counts for single-deck moves and deletion. That permanent ownership complexity
is not justified by the size of normal excerpts and learning images.

### Prefix flat files with the deck ID

The prefix encodes ownership but keeps unrelated objects mixed together and
makes every filename harder to inspect. A directory is the native movable
filesystem boundary.

### Share assets between deck directories

Hard links, symbolic links, or a hidden object pool make a deck directory look
self-contained while retaining external dependencies. They also behave
differently across archives, synchronization tools, and mobile filesystems.

### Use citation fingerprints as asset addresses

Citation fingerprints normalize text and deliberately ignore some byte
changes. Asset addresses identify exact stored bytes and may cover binary
files. One hash cannot honestly represent both contracts.

### Copy the complete live source tree

This would preserve broad exam context but could export large, irrelevant,
licensed, or sensitive source that no card cites. Explicit files are already
bounded; directory trees are reduced to cited excerpts so the portable learning
artifact remains auditable and data-minimized.

### Refresh evidence automatically

An upstream edit can invalidate the question, answer, notes, distractors, and
augmentations. Replacing only the evidence would create a current-looking but
semantically stale card.

## Compatibility

This is an intentional pre-1.0 format and layout break. Production code
recognizes only deck-owned, content-addressed managed assets for initialized
source-backed workspace members. It contains no reader for flat numeric
snapshots and no fallback to live source.

Before release, existing workspaces are backed up and converted outside
production code. The conversion preserves all deck and card IDs, creates each
deck-owned asset directory, hashes exact stored bytes, rewrites citations and
images, and removes the abandoned flat artifacts after verification.

Progress and augmentation document addresses do not change because their deck
IDs do not change.

## Security

Freezing is a data-export boundary. Initialization may copy explicitly named
source files, cited excerpts from source directories, and referenced images. It
must reject paths outside the resolved source or workspace boundaries and
report every unreadable required file.

A deck share exposes all bytes below its owned asset directory. Ownership makes
that export auditable, but the user must still review sensitive source before
sharing.

Asset hashes detect accidental byte changes and make received objects
verifiable against their names. They do not authenticate the author, make
untrusted media safe, or prevent a malicious deck from naming malicious
content. Receivers continue to validate paths, media handling, and deck
identity.

New assets are written before deck references, deck files are replaced
atomically, and old referenced assets are not deleted during a failed update.
This ordering prevents partial updates from destroying the last usable
evidence.

## Verification

- Asset tests pin deck-owned path derivation, SHA-256 names, canonical excerpt
  bytes, exact source-file and image bytes, normalized extensions, reuse, and
  cross-deck duplication.
- Initialization and generation tests prove a member is not published until
  every required asset is staged and validated.
- Source-consumer tests prove initialized workspace members use frozen evidence,
  origin access remains separately gated, and the tutor warns when only frozen
  context is available.
- Doctor tests report live, missing, corrupted, misnamed, and cross-deck
  references without writing.
- Sharing tests prove one deck bundle contains its complete owned asset
  directory and matching augmentation but excludes progress and unrelated
  assets.
- The semantic-update design includes transaction tests proving
  assets precede references, failures preserve the old workspace, and evidence
  cannot refresh independently of card text.
- Contract and mobile tests pin the same frozen-source behavior in every
  client.
- A repository audit proves production contains no flat-snapshot reader or
  pre-1.0 conversion path.

## Reversal

Replace deck-owned frozen assets only if measured workspace sizes make
duplication materially harmful or another mechanism preserves exact evidence,
offline use, deterministic per-deck moves, bounded exports, and atomic semantic
updates with less complexity.

A replacement must define ownership and deletion without scanning unrelated
decks, preserve stable deck and card identity, and provide an external
pre-release conversion before production adopts the new structure.
