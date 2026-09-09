# Opening a workspace on the phone: timing before and after parse-once

alix is a spaced-repetition trainer for Markdown flashcard decks; its
Android app embeds the Rust library through a Dart-to-Rust bridge and
can be paired with a desktop over the local network, pulling a copy of
the desktop's decks. Measured 2026-09-09 on the phone of the maintainer
(Alex), who also did the tapping. Raw rows, no averaging. The occasion: on 2026-09-08 he
reported the Android app as slow when opening a big workspace, and the
listing code was changed to parse each deck once per listing (commit
e78cf0d4). This note records what the phone measured before and after
that change.

## What was measured

The Android app's first screen, the picker, lists decks and workspaces
(a workspace is a folder with an `alix.toml` manifest and a `decks/`
directory of member decks; a loose deck is one that sits directly in a
folder instead of in a workspace). Each listing is one bridge call into
the library. Opening the app lists the app's own root folder (`root`)
and, because this phone is paired with a desktop, the pulled copy of the
desktop's root (`paired-root`). Tapping a workspace row lists
that workspace's members (`members`). A build made with `PROFILE=1`
prints one line per listing call with a Dart-side stopwatch around the
bridge call (`bridge_ms`), a Rust-side timer around the library call
(`lib_ms`), and the library's file-work counters for that call:

- candidates classified: Markdown files (`.md`) in the listed folder
  that the member classifier read to decide whether each is an
  initialized deck; the classifier filters to `.md` files before
  counting, so directories and other entries never reach the counter
  (loose decks at a root are examined by a different path that is not
  counted, which is why a `root` row can show fewer candidates than
  parses);
- manifest reads: reads of an `alix.toml`;
- deck parses: member decks parsed in full;
- prerequisite parses: decks parsed only to check whether they are
  finished (a `requires:` line in a deck's frontmatter names a
  prerequisite deck, and a deck with an unfinished prerequisite is
  locked);
- id scans: directory scans to find a prerequisite named by deck id;
- geometry reads: reads of the geometry file that `alix deck init`
  writes beside the image it renders from a text (mermaid) diagram;
- progress docs and augment docs: per-deck review-progress documents and
  per-deck cached AI material (choices, notes) read;
- canonicalize: `canonicalize` syscalls, each resolving one path's
  symlinks and relative parts to one absolute form;
- sidecar reads: per-deck files holding the user's personal notes. The
  after build counts them and reported 0 on every row; the before build
  predates the counter, so its rows show "not counted".

Setup:

- Device: Samsung Galaxy S10e (SM-G970F), Android 12, reached over
  wireless adb (Android Debug Bridge).
  Screen on, app in the foreground, nothing else started by hand.
- Before build: commit bcd3adaa, which adds the counters and the profile
  line to the listing code as it stood, built with `make apk PROFILE=1`.
- After build: commit e78cf0d4, the parse-once listing, same build
  command.
- Workspace: the paired desktop root holds one workspace of 218 member
  decks with no prerequisites between them (0 prerequisite parses, 0 id
  scans on every row) and no rendered diagrams (0 geometry reads). The
  local root holds four loose decks.
- Every row below had the files warm: the app had listed the same files
  seconds earlier. See "Not measured".

## Rows

`members`: one tap on the workspace row, back to the picker, tap again.

| Build | bridge_ms | lib_ms | candidates | manifest reads | deck parses | prerequisite parses | id scans | geometry reads | progress docs | augment docs | canonicalize | sidecar reads |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 2078 | 2077 | 436 | 6 | 654 | 0 | 0 | 0 | 36 | 62 | 654 | not counted |
| before | 2106 | 2105 | 436 | 6 | 654 | 0 | 0 | 0 | 36 | 62 | 654 | not counted |
| before | 2078 | 2077 | 436 | 6 | 654 | 0 | 0 | 0 | 36 | 62 | 654 | not counted |
| before | 2091 | 2090 | 436 | 6 | 654 | 0 | 0 | 0 | 36 | 62 | 654 | not counted |
| before | 2096 | 2095 | 436 | 6 | 654 | 0 | 0 | 0 | 36 | 62 | 654 | not counted |
| after | 745 | 744 | 218 | 1 | 218 | 0 | 0 | 0 | 18 | 31 | 654 | 0 |
| after | 744 | 743 | 218 | 1 | 218 | 0 | 0 | 0 | 18 | 31 | 654 | 0 |
| after | 757 | 756 | 218 | 1 | 218 | 0 | 0 | 0 | 18 | 31 | 654 | 0 |

Launch rows: every listing line printed in the ten seconds after the app
was started fresh (the install had stopped it), in order, with the
timestamp's seconds.

| Build | at | listing | bridge_ms | lib_ms | candidates | manifest reads | deck parses | prerequisite parses | id scans | geometry reads | progress docs | augment docs | canonicalize | sidecar reads |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| before | 14.421 | root | 5 | 5 | 4 | 8 | 4 | 0 | 0 | 0 | 0 | 0 | 7 | not counted |
| before | 14.426 | root | 3 | 3 | 4 | 8 | 4 | 0 | 0 | 0 | 0 | 0 | 7 | not counted |
| before | 15.297 | paired-root | 870 | 870 | 430 | 5 | 215 | 0 | 0 | 0 | 18 | 31 | 431 | not counted |
| before | 15.314 | root | 3 | 3 | 4 | 8 | 4 | 0 | 0 | 0 | 0 | 0 | 7 | not counted |
| before | 16.093 | paired-root | 779 | 779 | 430 | 5 | 215 | 0 | 0 | 0 | 18 | 31 | 431 | not counted |
| after | 19.871 | root | 4 | 4 | 2 | 3 | 4 | 0 | 0 | 0 | 0 | 0 | 7 | 0 |
| after | 19.877 | root | 3 | 3 | 2 | 3 | 4 | 0 | 0 | 0 | 0 | 0 | 7 | 0 |
| after | 20.700 | paired-root | 823 | 823 | 218 | 1 | 218 | 0 | 0 | 0 | 18 | 31 | 437 | 0 |
| after | 20.713 | root | 3 | 3 | 2 | 3 | 4 | 0 | 0 | 0 | 0 | 0 | 7 | 0 |
| after | 21.578 | paired-root | 864 | 864 | 218 | 1 | 218 | 0 | 0 | 0 | 18 | 31 | 437 | 0 |
| after | 21.676 | root | 6 | 6 | 2 | 3 | 4 | 0 | 0 | 0 | 0 | 0 | 7 | 0 |
| after | 22.493 | paired-root | 817 | 817 | 218 | 1 | 218 | 0 | 0 | 0 | 18 | 31 | 437 | 0 |

## Reading

- The drill-in, the thing Alex reported as slow, dropped from about
  2.1 s to about 0.75 s per open. The before build parsed every member
  three times (654 = 3 x 218) and classified every candidate twice
  (436 = 2 x 218); the after build parses and classifies each member
  once. Which passes made the three parses is read from the code, not
  from these rows: one to build the member rows, one to find each
  member's parent for the dependency tree, and a second member pass that
  computed the workspace's deadline.
- `bridge_ms` equals `lib_ms` within a millisecond on every row, so the
  crossing between Dart and Rust costs nothing measurable. Whatever the
  user still waits for beyond `lib_ms` happens after the bridge call
  returns; this instrument does not see it and this note does not
  measure it.
- The remaining 0.75 s is 218 parses plus 18 progress and 31 augment
  documents. The phone rows do not split it; the host split below does,
  and there the parse is 84 percent. These rows are one workload size and say nothing
  about scaling; the tests added with e78cf0d4 (`law_a_member_listing_`
  in `src/listing.rs`, `law_one_open_` in the bridge crate) assert
  exact counter values per listing at several sizes, and they, not this
  note, pin the counts.
- The `paired-root` listing did not get faster. On both builds the
  workspace's row on that screen parses every member once to compute
  whether the workspace has cards due and its deadline setting, about
  0.8 s per listing. The before build's `paired-root` rows report 215
  parses and 430 candidates, which is 215 members classified twice,
  where its `members` rows report 218 members (436 candidates, 654
  parses); the after build reports 218 members on both. The three-member
  difference in the before build's root pass is not explained by this
  note.
- The launch rows show the before build listing `root` three times and
  `paired-root` twice within 1.7 s of a fresh start, and the after build
  listing `root` four times and `paired-root` three times within 2.6 s
  (first row 19.871, last row 22.493). The after build's listings sum to
  about 2.5 s of bridge time, of which the repeats are work just done.
  That is a picker-side behaviour, separate from the parse-once change.
- `canonicalize` calls did not drop on the drill-in (654 on both
  builds). The reason is read from the code, not from the rows: the
  dependency-tree pass canonicalizes each member and each requirement
  target on its own, separately from the map of canonical paths the
  listing's parse table already keeps.

## Host split, same workspace

Run on the maintainer's desktop against the same 218-member workspace
(the counters of one `list_members` call there equal the phone's after
rows exactly: 218 candidates, 218 parses, 18 progress documents, 31
augment documents, 654 canonicalize calls). A scratch program linked
against the library timed each phase over all 218 members, best of seven
warm runs; it is not in the repository, and the phases are named by the
library calls so the run can be repeated.

| Phase over all 218 decks | Total | Per deck |
| --- | --- | --- |
| `std::fs::read_to_string` of every deck | 0.8 ms | 0.004 ms |
| read plus a string search for the frontmatter's end | 0.8 ms | 0.004 ms |
| read plus `parser::deck_identity` (the frontmatter id reader) | 8.2 ms | 0.038 ms |
| read plus counting the `<!-- id: card-` lines | 2.0 ms | 0.009 ms |
| `Deck::load` (the full parse) | 220 ms | 1.0 ms |
| `listing::list_members` (everything) | 260 ms | 1.2 ms |

The full parse is 84 percent of the listing on the desktop; the progress
and augment documents and the rest are at most the remaining 40 ms. The
phone's 745 ms is the same work about three times slower, and its parse
share is not measured separately.

A second run later the same day split the parse itself (same workspace,
same method; the full parse measured 217 ms in that run):

| Phase over all 218 decks | Total | Per deck |
| --- | --- | --- |
| read plus the parser's block stage alone (line preparation, frontmatter, headings, directives, notes; the private `parse_document`, exported for the run and not kept) | 31 ms | 0.14 ms |
| `parser::parse` (the block stage plus card building) | 217 ms | 1.0 ms |
| `parser::content_fingerprint` once over the 2216 cards those decks hold | 62 ms | 0.28 ms |

So card building is about 86 percent of the parse. Inside it, the content
fingerprint (the hash the tutor and the augment cache key on, computed by
stripping inline markup from every card's text) is computed twice per
authored block at parse time: a scratch call counter saw 4432 calls in
one full parse that returned 2216 cards. Codex's independent cross-check
(direct in-function timing in an isolated clone, 21 warm release runs,
same workspace) confirmed the count and split it: 2210 calls in
`Card::plain` over the built card's front and back (65 ms), 2210 calls
for the parser's block-level key over the front plus the cover-masked
raw answer lines (84 ms), and 12 recomputations for region cards
(0.2 ms); the 4432 equals two per returned card only in aggregate, since
six blank-bearing templates are replaced by the twelve region cards. The
hash is 149 ms of the 221 ms `Deck::load` there, 67.5 percent, and the
block-key pass costs more than the constructor's pass because it hashes
the raw lines rather than the derived display lines.

## Not measured

- Rows after a device reboot (files cold). Wireless adb does not survive
  a reboot on this phone, so the cold rows were skipped.
- Anything after the bridge call returns: layout, paint, the picker's
  own rebuild time.
- Other devices, other workspaces, a workspace with prerequisite chains
  or rendered diagrams.
- The phase split on the phone itself (the host splits above are the
  desktop's).

## What follows

Recorded on the maintainer's local roadmap (a file outside the
repository) under its workspace-listing item: the root screen's
per-workspace member pass, the repeated listings on launch, a shared
canonical-path map for the dependency-tree pass, and a listing cache for
a reopen with nothing changed. The exact-count tests keep the
per-listing file work pinned; a slower parser at equal counts is
invisible to them, which is why the mobile release checklist in
RELEASING.md asks for this measurement to be repeated on a real phone
before a release.
