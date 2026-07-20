# Changelog (alix mobile)

The Android app's own stream, independent of the crate's `CHANGELOG.md`:
`mobile-vX.Y.Z` tags release it (see RELEASING.md). Keep a Changelog shape;
the release workflow lifts the tagged version's section into the GitHub
Release notes, so a release without its section fails loud.

## [Unreleased]

### Changed

- Settings' Support, Connected devices, Theme, and About rows now stay in
  Settings (their sheet opens over it) instead of returning to the deck list;
  Decks folder and Generate deck still take you back to the list so you see the
  result.

## [0.2.0] - 2026-07-21

### Added

- **Pair with your desktop's alix for the tutor and the AI exam.** Settings'
  "Connected devices" sheet takes the URL `alix --lan`
  prints; a bad paste, an unreachable desktop, a too-old server, and a
  rejected token each show one inline line instead of failing silently.
  Once paired, review gains an Ask chip once you've attempted the current
  card, opening the same tutor conversation as the desktop (including
  "Make a card"), and the session summary gains a "Take the exam" chip for
  any deck with a `% source:`. Both borrow the desktop's AI backend over
  your LAN; the phone keeps its own decks and progress, the desktop only
  computes answers. The same row reopens the sheet with an Unpair button.
- The app now declares the INTERNET permission and allows cleartext HTTP,
  both only for talking to a paired desktop on your LAN.
- **A trace deck now opens a walk on the phone, fully offline, instead of
  being refused.** Predict a checkpoint, reveal it against the real
  gutter-numbered source excerpt, then self-grade: no desktop involved.
  Once paired, the walk's done screen offers "Take the exam" for the
  trace's compression question, graded on the desktop the same way a fact
  deck's exam already is; a pass (or a fail, which is re-walked rather than
  turned into remediation cards) lands in the phone's own progress store.
- **The tutor sheet gained "Make a note" beside "Make a card".** Condenses
  the open conversation into up to three lines on the paired desktop, then
  appends them to the deck file on the phone; an empty result says
  "nothing to save" instead of doing nothing silently.
- **"Generate deck" in Settings, shown when a paired desktop is reachable.**
  Give it a URL and optional guidance; the desktop generates the deck text
  the same way `alix generate` does, then the phone asks where to save it and
  writes it under a collision-free file name. The item probes the paired
  server for liveness (as review's Ask chip does), so a paired-but-offline
  desktop hides it rather than offering a button that would only fail.
- **A theme gallery: 18 named themes**, the alix originals plus the web
  app's editor and slide palettes, picked live from Settings' "Theme"
  row. The whole app re-themes without a restart.
- **The deck picker grew:** workspace rows show their emblem and the
  dependency tree's branch prefix, a due exam gets its own marker, mastered
  decks tuck behind a "Mastered · N" row, and a deck now opens straight at
  the depth you last used instead of asking every time (long-press still
  re-picks it).
- **A region breadcrumb above a topology-ordered review**, naming the
  current region and coloring each region by strength, mirroring the web
  client.
- **A "Re-pair" action on the pairing-expired notice**, wherever the app can
  show one: it reopens the pairing sheet directly instead of sending you
  back to Settings.
- A quiet Support row in Settings ("Support alix"): the free alternative
  first, a sponsors link second.
- **The empty deck list offers to add the tutorial deck.** A folder that never
  got the first-run tutorial (a shared folder, or one you emptied) shows an
  "Add the tutorial deck" button that copies the bundled deck in.
- **Workspace deadlines, on the phone too.** A workspace's "ready by" date
  shows on its row (date, days left, ready percent, colored when near or
  past) and again once you drill in; long-press the row to set, move, or
  clear it. The date lives in the workspace's own `alix.local.toml`, so a
  synced folder carries it between phone and desktop, and the phone's
  offline sessions bend their scheduling toward the date exactly as the
  desktop does.

### Changed

- **Breaking: decks are now Markdown `.md` files.** The app reads the new
  Markdown deck format (`## ` fronts, `> ` notes, `\blank{}` gaps, YAML
  frontmatter) and no longer lists old `.txt` decks, so any `.txt` decks in a
  synced folder stop appearing until you convert them (regenerate or
  hand-convert; there is no bundled converter). The first time you open a deck
  for review, alix writes a stable id into each card in the file, which is how
  your review progress now survives edits.
- **Breaking: the Recognize depth is greyed out on a deck without an
  augmentation.** Recognize is now pick-only (it builds its multiple-choice
  from cached AI distractors, never sampled options), so a deck with none can't
  be drilled at Recognize; the depth sheet disables it, and a deck whose
  remembered depth was Recognize opens at Recall instead of an empty session.
  Augment the deck on the desktop (`alix deck augment --target choices`) and
  sync it to enable Recognize.
- The deck list's controls now live in a Settings page that slides up from a
  hamburger (☰) at the top left (replacing a corner 3-dot menu): Support and
  Connected devices up top, then decks folder, generate, theme, and about,
  each with an icon.

### Fixed

- **The `alix` wordmark stays put, in the picker and across screens.**
  Drilling into a workspace added a back arrow that shoved the wordmark to the
  right, and moving between the picker, review, and walk screens shifted it
  too; every wordmark-bearing screen now positions it identically and pins it
  in place during transitions, so it sits at the same spot with or without a
  back arrow and no longer slides when a screen changes.
- **Leaving a trace walk mid-way now asks to confirm, like a fact review.**
  The leave-confirmation only guarded review sessions, so a stray back-swipe
  abandoned a walk silently; both screens now share one guard, so the two
  deck kinds behave the same.
- The session summary no longer shows all zeros after a first pass over a
  fresh deck: it now says how many new cards were introduced, and hides
  the passed/failed rows when nothing was graded.
- The trace walk's "Take the exam" button no longer offers itself when the
  paired desktop is unreachable: it now probes liveness the same way
  review's Ask chip does, hiding the button instead of surfacing "the
  desktop refused" after a dead tap. Predict/reveal/grade stay fully
  offline either way.
- A region breadcrumb with no regions no longer renders an empty strip; a
  few visual-parity fixes (tutor chip wording, exam working/result colors)
  now match the web client.

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
