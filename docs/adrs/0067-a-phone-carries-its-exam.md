# 0067: A phone carries its exam, and the desktop grades what it is handed

- Status: Accepted
- Recorded: 2026-11-18
- Retrospective: No
- Supersedes: 0066
- Amends: 0010 (remote handlers keep nothing on the desktop, for a
  generated exam the phone has not fetched yet)
- Evidence: pub struct CarriedExam in src/exam_carried.rs
- Evidence: pub fn exam_kept in src/state.rs
- Evidence: fn the_generation_desk_answers_every_request_in_every_state_as_tabled in tests/api.rs
- Evidence: fn the_grading_desk_answers_every_request_in_every_state_as_tabled in tests/api.rs
- Evidence: fn a_ready_exam_survives_a_restart_and_a_deleted_deck_with_equal_question_units in tests/api.rs
- Evidence: fn a_fresh_server_grades_a_request_for_a_deck_it_does_not_serve_and_writes_nothing in tests/api.rs
- Evidence: fn remote_endpoints_never_write_the_server_store in tests/api.rs
- Evidence: fn a_kill_at_each_point_ends_like_an_uninterrupted_apply in mobile/alix/rust/src/api/exam.rs

## Context

ADR 0010 lets a paired phone borrow the desktop's AI for an exam. Until
now the desktop held the exam, its questions and their key points, from
generation until the phone closed it, and the phone held only the answers.
ADR 0066 put that exam into a file so that a desktop restart no longer
lost it.

Three things stayed wrong for the learner. A prepared exam could not be
opened away from the desktop, although answering needs nothing from it.
Answers could outlive their exam: after a restart the desktop reopened a
kept exam only by finding its deck by name and its topics in that deck,
so a deck renamed, moved or changed in the meantime could leave the phone
with answers nothing could grade. And the
desktop's one slot stayed taken for as long as an exam was open, which can
be days.

Grading itself needs none of what made the desktop the keeper: it compares
answers with key points under a strictness, and reads neither the deck nor
its sources.

## Decision

1. An exam is one versioned document: its sitting id, the deck's name,
   the strictness, whether it covers every topic of the deck (so that its
   result can stand as the deck's whole-exam result), the ids of cards the
   start request named that the desktop's deck does not hold (reported by
   the state poll), the topics, each with its id and fingerprint (which the
   phone compares with its deck to tell a stale exam), the questions with
   their key points, and the ideas document as updated at generation.
2. The desktop keeps that document in `exam/remote.json`, in the served
   folder's private state, from the moment the questions exist until the
   phone closes the exam or a new exam start replaces it. The phone
   fetches it by sitting id, which removes nothing, and then closes it;
   the close removes the file. The desktop keeps nothing about an exam
   that was closed.
3. The phone keeps a fetched exam in one file per deck, found by pairing
   root and deck id, together with the answers and, after grading, the
   result. Taking the exam needs no desktop. The file is removed when the
   learner leaves the result or discards the exam.
4. Grading takes the questions, their key points, the answers and the
   strictness in the request. It opens no deck, reads no source, writes
   nothing under the served folder, and runs the model without tools, as
   the grading of every exam now does. Remediation takes the missed points
   per topic in its request, the same way.
5. The desktop runs two independent job slots, one for generation and one
   for grading and remediation, each holding one job. A finished job does
   not hold its slot against a new one. In the grading slot, a request
   equal to the job that is running or has finished with a result is
   answered with that job, not run again.
6. The phone stores a grading result before it applies it, and a stored
   result is never replaced by another grading.
7. When it records a result, the phone replaces the deck's ideas document
   with the copy the exam carried, stamped with what was tested.
8. The key points are plain data on the phone.

ADR 0010's reason stands: the phone remains the only owner of answers,
results, and progress. It now also owns the exam it is taking.

## Consequences

- A prepared exam is taken offline; only generation and grading need the
  desktop.
- A desktop restart, a deck rename, and a deck edit that leaves the exam
  material alone no longer lose an open exam. If the material changed
  since the exam was written, the exam is stale: nothing of it is sent to
  be graded and the phone offers only Discard exam. If the material
  changes after Submit, while the desktop grades, the result is shown and
  not recorded.
- Another deck's exam can be generated while one is being taken.
- "The key points never leave the server" is withdrawn. A learner can
  read the expected points on the phone before answering.
- The desktop grades documents it did not necessarily generate.
- An exam now lives and dies with the phone's file: an uninstall, cleared
  app data, or a removed workspace loses it.
- "No remote call changes a file under the served folder" keeps the one
  exception ADR 0066 named, for a shorter time: while a generated exam
  waits to be closed.
- Not covered: a close that is lost, or that cannot remove the file,
  leaves the desktop's copy ready until a later close removes it or the
  next exam start request does (a start whose removal fails again answers
  an error and the copy stays); a server stopped during generation loses
  that generation; a ready exam nobody fetched is replaced by the next
  exam start request, without a notice to the phone that began it; a deck
  deleted on the desktop while its exam is held leaves that exam on the
  phone unseen; a desktop on another alix version refuses a valid exam
  until both are updated.

## Alternatives considered

### The desktop keeps each exam and the phone caches the questions

The key points stay off the phone and the desktop only grades what it
generated. It keeps the loss this record removes: the exam depends on the
desktop's copy for as long as it is open. And with one slot, an exam taken
over days blocks every other generation, so it needs several kept exams,
which is a larger change than it looks.

### One slot for generation and grading

Simpler to state. A generation takes minutes, and for that long it would
refuse the grading of an exam someone just finished offline.

### Grading in one long request, with no job to poll

No slot, no job identity, nothing to match. A phone that goes to the
background during the minutes a grading takes would lose the reply and
pay for the grading again.

### Recognizing a repeated request by its sitting id

It saves a paid call after a lost reply. An answer left empty can still be
filled before the learner submits again, and the earlier grading would
then be recorded for answers that were never graded. Requests are compared
as a whole instead.

### A sealed or signed answer key

It would keep the learner from reading the expected points. Nobody but the
learner relies on the result today, so it is left until a use case needs
it.

## Compatibility

`exam/remote.json` changes its document; a file in ADR 0066's shape is an
unreadable document when the server next starts, reported and left. The
phone's start records change shape and its earlier exam file is no longer
read.
The remote exam contract changes: the generation reply loses the questions
and everything after generation, and gains a `ready` phase; fetching,
grading, remediation and both closes are new or changed calls
(`docs/API.md`). An exam open across the update is lost on both sides.

## Security

A pairing-token holder can read the key points of an exam that waits to be
closed; discard such an exam by starting another or by closing it, since
the state poll shows its sitting id; and have the desktop run grading and
remediation calls on a document of their own making, up to 4 MiB a
request. Those calls run without tools, so text placed in the key points
reaches a model that cannot fetch or search. The deck name in a request is
used only to look up a served deck's exam effort; a name that is not
served selects the global effort. `docs/security/README.md` records these and
their regression evidence.

## Verification

- A table-driven test per slot checks every state against every request.
  The generation table also checks whether the exam file exists right
  after each request, and the
  grading table counts model calls so that "not run again" is observed.
- `tests/api.rs` restarts a server over an exam that waits to be fetched,
  deletes the deck, and fetches it; and grades a request for a deck the
  server does not serve, with the served folder byte-identical afterwards.
- The phone's tests kill the app after a fetched exam's file is written and
  before it is flagged as fetched, and check that the next poll writes
  nothing, sets the flag and closes. The bridge's tests kill the recording of
  a result at each of its steps (after the result is stored, after progress
  is saved, after the ideas are saved) and compare the progress and ideas
  records with an uninterrupted run.
- `remote_endpoints_never_write_the_server_store` asserts that the only
  file a remote call adds is the exam waiting to be closed, and that it
  is gone after the close.

## Reversal

Replace this record when someone other than the learner relies on an exam
result, which needs the key kept from the phone or sealed; or when the
desktop should grade only what it generated. The files are short-lived,
so a replacement needs no conversion: an exam open across the change is
lost.
