# 0066: A phone's open exam is kept in the desktop's private state

- Status: Accepted
- Recorded: 2026-11-17
- Retrospective: No
- Evidence: fn a_paused_remote_exam_survives_a_server_restart_until_it_is_closed in tests/api.rs
- Evidence: fn remote_endpoints_never_write_the_server_store in tests/api.rs
- Evidence: fn exam_kept in src/state.rs
- Amends: 0010 (remote handlers keep nothing on the desktop, for an open exam's sitting)

## Context

ADR 0010 lets a paired phone borrow the desktop's AI for an exam: the desktop
generates the questions and holds their key points, the phone holds the
answers and records the result. The desktop held the open sitting only in
memory. Generation takes minutes and an exam can stay paused for days, so a
desktop that restarted or shut down in that time lost the questions. The
phone was left with saved answers to an exam that could no longer be graded,
and the generation had to be run again.

ADR 0010's evidence test reads wider than its decision. The decision keeps
remote handlers away from the desktop progress store; the test asserts that
no file under the served folder changes. A sitting written to disk has to
give up the wider reading somewhere.

## Decision

1. Once a remote exam's questions exist, the server writes the sitting to
   `exam/remote.json` in the served folder's private state before it reports
   the questions. A sitting that cannot be written fails its generation.
2. The file holds what the desktop computed and needs for grading: the
   sitting id, the deck, the strictness, each topic with its fingerprint, the
   named cards the deck does not hold, the questions with their key points,
   and the ideas document. It holds no answer, grade, progress, or mastery.
3. At start, the server reopens a kept sitting in `answering`, unanswered,
   under the same sitting id. The phone still holds the answers and submits
   them again.
4. Closing the exam removes the file. A second start is refused while a
   sitting is open, so the folder holds one such file at most.
5. A kept file that cannot be read, or that the deck can no longer carry, is
   reported and left in place, and the slot starts idle.

ADR 0010's reason stands: the phone remains the only owner of answers,
results, and progress.

## Consequences

- A paused phone exam survives a desktop restart, with the phone unchanged.
- "No remote call changes a file under the served folder" has a named
  exception, asserted where it happens: while an exam is open, the kept
  sitting is the only added file.
- The key points of an open exam are on disk in the served folder, readable
  by anything that reads the folder. The browser exam's prepared file already
  puts the same kind of content there.
- Share leaves private state out, and paired sync accepts no private-state
  path other than progress documents, so neither carries the file.
- Whatever copies the folder as files (a backup, a file synchronizer, version
  control without an ignore rule) copies it too, as it copies progress.
- Grading still needs the desktop that holds the file.
- Not covered: a server stopped during generation loses that generation; a
  server restarted after grading reopens the exam unanswered; a close whose
  file cannot be removed still succeeds, and the next start reopens that
  exam; a deck renamed or moved while its exam is open is no longer found
  under the kept name, so the file stays and the slot starts idle.

## Alternatives considered

### The platform state directory, beside the server log

This keeps the served folder untouched. Files there are named after how the
server was launched, so the same folder launched another way would not find
its exam. It would also put exam content, built from the user's sources, into
a directory that so far holds only content-free logs and that nothing cleans,
and it would split exam state across two places.

### Reuse the browser exam's prepared file

That file is one per deck and the browser reads it as its own exam. It
carries no sitting id, no named-but-unknown cards, and no ideas document, and
a phone exam appearing as a browser exam would give one sitting two front
ends.

### The phone carries the whole sitting

The phone would store the questions and key points and hand them back for
grading, so the desktop keeps nothing and any paired desktop serving the deck
could grade. This follows ADR 0010's ownership rule more closely. It is not
adopted here because it changes the remote exam contract and needs the phone
and the desktop to change together. A record adopting it would supersede this
one.

## Compatibility

`exam/remote.json` is a new persisted document, version 1, and rejects
unknown fields. `RemoteExamDto` is unchanged. `docs/API.md` states the kept
sitting and the restart behavior.

## Security

A pairing-token holder can make the desktop write this one file at a fixed
path. The desktop generates its questions and key points. Three values in it
come from the phone's start request, inside the remote body cap: the ideas
document it starts from, parsed into its typed shape, with the text of each
idea asked about repeated in its question; the deck's catalog name, kept only
when the catalog resolved it; and the names of requested cards the deck does
not hold, kept as given. A restore resolves the deck name through the catalog
again; nothing else in the file is used to find a file.
`docs/security/README.md` records the write and its regression evidence.

## Verification

- `tests/api.rs` restarts a real server over a paused exam, grades it against
  the kept key points, and checks that a close removes the file and that a
  closed exam does not come back.
- `tests/api.rs` leaves an unreadable kept file in place with the slot idle,
  and fails a generation whose sitting cannot be written.
- `remote_endpoints_never_write_the_server_store` asserts that an open exam
  adds no file but the kept sitting, that grading and remediation leave that
  file's bytes unchanged, and that the folder is unchanged after the close.
- `src/exam.rs` holds a generation at its keep write, cancels it as a close
  does, and checks that no file is written afterwards.
- `src/exam.rs` tests reopening over the kept topics and fingerprints, the
  refusals, and an unreadable or missing file.

## Reversal

Supersede this record when the phone carries the sitting itself, or when the
desktop may hold more than one remote sitting. The file is short-lived, so a
replacement needs no conversion: a sitting open across the change is lost.
