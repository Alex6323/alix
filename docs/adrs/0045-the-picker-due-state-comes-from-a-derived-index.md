# 0045: The picker's due state comes from a derived per-deck index

- Status: Rejected
- Recorded: 2026-09-10
- Revised: 2026-09-11, rewritten from a count-bearing index to a boolean one
- Rejected: 2026-09-11. Alex ruled to go without a per-deck due signal for now
  and to keep the question as an investigate roadmap item
  (`{#picker-due-signal}`). The index existed to carry that signal, so with the
  signal gone it has nothing left to do. The measured performance win it was
  credited with comes from not parsing decks to build a picker row, which does
  not need an index. Do not build from this record.
- Retrospective: No
- Refines:
  [ADR 0044](0044-private-state-in-a-dot-alix-directory-and-local-twins.md)
- Relates to:
  [ADR 0005](0005-progress-store-durability.md) and
  [ADR 0042](0042-paired-entry-pull-and-progress-push.md)

## Context

The picker is where every session starts. Deciding what a row shows requires a
full parse of every deck in the listed collection, so a workspace with many
members pays a parse per member to produce a few booleans per row.

The served desktop softens this with a process-lifetime, (mtime, size) validated
parse cache, so a warm re-list of unchanged decks parses nothing. The softening
is partial by construction: the cache dies with the process, only the server
builds one, and any deck whose file changed falls out of it. The phone holds no
cache at all. Measured on 2026-09-11 (`docs/results/2026-09-11-picker-due-snapshot-phase-0-baselines.md`):
a 220-member workspace lists in about 32 ms warm on the desktop and 525 ms on an
SM_G970F, and the phone's profile counters report `decks_loaded=220` on every
single listing, so the phone has no warm case at all.

This record originally proposed that the index carry a due COUNT, and that the
row show it. That is no longer the design. Alex ruled on 2026-09-11 that a
number on the row is the wrong thing to show, in his words because of the
"workload shock if learners see they have 1000 due cards", and that rows carry a
title and a description while every count and statistic moves to the deck view.
The row vocabulary he ruled is four states:

| Row state | Shows | Derived from |
| --- | --- | --- |
| never started | the existing "new" badge | the progress document; no parse, no index |
| due | a SOLID dot | a valid index saying something is due |
| unknown | a DOTTED dot | no valid index |
| nothing due | nothing | a valid index saying nothing is due |

Solid against dotted reuses an idiom already on these rows: the depth badge
renders dotted for "earned, currently lapsed" (`web/alix/review/picker.js` lines
392-393). "New" takes its DECK-level meaning, "you have never started this
deck", which the progress document answers without a parse. It is not today's
meaning in `src/listing.rs` line 594, "contains at least one unseen card", which
needs a parse and which saturates: for a learner part-way through many decks it
is true on nearly every row, and a badge on every row is not a signal.

What survives the collapse is the expensive half. The row still cannot be drawn
without knowing whether a session would serve anything, and that still requires
applying the session's admission rule, which still requires a parse. Making the
listing cheaper and making it honest remain one problem.

The obvious shortcuts are all worse than they look. Caching the answer in the
progress document turns a derived value into irrecoverable state and makes every
deck edit a write to the durable store of ADR 0005. Caching it in memory dies
with the process, which is exactly the case the phone's first screen is. Showing
a remembered answer and marking it stale produces a picker that can lie, and the
maintainer ruled that a signal that can lie is worse than no signal.

## Decision

The picker's row state comes from a per-deck **index**: a small derived JSON
document holding what only a parse of the deck knows, so that a listing can
apply the admission rule without parsing.

**Nothing derived from the index is a number.** The row's signal is a verdict,
not a count: no part of the listing, the wire contract, or any client counts
cards.

**One ruling this record depends on is NOT MADE, and it must not move to
Accepted before it is.** Whether picker rows keep their per-depth launch
controls decides how many verdicts the rule computes per row and how many fields
the row DTO carries; the spec tracks it as D16 and as its open question 1. This
record is written on the branch where they stay. That branch also decides
whether `can_recognize` stays in the listing at all, and with it whether the
listing keeps `depth::deck_recognizable`, measured at 4.082 ms of the study
workspace's 19.663 ms `deck_status`.

**What the index stores is inputs, not verdicts, and the distinction is
load-bearing.** An earlier draft of this record said the index "carries the
per-depth reviewability flags", which is wrong and would have been built. A
reviewability flag is a function of the parse AND the store AND the clock AND
the augment cache; only the first is pinned, so a stored flag would go wrong
whenever a review, a passing hour, or a distractor generation changed one of the
other three, without moving any pin. The index stores only what a parse knows
(the Content paragraph below is the list), and the admission rule evaluates
those entries against the store, the review configuration, the clock, and the
augment cache AT LISTING TIME to produce the verdict. This is the same rule ADR
0044's neighbour records already follow and the same one this record's own
"Entries are not the rule's only input" paragraph states; the deleted sentence
contradicted both.

Two consequences of "verdict, not count" are worth stating because they are why
this shape is cheaper than the one it replaces, not merely smaller. The
admission rule may stop at the first admissible entry instead of visiting every
entry, and a collection aggregate becomes an OR rather than a sum, which removes
the exact-or-null aggregate problem entirely: a member without a valid index
cannot corrupt a disjunction, it can only leave it unknown.

**Precedence among the four row states**, which the table above does not carry:
"new" wins over everything (Alex, 2026-09-11). A deck with no progress document
shows the badge and no dot, whatever an index would say, so the first two rows
of that table never collide even though a never-started deck's index would
report everything due.

**Location and category.** An index lives at
`<content root>/.alix/index/<deck-id>.json`, beside the private state ADR 0044
placed there. The content root is the folder that owns a deck's private state,
`workspace::content_root` in the code; ADR 0044 calls the same folder a store
root. It is a third category in that directory: **derived and disposable**, as
against the irrecoverable private state (progress, recent, the paired-sync
files) that directory held until now. Deleting the whole `index/` directory
costs nothing but recomputation, though with the bulk action deferred that
recomputation is manual: every row goes dotted and clears only as its deck is
opened. Any tool that reasons about `.alix/` must treat
this category as recomputable rather than as the person's state.

**Exact or absent against every input the pins observe.** An index is valid only
when its `schema` equals the current `INDEX_SCHEMA` and all four of its pins
match: the hash of the deck bytes just read, the hash of the sidecar bytes (null
on both sides when there is no sidecar file), the hash of the resolved workspace
defaults that affect the derivation, and a **derivation pin**: the fingerprint of
what the derivation produces for a deck-and-sidecar fixture compiled into the
binary, computed once per process. Anything else, including an unreadable,
truncated, or unknown-field document, reads as ABSENT. A listing never fails
because of an index, and a row whose index is absent shows the dotted dot rather
than a stale claim. The derivation pin invalidates every index mechanically when
the deriving code changes, without a human remembering to bump a version, for
every change the fixture's shapes exercise. Its limit is exactly that: a change
that leaves the fixture's derived output identical moves no pin, so the
fixture's coverage is the mechanism's coverage, and widening the fixture is how
that coverage grows.

Validating the deck pin means reading every deck's bytes on every listing. The
index removes parses, not file reads, and that is priced rather than assumed:
reading and hashing all 221 deck files of the study workspace costs 0.987 ms
warm against the 20.4 ms of parse-dependent work it lets the listing skip.

**One derivation, and a named set of writers.** The two writers are the private
inner writer behind `write_deck_text` (which the card-removal path also reaches)
and the stamper's writer; naming them here is the point, since this record calls
the set "named" and a reviewer cannot keep a two-function enumeration honest
against a document that never enumerates it. Exactly one function derives an
index body, from bytes the caller read and defaults the caller resolved, and
nothing else constructs one. Two OWNER OPERATIONS may create an index that does
not exist yet, and each is paid on an explicit action rather than on a listing:
a session build, and opening a deck's own screen. Separately, the two functions
that write a deck file refresh an index through that same derivation, update
only, so a write into a staging tree leaves no index behind.

It follows that a deck nobody opens or edits has no index, shows the dotted dot,
and stays that way until someone opens it. Under the count-bearing design a
third owner operation, a bulk "count due cards" action, existed to clear that
state in one press. With no count to display, that action needs a new
justification and a new name, and Alex ruled on 2026-09-11 that it is separate
work rather than part of this record. **Until it exists, the only way to clear a
dotted row is to open that deck.** That is a real cost of shipping this record
alone and is stated here rather than left to be discovered.

**One admission rule.** The rule that decides whether a session would serve
anything at a depth is implemented once, in a library module over index entries,
and never reads a `Card`. The session build derives an entry from each parsed
card and calls that same rule, so the dot on the row and the queue the session
builds agree by construction rather than by discipline, EXCEPT in the two
windows named immediately below. The unqualified form ("cannot disagree") stood
in an earlier draft and was falsified three paragraphs later by its own
document.

That guarantee has two named limits. First, a session normally stamps the deck
file it opens, so no unstamped unit survives into its queue, while an index
keeps them. On a file alix can write but cannot stamp, the session therefore
excludes cards the index admitted. Under a count this made the row read HIGH by
a visible amount; under a boolean it can only turn a dot solid where the session
would find nothing, which is a smaller and quieter error. The server logs it; no
client surfaces it.

Second, a sibling's claim on a card token. `resolve_duplicates_at_open`
(`src/assemble.rs`) scans the whole member directory when a deck is opened and
re-mints the losing side of any repeated card token; a re-minted id carries no
progress, and a card with no progress is due. No pin observes a sibling's bytes,
so from the moment a second deck claims a token until the loser is next opened,
the loser's index matches every pin while the session would serve more than it
admitted. Under a count this read LOW. Under a boolean it can only read "nothing
due" where the session would in fact serve something, so the row shows no dot
where it should show a solid one. Copying a deck file inside its own folder is
enough to enter that window. Closing it means reading every sibling on every
listing, which is exactly the cost this design exists to remove, so it is
disclosed rather than built, and named `{#index-misses-a-sibling-token-claim}`
for the day the trade looks different.

Entries are not the rule's only input: it also takes the store, the review
configuration, the clock, and, because recognizability at Recognize depth turns
on cached AI distractors, the augment cache. Those stay evaluation-time
arguments and are deliberately not pinned into the index, so generating
distractors changes what the rule returns without invalidating anything.

**An unsynced writer.** The index writer writes a sibling temporary file and
renames it, so a concurrent READER sees the whole old document or the whole new
one, but it does not fsync, which is where it departs from the writer model ADR
0005 sets for the progress store.

The atomicity claim covers readers, not concurrent WRITERS, and the house
convention does not extend to them: `temp_beside` (`src/fsio.rs` lines 24-27)
builds a deterministic `.<name>.tmp`, so two processes writing one deck's index
(a server plus a CLI invocation, or two CLI invocations) create the same tmp
path, truncate each other, and one renames the other's partial bytes into place.
The outcome degrades safely, since a mangled document fails to parse and reads
ABSENT, but this record must either specify a per-writer-unique tmp name or
declare concurrent writes to one index out of scope. It currently says neither.

The reason for skipping fsync is the category: a lost or
truncated index costs one re-derivation, and a truncated document fails to parse
and therefore reads as absent. Paying two fsyncs per member to make a cache
durable would spend the saving this record exists to produce.

**Not synchronized.** Index files are never listed in a paired-device pull
manifest and are never pushed. A pull carries the ones already on the receiving
side across its swap without reporting them as device-local files, so a pulled
subtree keeps working; the pins decide validity there as everywhere.

**Content.** An index holds review-unit ids (null for an unstamped card),
whether an entry came from the personal sidecar rather than the deck, lock
structure, shape and recognizability inputs, link-definition labels, line
numbers, and hashes. It holds no answer text, no notes, and no counts. It does
not hold the deck's title or description: those reach the row from the
classifier, which never parses cards (`picker::DeckEntry`, `src/picker.rs` lines
137-147), so the index has no reason to duplicate them.

## Consequences

- A listing applies the admission rule per row without parsing, so the picker
  can show the four-state vocabulary above on a phone that would otherwise parse
  the whole collection. Measured projection for the study workspace, every term
  from the phase 0 baselines rather than estimated: about 14.5 ms warm against
  32 ms today at that workspace's card density (10 review units per deck), and
  about 24.3 ms at 100 per deck. The index read is the one term that scales with
  total review units rather than with deck count, so a dense collection narrows
  the saving.
- The same one job reaches slower devices, which is what this record is for. The
  desktop's warm case was never the motivation and is not the test.
- `new_cards` changes meaning and stops needing a parse. Today it is
  "contains at least one unseen card", computed by iterating cards
  (`src/listing.rs` line 594). Under Alex's ruling the row's "new" badge means
  "this deck has no progress document", which the store answers directly. The
  field's old meaning has no remaining consumer on the row; whether the deck view
  keeps it is the deck view's question.
- A collection's aggregate is an OR over its members, with unknown absorbed:
  any member due makes the collection due, otherwise any member unknown makes it
  unknown, otherwise it shows nothing. The count-bearing design had to choose
  between an exact-or-absent sum and a partial "12 due, 3 unknown" shape, and
  that choice is now moot rather than settled. Under a sum, one edited member
  removed a collection's number; under a disjunction, one edited member can only
  fail to contribute a true.
- Two named lib writers gain a second obligation, refreshing the index through
  the same derivation, update only. No other writer refreshes; where one changes
  a pinned input the old index becomes pin-stale and reads absent, and where it
  publishes unchanged pinned bytes a valid index stays valid. The obligation is
  a fixed two-function enumeration a reviewer must keep honest, not a duty on
  all writers.
- The listing gains a second implementation of the prerequisite gate, over
  frontmatter facts rather than the recursive walk over parsed decks. Two
  divergences are permanent and named: an empty deck and a deck whose body fails
  to load. This is real maintenance surface, taken deliberately because the
  alternative is a null lock state on every row whose index is absent, which
  would move rows in the list.
- An index is addressed by deck id, so two files that share a deck token share
  one index file. This is a property of the addressing, not a bug to fix later,
  and `alix receive` reaches it on an ordinary path because its stage sits in the
  member's own directory. Moving that stage elsewhere is tracked separately.
- **Nothing reaps an orphan index.** A deck deleted, renamed, or moved between
  workspaces leaves its `<deck-id>.json` behind in the old content root forever.
  There is no collector, no doctor rule, and no `alix workspace update` hook, so
  `.alix/index/` grows monotonically with every deck ever opened. Checked and
  NOT a problem: `alix doctor`'s store check targets a specific progress path
  rather than globbing `.alix/**/*.json`, its backup scan matches only backup
  names, and `alix share` excludes anything beginning with a dot, so an orphan
  is inert rather than misreported. It is unbounded growth, not corruption, and
  this record assigns it no owner.
- **A content root that cannot be written produces permanent dotting with no
  diagnosis.** Decks on read-only media, a directory owned by a sync tool,
  Android storage the app cannot write. Every open attempts an index write, the
  best-effort writer swallows the failure, every row stays dotted forever, and
  nothing tells the person why. This record has a law that a listing never fails
  because of an index; it has none for the write path failing.
- **A crash or a full disk leaves `.tmp` siblings in `.alix/index/`.** Nothing
  reaps them either; doctor's backup scan does not match that name.
- **A paired phone re-dots after every sync.** A pull that changes a deck's
  bytes invalidates that deck's index, and a pull never carries one, so the row
  goes dotted until the deck is next opened. On the device this record exists
  for, that is recurring rather than a one-time transition cost.
- **Two profiles over one decks directory share one `.alix/index/`.** Every
  pinned input is profile-independent and every per-person input is read live,
  so this appears safe today. It stops being safe under any variant that pins
  the progress document, which is one of the open shapes.
- Deliberately unsupported: a count of any kind on a row; an approximate or
  last-known state; an index inside the progress document; index synchronization
  between devices; a cross-process invalidation protocol beyond the pins.

## Alternatives considered

- **Show no due signal at all: title and description only, no dot.** NOT
  REJECTED, and until 2026-09-11 not even listed, which an adversarial pass
  called this record's central omission. It is the chosen design minus its
  contested part, and it is the closest reading of what Alex actually asked for
  ("it should show the title and the description"). It matters because D1 alone,
  rows built from frontmatter and the store with no parse, already delivers the
  ENTIRE measured performance win: `deck_status` goes (19.663 of the 32.0 ms
  warm desktop listing), the parse goes, the phone's `decks_loaded=220` becomes
  zero. Everything in this record exists to put the dot back.

  The runtime price of the dot is now measured rather than argued. On the phone
  the same 220-member workspace lists in about 525 ms today, about 23 ms with no
  due signal, and about 60 ms with the index, so the dot costs roughly 37 ms on
  a listing that would otherwise be about 23 ms. That is cheap in absolute terms
  and it is not the argument against it.

  The argument against it is conceptual surface, which the fit gate calls the
  real cost: the dot alone brings a persistence category, a four-pin validity
  algebra, a derivation fixture, a second prerequisite-gate implementation, an
  unsynced writer, a two-function refresh obligation, and a progress-document
  break that hits every reviewed deck. And on the screen this record names as
  its reason to exist, a fresh paired phone, the dot has nothing to show:
  progress documents arrive through a pull, index files never do, nothing writes
  one until a deck is opened, and the bulk action that would have filled them is
  deferred.

  **This is Alex's to rule and it is open.** It is on DESK. No part of this
  record should be built before it is answered, because the answer may be that
  none of it should be.
- **Keep parsing, lazily.** Rejected: lazy parsing still pays the parse on the
  screen where it hurts, which is the phone's first screen.
- **Keep the count.** Rejected by ruling, on product grounds rather than
  technical ones: a backlog number is a discouragement, and every sitting is
  capped at `DEFAULT_MAX_SESSION` anyway, so the number describes a queue the
  learner will not face in one sitting. The technical simplifications that
  follow (a short-circuiting rule, a disjunction instead of a sum, no
  exact-or-null aggregate problem) are a consequence of that ruling, not the
  reason for it.
- **Store the state in the progress document.** Rejected: it is a function of
  deck content, store, configuration, time, and (at Recognize) the augment
  cache; putting it in the store makes a derived value irrecoverable, and every
  deck edit would write the durable store to maintain a cache.
- **One index file per collection instead of per deck.** Rejected, and now with
  a measurement rather than an argument: over index files carrying realistic
  per-card entries, one workspace file's read advantage is 0.53 ms at 10 review
  units per deck and 0.38 ms at 100, while its whole-file rewrite for one deck's
  change costs 0.656 ms and 5.967 ms against 0.031 ms and 0.047 ms for one
  per-deck file. The read advantage is paid once per listing; the write penalty
  is paid on every review. The concern that 220 file opens would
  punish the phone was tested and did not survive: on the partition holding app
  data, 220 opens cost under about 11 ms.
- **An in-memory parse cache only.** This is what the served desktop already
  has, and it is not enough as the primary mechanism: it dies with the process,
  which is the phone's first screen, and only the server builds one. Its warm
  hit costs 0.767 ms while the work it does not elide, `deck_status`, costs
  19.663 ms, so it is not what makes the warm listing fast and keeping it is not
  an alternative to this record.
- **A remembered value marked stale.** Rejected by ruling: a signal that can lie
  costs more trust than it gains.
- **A version integer bumped by hand instead of a derivation pin.** Rejected: it
  depends on a person remembering, and the failure is silent and wrong rather
  than loud and absent.

## Compatibility

The package is pre-1.0 and recognizes no old format. `INDEX_SCHEMA` starts at 1;
a document whose schema differs is absent, with no dedicated message and no
conversion tooling. The document struct denies unknown fields, so a field
removed or renamed later fails to parse and the index is absent rather than
silently misread.

The row contract changes with this decision. No field becomes a nullable number,
because no number is added; instead the row gains an explicit tri-state for its
dot (due, unknown, nothing due) and `new_cards` changes meaning as described
above. The deck drawer endpoint is renamed rather than aliased. The progress
document gains one required field for the remembered cram flag, a plain `bool`
with no serde default. An older document that lacks it fails to parse because
the field is REQUIRED, not because unknown fields are denied; those are two
different guards and an earlier draft of this record conflated them. The blast
radius is NOT narrow, and an earlier draft of this record claimed it was.
`DeckStoreFile.deck` is skipped when empty (`src/store.rs` lines 221-222), so a
document whose `DeckProgress` was never written has nothing to reject. But
`assemble::select` calls `store.set_last_depth` unconditionally
(`src/assemble.rs` line 547, whose own comment records that the write "always
fires even when the built session has nothing due"), so EVERY deck that has ever
had a session built carries a non-empty `deck` object. The break therefore hits
every reviewed deck's progress document, and each failure takes that document's
whole `cards` map with it, surfacing as `state: error`.

That is permitted pre-1.0 and it is the intended break, but the project's hard
rule specifies its SHAPE: production code carries only the new design, AND
disposable tooling outside the repository backs up, converts, verifies and
deletes the old artifacts. This record names no such tooling and the plan
schedules none. Either that tooling exists before the field lands, or the
maintainer accepts losing every deck's review history, and that is not a
decision an ADR may make on his behalf.

## Security

An index is local, machine-written, and read only by alix. It is inside
`.alix/`, which the share tool strips and the recommended ignore block covers,
so it is not published with a shared deck. It stores hashes, labels, ids, and
line numbers rather than answer text, so it does not widen what a leaked folder
reveals beyond what the deck already does.

It is parsed into a fixed struct with unknown fields denied and is trusted only
as far as its pins: a document that does not match the bytes on disk is
discarded rather than believed. An index cannot cause a listing to fail, so a
missing, malformed, truncated, or pin-stale file degrades the picker to its
dotted state rather than breaking it.

The pins are fingerprints of the derivation's inputs, not an authenticator over
the body, and the claim is scoped accordingly: a hand-written document that
copies a genuine index's pins and alters its entries is believed, and the row
then shows whatever it says. That is accepted rather than defended. The file is
local state under the same person's control as the deck it describes, holds no
secret, and reaches no other machine; someone editing their own cache to lie to
themselves has simpler ways to reach the same screen. Defending it would need an
authenticator whose key is not stored beside the body, or re-derivation from
source, which is the cost the design exists to avoid. The body is otherwise
unbounded, so a pathologically large document is a memory question rather than a
correctness one; if that ever matters the answer is a pre-parse byte limit that
fails to ABSENT like any other unreadable document.

Two containment properties follow from decisions made above rather than from
anything added here. On the paired-sync route, a pull is the way another
machine's paired bytes arrive, and index files are excluded from it, so that
route cannot deliver one. It is not the only ingress: `alix receive` lands an
archive too, and a person can copy a directory by hand. Both sit inside the
local-tampering model above. And the derivation pin is computed by the binary
doing the reading, so an index whose derivation differs from this binary's reads
absent without that build having to be recognized or even known about. It does
NOT follow that every other build's index is rejected: the pin is the derived
output of a fixture, not a build identity, so a rebuild of unchanged code and a
code change that leaves the fixture's output untouched both still match, by
Decision's own definition. What keeps another machine's index out is the pull
exclusion above, and only for the ordinary paired-sync route.

## Verification

The implementation plan states the following as executable laws rather than
examples, each able to fail:

- For every fixture deck, store, time, and depth, the rule's verdict over the
  index equals whether an uncapped session build produces a non-empty queue for
  the same inputs, EXCEPT on the two windows the Decision names. Each of those
  two windows owes its own row proving the documented direction: an unstampable
  file reading due where the session serves nothing, and a sibling's token claim
  reading not-due until duplicate repair. Neither row exists yet, so until both
  are written and phase-owned this equality law is stated more broadly than
  anything that gates it.
- Each of the four pins invalidates the index when its input changes, and the
  derivation pin is stable within a process and across processes for unchanged
  code.
- A listing over a fully indexed collection performs zero deck parses, one deck
  byte read and hash per member, one index read per member, and the reads the
  other two pins require: one sidecar read or `stat` per member, and one
  defaults resolution per content root rather than per member. The earlier
  version of this law named only the deck pin's cost, which would have let an
  implementation satisfy it while re-resolving defaults 220 times.
- The dot on a row agrees with what the deck screen computes for the same deck
  against the same store snapshot.
- A collection row is due when any member is due, unknown when no non-trace
  member is due and any non-trace member is unknown, and shows nothing
  otherwise. A TRACE member contributes neither a due nor an unknown; an
  `error` member contributes unknown. This is a law over every combination of
  member states, not an example. The trace clause is not decoration: an earlier
  draft said "any member" twice, which a trace-bearing collection falsifies on
  its first listing, and this record and the spec's D13 disagreed about whether
  a trace is indexed at all.
- Opening a deck that has no index writes one, and opening the same unchanged
  deck again writes none. Without this law the whole list above passes on an
  implementation that never creates an index at all.
- Every deck writer leaves the index reading either ABSENT or exact, including
  the receive path, whose stage shares the live deck's directory. Absent here is
  the READ result, never a requirement to delete the file: a non-refreshing
  writer leaves a pin-stale file in place, and that file is what an update-only
  refresh later repairs.
- A paired pull carries no index file, on the collection path or the single-deck
  path. An unchanged deck whose local index still matches every pin keeps that
  index across the swap.

When the implementation lands, this record moves to Accepted and names the
marker that proves it is in force, per the folder's README.

## Reversal

Evidence that would justify replacing this decision, in the order it is most
likely to arrive:

- An invalidation rate high enough in ordinary use that rows are usually dotted,
  which would mean the mechanism does not deliver the information it exists for.
  This is now the FIRST thing to watch rather than the third, because shipping
  this record without the bulk action means a dotted row clears only by opening
  its deck. Under the count-bearing design the same risk was recorded as
  acceptable for collection rows and unacceptable for deck rows; that
  distinction still holds and is the thing to measure.
- Measurement showing the read-and-hash pass plus the index reads cost as much
  as the parse they replace. This was the first-listed risk before 2026-09-11
  and it was tested: 0.987 ms plus 1.843 ms against 20.4 ms, on warm page cache,
  on a 221-member workspace with one sidecar averaging 10 review units per deck.
  The margin narrows with card density (the index read reaches 11.583 ms at 100
  units per deck) and the risk would return on a dense collection, on one whose
  members mostly carry sidecars, which is unmeasured, or on cold storage, which
  is also unmeasured.
- Measurement on a WARM served desktop, where the comparison is against the
  in-memory cache rather than against a parse. Also tested: the cache elides the
  parse but not `deck_status`, which is 19.663 ms of the 22.9 ms this record
  could attribute to named phases (the warm listing as a whole is 32 ms; the
  remainder is DTO construction, JSON serialization and HTTP), so the
  index is cheaper than the warm cache path too.

Replacing it requires no data migration. The index is derived: delete the
`index/` directories, remove the module and its writers, and the listing returns
to computing from a parse. The row contract would have to change again, which is
the part that costs, and after 1.0 that would need its own migration decision.
