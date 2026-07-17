# Changelog (alix mobile)

The Android app's own stream, independent of the crate's `CHANGELOG.md`:
`mobile-vX.Y.Z` tags release it (see RELEASING.md). Keep a Changelog shape;
the release workflow lifts the tagged version's section into the GitHub
Release notes, so a release without its section fails loud.

## [Unreleased]

### Added

- **Pair with your desktop's alix for the tutor and the AI exam.** The
  overflow menu's "Pair with desktop…" sheet takes the URL `alix --lan`
  prints; a bad paste, an unreachable desktop, a too-old server, and a
  rejected token each show one inline line instead of failing silently.
  Once paired, review gains an Ask chip once you've attempted the current
  card, opening the same tutor conversation as the desktop (including
  "Make a card"), and the session summary gains a "Take the exam" chip for
  any deck with a `% source:`. Both borrow the desktop's AI backend over
  your LAN; the phone keeps its own decks and progress, the desktop only
  computes answers. The same menu item reopens the sheet with an Unpair
  button.
- The app now declares the INTERNET permission and allows cleartext HTTP,
  both only for talking to a paired desktop on your LAN.

### Fixed

- The session summary no longer shows all zeros after a first pass over a
  fresh deck: it now says how many new cards were introduced, and hides
  the passed/failed rows when nothing was graded.

## [0.1.1] - 2026-07-16

### Added

- The alix tutorial deck: a fresh install's app-private decks folder now
  starts with a small deck that teaches alix while you review it. Deleting
  it is the graduation; it only seeds into a brand-new folder, so it never
  comes back.

### Fixed

- **A trace deck no longer white-screens the review.** Trace decks
  (`% trace:`, guided source walks) live in the web app; the phone refused
  to open them but rendered the refusal as a blank screen. The picker now
  marks trace rows and explains on tap, and any deck the session cannot
  open shows the reason with a way back instead of going white.

## [0.1.0] - 2026-07-15

The first published build: the review loop on your own phone, against the
same core the desktop runs. Early software; expect rough edges.

### Added

- The full review loop with the embedded alix core: every check mode
  (flip, choice, typing, type-line, line-by-line, the Explain keypoint
  checklist), attempt-first acquisition for new cards, FSRS scheduling,
  and workspace-aware progress stores.
- A user-chosen shared decks folder (Android 11+, via All Files Access):
  point alix at a folder another tool already syncs (e.g. Syncthing) from
  the picker menu; decks and progress roam with the folder. App-private
  storage stays the default and the fallback.
- Roaming guards: a banner when another device wrote the progress store
  moments ago, and a loud warning when a sync conflict file sits next to
  it. One device at a time is the rule; these make slips visible.
- The alix look: the web app's dark palette (its default identity), the
  orange wordmark, IBM Plex faces, bordered deck rows, and the brand-orange
  primary action. A light palette exists; the app opens dark to match the
  web out of the box.
- The review surface mirrors the web client closely: a mono mode-tag, a
  bold question over a faded divider, monospace numbered multiple-choice
  options that lock and tint green/red on a pick, monospace answers, the
  keypoint checklist, a warm boxed note, and the web's chip legend (Missed
  it / Partly / Got it, Next, Continue, Knew it / Not yet, Reveal, Seen).
- About (in the picker menu): the app version and the embedded core
  version side by side.
- The alix launcher icon and app name.
