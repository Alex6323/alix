# 0064: Exam material is frozen per deck and bound to topics

- Status: Proposed
- Evidence: fn the_exam_reads_only_frozen_sources_never_exam_entries_cite_roots_or_cited_text in src/exam.rs
- Recorded: 2026-11-08
- Retrospective: No
- Amends: [ADR 0026](0026-self-describing-ids-and-named-locator-fields.md),
  [ADR 0053](0053-cite-names-the-keyed-roots-citations-resolve-against.md)
  and [ADR 0063](0063-an-exam-result-lives-on-its-cards.md)
  (clauses listed under Compatibility)

## Context

Terms used below:

- A **deck** is a Markdown file of cards. A **workspace** is a folder with an
  `alix.toml` manifest and a `decks/` folder; a deck outside one is a
  **loose deck**.
- A `#` heading opens a block of prose and the cards up to the next `#`.
  This record names that block a **topic**.
- The **AI exam** writes "ideas" (what a learner must understand)
  and questions from the deck's material, then grades answers. The
  **tutor** answers questions about a card. Both run on a **backend**, an
  AI command-line tool alix spawns (Claude, Codex, Copilot, Gemini).
- **Passed** and **Mastered** are exam standings recorded on a deck.
  Passing a deck unlocks the exams of its **dependents**, the decks that
  name it as a prerequisite; review is never locked. A standing is
  **latched**: once recorded with its date it is never cleared by later
  results, only by an explicit reset. In the picker a standing shows
  **solid**, or **dotted** when what it examined has since changed (rule
  12).
- Review runs at three **depths**: Recognize, Recall and Reconstruct.
  Each card has its own schedule per depth, and a card is **due** at a
  depth when that schedule says so. Introducing a card (the Seen press)
  shows it without grading it.

Today a deck's `exam:` key (ADR 0053), and its workspace manifest's,
names live paths, folders and URLs. Exam generation resolves those entries
and reads each local file when it builds the prompt; the tutor receives the
same live entries in its first prompt and reads or fetches them through the
backend (ADR 0026, "What a frozen asset IS"). A checkout that moves, a
deleted folder, or a changed web page silently changes or empties what the
exam reads, so the same deck can be graded against different material on
different machines or days. Each file is truncated at 100,000 bytes with a
marker, nothing caps the total, and some decks hold several megabytes, more
than any backend's prompt accepts. Codex and Copilot receive the prompt as
one command-line argument; on a Linux host with 4096-byte pages, a
131,071-byte argument succeeded and a 131,072-byte one failed with
`E2BIG`.

## Decision

### Frozen sources

1. A deck file lists no exam material. Each workspace member deck owns
   `sources/<deck-id>/` at the workspace root, holding flat copies of its
   source files and a `manifest.toml`. A loose deck has no sources. The
   deck `exam:` key and the `alix.toml` `exam` key are removed and fail as
   unknown keys.

2. A copy is named `source-<52>.<ext>`. `<52>` is the full SHA-256 of the
   file's exact bytes in the token base32 alphabet, which is lowercase
   Crockford base32 (`0123456789abcdefghjkmnpqrstvwxyz`, the alphabet of
   deck and card id tokens): the 32 bytes as one big-endian number in
   5-bit groups from the most significant end, 52 characters for 256 bits. The 4 spare top
   bits are zero, so the first character is `0` or `1`; any other first
   character is invalid. `<ext>` is the text after the last `.` of the
   display path's final component, lowercased, when that `.` is not the
   component's first character and the text is non-empty ASCII
   alphanumeric; otherwise `txt`. This is the rule asset objects already
   use; the extension keeps a frozen copy readable as its file type, so
   editors and viewers highlight it correctly. A
   hash-function change is a manifest `version` change, never
   a use of the spare bits.

3. Sources are local, non-empty UTF-8 text files, copied with their bytes
   unchanged; `add` and `update` refuse an empty file. A URL is not exam
   material. Two sources with identical bytes in one
   deck would share one id, so such twins are invalid candidate state for
   every mutating source command and for repair: `add` and `update` refuse
   the second, and a repair whose drifted files would converge to one id,
   or turn one into another source's bytes, refuses as a whole, naming
   both rows.

4. `manifest.toml` is the whole truth about a deck's sources:

   ```toml
   version = 1

   [sources]
   source-18pgda5qxze0435914h3awa04ej3cmvhk8hvwef6g09ybdv1czkc = { path = "sdk:src/lib.rs", bound-to = ["topic-c9ns6n7s0k9hzm7eb0e7hqn35j", "100-400,512|topic-3whzpgpjp1z1rsfgcvxx4984hn"] }
   ```

   The ids in this example are illustrative, not hashes of real bytes.

   Each key of `[sources]` is a full 59-character source id (`source-`
   plus 52). `path` is the display path, always `key:path`, split at the
   first colon. The key follows ADR 0053's `cite:` key grammar:
   `[a-z][a-z0-9_-]*`, and not `true`, `false` or `null`. The part after it
   is `/`-separated normal components: no `.`, `..`, backslash, colon,
   control character, empty component, or absolute, root or prefix form. No two
   sources in a deck share a display path. `bound-to` lists the topics the
   source grounds (topic ids, or the deck id, which always names the
   leading topic, rule 7, even when it has no cards) and is
   never empty. An entry is a topic id, binding the whole file, or
   `<ranges>|<topic-id>` (for example `100-400,512|topic-3whz…`), binding
   only those lines of the copy to that topic; the ranges come first
   because they are the source's lines, not the topic's. `<ranges>` is one or more of `<a>` or `<a>-<b>` joined by `,`:
   1-based inclusive line numbers without leading zeros, `a < b`, a
   single line written bare, ascending and disjoint, all within the
   copy's line count; anything else is refused. One entry per topic.
   Every persisted structure rejects unknown fields. Every
   load the exam or tutor uses, and every travel operation (rule 15), checks the
   version, the file name grammar, the extension rule, that the copy is a
   regular file and not a symlink (read without following links), and
   digest(bytes) == id, and refuses use on any mismatch.

5. Where each key's live files are on this machine is recorded in
   `sources/<deck-id>/manifest.local.toml` (`version = 1`, a `[roots]`
   table of key to absolute canonical folder). It is private by the
   existing `*.local.*` pattern (ADR 0044) and never travels; a receiving
   machine keeps its own. Roots serve drift detection only: they never
   decide what the exam reads. A key without a root here has its sources
   unchecked, and doctor's drift check says so per key.

6. Sources change only through `alix deck source` commands (`add`,
   `remove`, `update`, `bind`, `unbind`, `list`, `root set|unset`) and
   `alix doctor --repair-sources`. Every mutating command validates the
   complete new state first. `root set|unset` then writes only the roots
   file. A source mutation writes the roots file first when its roots
   change, then writes and verifies new copies, replaces `manifest.toml`
   once by atomic rename, and finally removes copies no row names. A
   returned error or a crash leaves the old or the new manifest with every
   copy it names intact; a root or copy that manifest does not use may
   remain as residue. A row whose copy is missing is damage: the exam,
   the tutor, doctor and every source command refuse the deck's sources.
   A copy no row names is that residue, reported by doctor and removed by
   the next source command or repair. `update` and every row a repair
   rewrites keep their ranges, refuse (a repair as a whole) when any range
   lies beyond the new copy's last line, and print every kept range for
   review. `update` and repair take `--keep-results`, the author's
   declaration that the change keeps the meaning: a result of an affected
   topic is re-keyed to the topic's new fingerprint only when its stored
   fingerprint equals the topic's fingerprint just before the command; a
   result already stale stays stale. The stamp is rewritten for the new
   state only when the standing would be solid with the old fingerprints:
   `sourced` recomputed with them equals the recorded one, and every
   current unsourced card id is in the recorded `unsourced_cards`. Old
   and new fingerprints come from snapshots taken before and after the
   whole command, so a repair touching several rows re-keys correctly.
   The manifest is written first, then the progress store; when the
   store write fails, the command errors naming every topic whose result
   was not kept, and those results stay stale.

### Topics

7. A topic carries a minted id, `<!-- id: topic-<token> -->`, a third kind
   beside `deck` and `card` under ADR 0026's id grammar. Minting writes 26
   characters of the token base32 alphabet from 128 random bits, as for
   cards. A token is valid when it matches `[0-9a-z]+`, exactly as ADR
   0026 rules for deck and card tokens; a valid token that is not
   canonical (26 characters of the token base32 alphabet, so another
   length or any of `i`, `l`, `o`, `u`) draws a doctor warning only, and a
   token outside that grammar is malformed. Exactly one marker closes the topic's
   heading block: after the heading and its prose, before the topic's
   first card, or, in a topic with no cards, before the next `#` heading
   or the end of the file. Two markers in one topic, a marker elsewhere, and a
   marker with no heading above it are parse errors. A topic id on two
   topics, in one deck or in two, and a `bound-to` entry naming no topic of
   the deck are hard errors in doctor and in every source command; card
   review continues. A malformed topic id is a parse error, as a malformed
   card id is. A duplicate is never reminted by alix: the error names the
   copy (in one deck, the later topic), and the author
   deletes its id line and runs `alix deck init`. Nothing attaches to a
   duplicated topic id (a binding, a sitting, a remediation card, a tutor
   draft all refuse it), so the copy holds no reference when the author
   fixes it.
   Every deck has a leading topic identified by the deck id, which always
   exists, possibly empty: it holds the cards before the first `#`, and
   every sidecar card that names it or whose own topic is gone. A deck
   without any `#` is that one topic. Two topics with the
   same heading text are two topics.

8. The word "topic" means this concept everywhere: book, interface, prompts,
   code, and the web API. The wire fields `section_first`,
   `section_context`, `section_context_runs` and `section_context_units`
   become `topic_first`, `topic_context`, `topic_context_runs` and
   `topic_context_units`, and every other wire name that says "section"
   says "topic" (`ExamTopicDto`, `topics`, `topic`). Today's `ContentUnit` types (a block of rendered
   content) keep their name; "unit" no longer names this concept.

### The exam

9. The exam makes one call per topic that has sources, writing that topic's
   ideas and questions from its bound sources only. Sources enter
   the prompt sorted by display path. Ideas are cached per topic,
   keyed by topic id with a material fingerprint: the topic's bindings,
   each written as the full source id (`source-` plus 52, ASCII) followed
   by `:<ranges>` exactly as in its entry when it has ranges, sorted
   ascending by byte, each followed by one `\n`, hashed with SHA-256, the 32 bytes rendered as 52
   characters in the token base32 alphabet exactly as rule 2 renders a
   source id, with no prefix. No version tag or length frame enters the
   hashed bytes: ids are fixed-length and cannot hold `\n`, and a change
   of meaning renames the field holding the value.
   Display paths stay out, so renaming a path or key regenerates nothing
   (cached ideas may name the old path; accepted). Changing one
   topic's sources regenerates only that topic. No
   source is truncated: the whole copy is frozen, and a binding's ranges
   are exactly what is examined. Each range enters the prompt under a
   header `<display path>:<a>-<b>` (or `:<a>`), its lines unchanged; a
   binding without ranges enters whole.
   A sitting stores each examined topic's latest result in
   `DeckProgress.topic_results`, a map from topic id to
   `{at_ms, pass, fingerprint}`, the fingerprint being the topic's
   material fingerprint when it was examined; cards carry no exam result.
   A result counts only while its fingerprint equals the topic's current
   one. A topic has passed while its counting result passes, whatever its
   cards are, and is unexamined while it has none, so a sitting filtered
   to topics needing work always includes a topic whose sources changed. A
   topic with no cards can be examined. Entries whose topic id is no
   longer in the deck are kept and ignored; doctor warns about each one,
   and `alix reset --orphans` removes them, as it does for card and deck
   progress.
   Every card in a deck's personal sidecar file (remediation cards and
   tutor-drafted cards) belongs to a topic, named by one
   `<!-- topic: <topic-id> -->` line directly before the card's
   `<!-- id: -->` line; a sidecar card without it is invalid, and only
   that card is skipped: doctor reports an error and opening the deck
   shows a load warning naming the file and line, never dropping the rest
   of the file. Remediation
   runs one call per failed topic and binds its cards to that topic; a
   tutor-drafted card binds to the topic of the card it was drafted on.
   A sidecar card naming a topic id that is no longer in the deck draws a
   doctor warning and belongs to the leading topic.

10. Exam preflight refuses a topic whose full prompt exceeds the current
    backend's measured limit, naming topic, backend, size and limit, each
    in that backend's own measure. Source commands and doctor only warn,
    because card edits also grow a prompt: they warn when any supported
    backend's own measure of a topic's prompt exceeds that backend's limit,
    reporting the same four facts. Prompts move
    to standard input first; the limits are then measured per backend and
    recorded in the implementation plan.

11. A topic without sources is not examined. A sitting can pass or master
    the deck only if every card of every unsourced topic meets the review
    condition. A retired card meets it. Otherwise the card is read at its
    deepest schedule of Recall or deeper (Reconstruct when it has one,
    else Recall) and meets it when, at that depth, it has graduated (left
    the learning steps; a later lapse keeps it graduated), its latest
    grade passed, and it is not due. A card with neither schedule does not
    meet it; introducing a card creates no schedule. The condition is evaluated once per sitting,
    at one captured time when the sitting starts (before any answer is
    graded, however long before that its questions were generated), and
    the sitting's result uses that one evaluation. A sitting whose check
    fails still runs; its grades are recorded and it cannot pass or master
    the deck. A deck that
    has never held a standing and has no sourced topic has no exam and
    gates nothing; a standing it already holds stays latched (rule 12).

12. Passed and Mastered stay latched, as today. Beside them, one
    persisted evidence stamp, `DeckProgress.exam_stamp` (unknown fields
    rejected), is replaced by every sitting that passes or masters the
    deck; both standings compare against it. It holds two fields:

    - `sourced`: SHA-256 over one line `<topic-id> <fingerprint>\n` per
      sourced topic (rule 9's fingerprint; the deck id for the leading
      topic), the whole lines sorted ascending by byte, rendered as 52 token base32
      characters as rule 2 renders a source id. It covers which topics have
      sources, their bindings, and every re-freeze (`update` or repair
      replacing a copy with new bytes, hence a new id).
    - `unsourced_cards`: every card id (`Card::id`, sub ids included) in
      the deck's unsourced topics, authored and sidecar cards alike (each
      sidecar card placed by its resolved topic, rule 9), sorted ascending
      by byte; always present,
      `[]` when there are none.

    The comparison runs for every latched standing, even when the deck
    has no sourced topic left: the current `sourced` is then the SHA-256 of
    zero bytes, rendered the same way. Every reset that clears Passed or
    Mastered clears `exam_stamp` and `topic_results` with them, in one
    write. `exam_stamp` is omitted
    exactly when the deck holds no standing; a progress document with a
    standing but no stamp is invalid input.

    The standing shows dotted when the current `sourced` differs, or when
    a current unsourced card id is missing from `unsourced_cards`.
    So a card moved from a sourced into an unsourced topic dots, and a new
    blank or blank group on an unsourced card counts as a card addition
    because it adds a sub id. Removing a card or a whole unsourced topic,
    or moving a card between unsourced topics, does not dot: the review condition then covers a
    subset of what the sitting checked. A display-path change alone
    leaves the stamp unchanged. Nothing dots on time alone: a card
    becoming due or lapsing after the sitting leaves the standing solid.
    Card text, card additions and removals in sourced topics, and heading
    or prose edits do not enter the stamp: the exam tests what a topic's
    sources teach on that topic, and the cards only show what the learner
    drilled. A dotted standing is display only; it never re-locks a
    dependent's exam. Accepted gap: a later sitting that passes without
    mastering replaces the stamp, so a dotted Mastered shows solid again
    although the new material was only passed.

### The tutor

13. On a backend whose file reads are confined to its working folder, the
    tutor browses `sources/<deck-id>/` read-only, and the prompt names the
    topic's ranges as its focus. On one that is not
    confined, it receives the card's topic's bound lines in the prompt when
    they fit the backend's limit (rule 10), and otherwise answers from
    the card alone with one visible line saying so.

### Content names

14. Every content-addressed object in `assets/<deck-id>/` (copies of
    cited passages a card's `at:` locator names, and a `diagram:` line's
    `asset:` image and `geometry:` file) is renamed `asset-<52>.<ext>`
    with the encoder of rule 2. Every persisted `fingerprint:` value, in
    `at:` and `diagram:` lines alike, becomes the bare 13-character token
    base32 of its xxHash64 (seed 0, 64 bits most significant first, one
    implicit leading zero bit), with no algorithm prefix; conversion
    re-renders the existing 64-bit value, whose preimage is unchanged. A later algorithm change is a
    deck-format change, converted by tooling outside the repository.

### Travel

15. `sources/<deck-id>/` travels with its deck wherever `assets/<deck-id>/`
    does: deck copy, move, remove, restore, paired sync, workspace share.
    Public share (CLI and web) leaves it out unless asked to include it
    (`--with-sources`, or `with_sources` on the web request), and says the
    receiver cannot take the exam.

## Consequences

- An exam reads the same bytes on every machine that holds the deck, and a
  verdict can be reproduced. Drift between frozen and live files is
  reported only on request, as for cited excerpts.
- Material is per topic, so a large deck no longer has to fit one prompt;
  a single topic still must. Chunked generation is the named fallback if
  measured topics do not fit.
- Authors bind material explicitly. Generators bind the files they drew
  on and fail without writing anything when a file is refused.
- A deck's sources grow the workspace folder and its shares; public share
  omits them by default.
- A Mastered deck means every sourced topic passed its exam and every
  unsourced topic was current on the day of the sitting, not retained for
  weeks.
- URL material, PDFs, and material shared across decks are not supported;
  each is a named roadmap item.

## Alternatives considered

- **Ranges in a separate per-topic map beside `bound-to`.** Rejected: the
  topic id would appear twice per row, so a hand edit could unbind a
  topic and leave its ranges behind.
- **Ranges per source row.** Rejected: two topics could not take
  different slices of one file, since a second row with the same bytes is
  a refused twin.
- **Results on member cards.** Rejected: chosen earlier only because
  topics had no ids; a topic with no cards could never pass, and adding a
  card to a passed topic would unpass it, though the exam tests the
  sources, not the cards.
- **Keep live `exam:` paths and cap the total.** Rejected: grading would
  still depend on files that move or change, and a cap cuts material
  silently.
- **Copies in the deck's asset folder.** Rejected: assets are excerpts by
  design (ADR 0026), and their collector removes unreferenced objects.
- **Hex ids naming the algorithm (`sha256-<hex>`), or a truncated hash.**
  Rejected: every content id takes the shape of deck and card ids,
  `<kind>-<token base32>`, at full hash length with no algorithm in the
  value.
- **Display paths without keys, with the private file holding one
  absolute path per source id.** Rejected: a receiving machine with a
  checkout would have to attach every file separately, and those per-id
  rows break whenever re-frozen bytes arrive from elsewhere with new ids.
  One `root set` per key attaches all of a key's sources and survives
  re-freezes.
- **The roots file under `.alix/`.** Rejected: the roots are settings the
  person chooses, the kind ADR 0044 keeps in `.local` files.
- **Every topic must have sources before the deck sits an exam.** Rejected:
  a vocabulary topic would need meaningless material bound to it.
- **A 21-day review bar for unsourced topics** (the FSRS stability the
  depth badges require). Rejected: adding one source
  to a mostly unsourced deck would make its dependents' exams wait weeks.
- **Any change dots the badge, card edits included.** Rejected: routine
  typo fixes would mark Mastered decks stale, and the exam does not test
  cards.

## Compatibility

Pre-1.0, no compatibility path. An old deck `exam:` key, an old
`alix.toml` `exam` key, and an old idea cache fail as ordinary
invalid input with no dedicated message. Disposable tooling outside the
repository removes old `exam:` lists (they are dropped, not converted),
deletes old caches, mints topic ids, and renames asset objects and
fingerprints in every deck.

This record replaces, once accepted:

- in ADR 0026: in "Id grammar", the kind set (`topic` is added, with the
  same token rule as `deck` and `card`); in "Locator fields", the
  `fingerprint: xxh64-<hex>` and `asset: sha256-<hex>.<ext>` value forms
  and, in "What a frozen asset IS", the examiner and tutor grounding on
  live sources; the whole "One source concept" section (deck and
  workspace material keys, examination and `has_exam`);
- in ADR 0053: rule 1's `exam:` key and the tutor listing `exam:` entries;
  rule 14 (the exam reads `exam:`); rule 15 (the tutor browses a `cite:`
  root, replaced by rule 13 here); rule 16's facts-deck `exam:` input; rule
  17's `exam:` condition on transfer; and the Security clause on local
  `exam:` entries;
- in ADR 0063: the card-level `CardState.exam_result` and the section
  status derived from member records (per-topic `topic_results` with the
  source fingerprint, rules 9 and 12, replace them) and the `--exam` reset
  clause's "card records" (now the topic results and the evidence stamp).

ADR 0026 and ADR 0053 gain an `Amended by: 0064` line naming these clauses
when this record is accepted, and ADR 0053's Evidence markers for replaced
behavior (the exam reading only `exam:` entries, the tutor browsing a
`cite:` root, transfer refusing on `exam:` entries) are replaced then. ADR
0063 already carries its line and lost its `exam_result` marker, since the
field left the code first.

## Security

- Source copies travel with decks and with paired sync, so whatever an
  author adds is disclosed to every receiver; public share leaves them out
  by default.
- The roots file holds absolute home paths; it is private by name and
  never crosses a share or sync boundary in either direction.
- Digest verification at every load and travel boundary stops a tampered
  or truncated copy from reaching the exam. Copies must be regular files,
  checked without following links, so a symlink in a deck's sources folder
  cannot point the exam or the tutor at a file outside it.
- A backend whose reads are not confined never gets a folder to browse;
  OS-level confinement for every backend is a separate roadmap item.

## Verification

To be named in the implementation plan. At least: a deck moved to a machine
without its live files sits an exam with byte-identical topic prompts;
adding a source to one topic regenerates only that topic; one sweep test over
every lifecycle operation proves where `sources/` and the roots file go;
every refusal in rules 2 to 7 has a test; fault injection per command step;
a dotted Passed still unlocks a dependent.

## Reversal

Evidence that would justify replacing this: measured topics that do not fit
the smallest supported backend (then chunked generation), or authors
needing material shared across decks or non-text formats. Only a change to
the manifest's grammar or encoding bumps its `version`; any reversal needs a
replacing ADR and whatever conversion its persisted-data change requires,
done by tooling outside the repository.
