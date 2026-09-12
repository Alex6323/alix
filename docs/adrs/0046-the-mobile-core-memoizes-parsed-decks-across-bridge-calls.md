# 0046: The mobile core memoizes parsed decks across bridge calls

- Status: Accepted
- Evidence: static LISTING_CACHE in mobile/alix/rust/src/api/listing.rs
- Evidence: pub mod cache; in src/lib.rs
- Recorded: 2026-09-12
- Retrospective: No

## Context

The web server has held a `DeckCache` across requests since the picker was
served from one process: an entry is keyed by path, validated by the file's
(mtime, size), and a listing over unchanged decks parses nothing. The mobile
core had no such thing. Its bridge was stateless by construction, every call a
fresh function over the filesystem, and the cache module itself sat behind the
crate's `full` feature, which the mobile build does not enable.

The consequence was measured on a 2019 Android phone over a 222-deck
workspace: every listing parsed every deck, about 490 ms rested and about
2650 ms under ordinary memory pressure, on app open and on every return from
a review to the picker. On the same corpus the parser is 98 percent of a
listing's cost and file I/O under one percent, so nothing short of not parsing
moves the number.

## Decision

The mobile bridge holds one `DeckCache` for the life of the process, guarded
by a mutex, and every picker listing goes through it. `crate::cache` moves out
of the `full` feature into the lean core; the one helper it needed from the
full-gated `picker` module, `deck_label`, moves to `listing`, which is lean.

The cache is bounded by entry count, 1024 paths. A listing that leaves the
count above the bound drops the cache whole after it has answered; there is
no eviction order to reason about, and a corpus past the bound costs what it
cost before the cache, never more. The bound is set from the measured cost
per cached deck, about 26 KB of native heap on the device, so a full cache is
about 27 MB.

Invalidation is the existing (mtime, size) check, unchanged. A member's
workspace facts that its own file cannot see, the manifest's defaults and
whether the workspace declares a source, are part of the entry and invalidate
it when they change. The listing passes those facts in from the workspace it
has already read, so a cached listing reads the manifest once, not twice.

## Consequences

A repeat listing over unchanged decks costs the row derivation only, measured
at about 180 ms on the device against about 2650 ms before. The first listing
in a process is unchanged; this record does nothing for cold start.

The mobile core now carries cross-call state. Every bridge function that
reads a deck through the cache sees the same memoized parse as the picker, and
a test of the bridge must account for a cache that survives between calls in
one test binary.

`DecksLoaded` counts parses, not loop iterations. A listing served from the
cache reports zero, and the counter law that pins it can state "a second
listing over unchanged decks parses nothing" as an exact count.

The cache is process-global and outlives a change of listed root. A person who
switches profiles or roots keeps the old entries in memory until the bound
clears them or the process ends. A corpus larger than the bound gets no
benefit at all; at ten times the measured workspace this design degrades to
the stateless behaviour it replaced. Deliberately unsupported: per-root
caches, eviction by recency, and a cache that survives the process.

## Alternatives considered

**A derived per-deck index on disk** (ADR 0045, Rejected). Precomputes a row's
state into a sidecar so listing needs no parse, and would have removed the
cold-start cost too. Rejected for carrying due state, which depends on the
progress store a phone owns locally, and for adding a persisted format with a
staleness contract.

**Two-phase rows.** Paint rows from frontmatter alone, then fill the rest in
behind. Attacks cold start, which this record does not. Not chosen first
because the lock check takes a full `Deck` and full-loads each prerequisite,
and because the fields a cheap row can carry are not the ones that gate the
launcher buttons; a first paint would show titles and little else. Remains
open.

**Clearing the cache after a sync cycle.** Sync replaces a pulled entry's
whole directory by rename, so every file arrives with a new inode and
modification time, and the (mtime, size) check sees it. An explicit clear
would guard only a same-size write landing within the filesystem's timestamp
resolution. Not built; recorded as a residual.

**Stateless bridge, keep re-parsing.** The measured baseline. Rejected by the
numbers above.

## Compatibility

No persisted data. No wire format. The web JSON API is unchanged. The bridge
signatures are unchanged by this record; the companion change that makes the
listing calls asynchronous is a client-boundary change on the mobile side
only and is recorded in the book and CHANGELOG rather than here.

## Security

Parsed decks live in process memory for longer. They already did for the
duration of a listing; the exposure window grows to the process lifetime,
which is the same window the web server has accepted since it held a cache.
No new trust boundary.

## Verification

- A counter-law test in `src/listing.rs` asserts that two `list_root_with`
  calls over one unchanged workspace through one cache parse every deck once
  in total, and that `manifest_reads` stays at one.
- `make lean-check` builds the no-default-features library with warnings
  denied, which is the build shape that now includes `cache`.
- `make adr-check` holds the evidence strings above against their paths.

## Reversal

If a measurement on a real device shows the cache's memory dominating (the
measured rate times the bound exceeds what the platform grants the process),
lower the bound or replace whole-clear with an eviction order, and record it
here as a refinement. If cold start is later solved by a design that makes
listing cheap without a parse, this cache becomes redundant and this record
should be superseded, with the static and the mutex removed from the bridge.
