# 0062: The tutorial ships stamped, outside every decks folder

- Status: Accepted
- Recorded: 2026-11-02
- Retrospective: No
- Evidence: id: "deck-00m30g5j1s54fjvkz49g5kzfkz" in assets/decks/tutorial.md
- Evidence: fn law_base_dir_uses_only_a_valid_profile_name in src/tutorial.rs
- Evidence: fn materialize_then_walk_leaves_the_deck_byte_identical in src/tutorial.rs
- Evidence: fn the_shipped_citation_fingerprint_matches_the_sample in src/tutorial.rs
- Evidence: pub fn allows_personal_writes in src/walk.rs
- Evidence: fn law_no_personal_write_reaches_a_tutorial_walk in tests/api.rs
- Evidence: fn law_the_tutorial_walk_refuses_each_personal_write in mobile/alix/rust/src/api/review.rs
- Amends: [ADR 0018](0018-explicit-deck-initialization.md) (tutorial
  seeding as a workflow that initializes its own output)

## Context

Alix ships a tutorial: a deck (`assets/decks/tutorial.md`) whose cards
teach the app by having the learner use the feature each card describes
(grade, open a section's text, open a hint, pick a choice, reveal line by
line, fill a blank, hide an answer, show a cited source). One card cites a
small sample file shipped beside it (`assets/decks/tutorial-sample.py`): its
locator names a line range in that file, so the learner can see the excerpt.
A citation carries a fingerprint, a hash of the cited lines that flags when
the file has drifted from what the card was written against.

Before this record, the desktop copied the tutorial into a brand-new decks
folder and the phone copied it (with a `basics.md` deck and a sample
workspace) into its decks folder on first run. Each copy was then
initialized like any new deck (ADR 0018: a Markdown file becomes an Alix
deck, which Alix may rewrite to add ids, only once a deck id is minted into
it, by `alix deck init` or by a workflow that creates the deck), so every
install minted its own deck and card ids. The tutorial was listed
in the picker beside the learner's own decks, it was retired by deleting the
file, and an app update could not change a copy already on disk.

The tutorial is now opened from a menu (the web header menu, the phone's
Settings) and from a button on an empty picker, as a walk: a read-through of
a deck one item at a time that remembers the last grade of each item and
never schedules (ADR 0061). This needs a place for the deck, its sample file
and its progress that is not the learner's library, and an identity for its
cards that survives an app update.

## Decision

1. **Location.** The library function `tutorial::materialize(base)` writes
   the bundled deck and sample file into a tutorial folder and returns the
   deck path. The folder is outside every decks folder:
   - Desktop: under the data root (the per-user state directory for alix,
     on Linux `$XDG_STATE_HOME/alix`, else the per-user data directory on
     platforms without one, such as macOS Application Support; the same
     root the server log uses), `tutorial/default/` for a launch without a
     profile name and `tutorial/profiles/<name>/` for a profile launch.
   - Phone: `tutorial/` in the app's support directory, beside its decks
     folder.
2. **Profile name.** A profile is a named config file in a `profiles`
   directory, launched with `alix profile <name>` or `--config`; its name
   is the file's stem. `tutorial::base_dir` uses a profile name only when it
   passes the same validation as `alix profile add` (not empty, no `/`,
   `\` or `.`, not one of the reserved names `add`, `list`, `remove`,
   `default`); any other name falls
   back to `tutorial/default/`. A name taken from a `--config` path is not
   otherwise validated, so this keeps the folder inside `tutorial/`.
   `alix profile remove <name>` deletes `tutorial/profiles/<name>/`.
3. **Progress.** The tutorial walk stores its progress the way any walk
   over one deck does, in the `.alix/` directory beside the deck (which
   holds that deck's progress), so in the tutorial folder. It never touches
   the learner's decks folder, the progress store of the served decks, or
   the recently opened decks list.
4. **Identity ships with the asset.** The deck id, card ids, the blank's
   `b:` and `position:` stamps (machine-written markers that identify a
   fill-in-the-blank span) and the citation fingerprint are minted once
   in the repository and committed. Every install shares these ids. Nothing
   initializes, stamps or re-mints the tutorial at runtime: `materialize`
   rewrites each of the two files only when its bytes differ from the
   bundled ones, and a walk of the bundled deck leaves it byte-identical.
5. **No personal writes.** A walk opened as the tutorial is flagged
   (`WalkSession::mark_tutorial`), and every caller of a personal write
   (a note or a card the learner creates through the in-session AI tutor)
   asks `WalkSession::allows_personal_writes` first. The web
   server answers 403 and the phone bridge returns an error. Asking the
   tutor stays available.
6. **No seeding.** Neither client copies the tutorial or any sample deck
   into a decks folder. A first desktop run still creates the decks folder.
7. **Editing the asset.** A new card is minted by running
   `alix deck init assets/decks/tutorial.md`. A change to the sample file
   needs its citation fingerprint re-stamped with the ignored test
   `stamp_the_shipped_tutorial` (`cargo test --lib -- --ignored
   stamp_the_shipped_tutorial`), which removes the old fingerprint first.
   Unchanged cards keep their ids, so an app update keeps their progress.

## Consequences

- The tutorial never appears in a picker and needs no deletion to retire;
  the learner simply stops opening it.
- A fresh install's picker is empty until the learner adds a deck; the
  empty picker offers the tutorial.
- An app update that edits the tutorial reaches every install on its next
  open. Edits the learner makes to the tutorial files are overwritten on
  the next open.
- A `tutorial.md`, `basics.md` or sample workspace already copied into a
  decks folder by an earlier version stays there as an ordinary deck.
  Nothing recognizes it.
- The repository now carries a stamped asset whose ids are shared by every
  install. Re-minting them (for example by stripping the id lines) resets
  the tutorial's progress on every install.

Residuals, named and not handled:

- Scoped launches (`alix <dir>`, which serve a named folder) and
  `--config` launches that name no profile share `tutorial/default/`.
- Two servers using `tutorial/default/` at once save its progress as whole
  documents, so one can overwrite the other's tutorial progress. Untested.
- `alix --config <dir>/profiles/<name>.toml` with a `<dir>` other than the
  profiles directory shares the real profile `<name>`'s tutorial folder.
- A served folder that contains the data root could list the tutorial
  folder as decks. Unverified.
- The deck and the sample file are each written atomically, but not as a
  pair. A crash between the two writes leaves one stale; the next open
  rewrites it.
- A profile name containing `:` passes validation and could make
  `Path::join` leave `tutorial/profiles/` on Windows. Unproven; a Windows
  file name cannot contain `:`, so such a profile file cannot exist there.
- The tutorial is in English only.
- The phone's hide gesture and source switch have no on-card cue for a
  learner who never opens the tutorial.

## Alternatives considered

### Keep seeding into the decks folder

This lists the tutorial among the learner's decks, mixes its progress into
the learner's store, makes deletion the only way to retire it, and leaves an
outdated copy on disk after an update.

### Initialize the tutorial at runtime on each install

Each install would mint its own ids on first open. A later update that
rewrote the deck file would drop those ids and mint new ones, resetting the
learner's tutorial progress on every update; keeping them would require
merging the update into the stamped copy. Shipping the ids avoids both.

### Keep the tutorial's progress with the served decks' progress

The tutorial's progress would then sit in the progress store of the served
decks, beside the learner's library. A separate folder keeps the two apart
and needs no new store machinery, because a walk over one deck already
stores progress beside its deck.

### Use the profile name unvalidated

A `--config` path whose stem is `..` would point the folder at `tutorial/`
itself. Because the layout is persisted on disk, the fallback to `default`
is decided now.

## Compatibility

The on-disk layout `tutorial/default/` and `tutorial/profiles/<name>/` under
the data root, and the shipped deck and card ids, are persisted. Changing
either loses tutorial progress (not library progress). Pre-1.0 (the
project's rule: no backwards compatibility before 1.0), no reader
recognizes an earlier seeded copy; it is an ordinary deck.

## Security

The data root gains a new persistent folder, written on every tutorial open
with bundled bytes only. The folder name derives from the profile name only
after validation, so a crafted `--config` stem cannot move it out of
`tutorial/`. The ADR 0018 boundary is unchanged for every other deck: the
tutorial carries its own deck id and is never initialized at runtime. A
tutorial walk refuses personal writes, so no learner content lands in the
tutorial folder. A web server without a data root answers 503 and
writes nothing.

## Verification

- `src/tutorial.rs`: `law_base_dir_uses_only_a_valid_profile_name`,
  `materialize_then_walk_leaves_the_deck_byte_identical`,
  `materialize_restores_edited_files`,
  `law_every_do_it_card_reaches_its_affordance` (each teaching card offers
  what it teaches in a walk), and
  `the_shipped_citation_fingerprint_matches_the_sample`.
- `src/walk.rs`: `a_walk_allows_personal_writes_until_marked_tutorial`.
- `tests/api.rs`: `the_menu_tutorial_is_a_walk_outside_the_library`,
  `law_no_personal_write_reaches_a_tutorial_walk`,
  `an_unwritable_tutorial_base_fails_without_touching_the_library`.
- `tests/cli.rs`: `a_first_launch_creates_the_decks_folder_and_seeds_nothing`,
  `a_profile_launch_keeps_its_own_tutorial`; `src/cli/profile.rs`:
  `profile_management_round_trips_configs_rows_and_removal` (removal deletes
  that profile's tutorial folder only).
- `mobile/alix/rust/src/api/review.rs`:
  `open_tutorial_walks_to_a_resolved_excerpt`,
  `law_the_tutorial_walk_refuses_each_personal_write`.

## Reversal

If a tutorial in the picker, or one per decks folder, proves more useful
than a menu entry, replace this record. Moving the folder needs external
tooling that moves `tutorial/` with its `.alix/` directory, or accepts the
loss of tutorial progress, which is all that is at stake.
