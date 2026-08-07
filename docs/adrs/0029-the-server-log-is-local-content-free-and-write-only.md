# 0029: The server log is local, content-free, and write-only

- Status: Accepted
- Recorded: 2026-08-07
- Retrospective: No

## Context

Alix has no log. The only diagnostic is `ALIX_HTTP_LOG`, one stderr line
per request behind an environment variable that only `make web-debug`
sets, so nothing survives a session a developer was not already watching.

That is affordable while every user can read code, and it stops being
affordable at the first bug report from someone who cannot. A user who
hits the wrong behaviour can say what they saw; the state that explains
it is gone by the time they write. Diagnosing one such report cost about
an hour of reading source and progress JSON to reconstruct which cards
had been served and why (the acquire loop, fixed in `f84b6d8`), and the
inputs to that reconstruction all existed in memory when the card was
served.

Alix is local-first with no accounts and no server we run. There is no
channel by which we could receive diagnostics except a user choosing to
send a file. That makes the file, and what may appear in it, an
architectural decision rather than an implementation detail: it is
personal data at rest, written by us, on a machine we will never see,
designed to be forwarded to strangers.

## Decision

Alix writes a log file itself, always, and constrains it as follows.

1. **Local only.** Alix never transmits the log. It contains no upload
   path, no remote endpoint, and no identifier that would let separate
   reports be correlated. Transmission is the user attaching a file.
2. **Write-only.** Nothing in alix reads the log back. No behaviour, no
   diagnostic, and no recovery path may depend on its contents, so
   deleting it can never change what alix does.
3. **Content-free with respect to learning material.** Card fronts,
   backs, notes, tutor exchanges, exam questions and answers, and deck
   names, titles, and file paths never appear, at any verbosity. Decks
   and cards are identified by their minted ids alone.
4. **Bounded.** The log is size-capped and self-rotating. An always-on
   writer on an unattended machine may not grow without limit.
5. **Outside the user's content.** It is written under the platform
   state directory (`ProjectDirs::state_dir()`, falling back to
   `data_dir()` where the platform has no state directory), never inside
   the decks directory, which holds the user's own files and is walked by
   `share` and `receive`.
6. **Alix's own records only.** The emitter is written here, and no
   logging facade is installed, so no dependency can write into the file.
   Constraint 3 is then structural: it cannot be broken by a dependency
   upgrade, only by an alix commit that a content law would fail.

## Consequences

Easier: a non-technical bug report can carry the evidence needed to
answer it, without a terminal, a flag, or a reproduction.

Harder: diagnosis works from ids, so a report must pair the file with the
user saying which deck they were on. This is a real and accepted cost of
constraint 3. It is cheaper than the alternative, which is receiving a
filename like `depression-notes.md` that the sender did not intend to
share and that we cannot un-see.

Deliberately unsupported: analytics, telemetry, crash reporting, remote
collection, usage measurement, and any feature that would read the log
to change alix's behaviour. Adding a transmitter is not an extension of
this record; it contradicts it and requires a superseding ADR.

## Alternatives considered

**Stderr only, with the user redirecting to a file.** The Unix answer,
free, no rotation policy, no new persistence surface. Rejected because
it only serves users who own a terminal, which is precisely the group
that does not need help. (Alex, 2026-08-07: "we should own writing to a
log file because that's what users (non tech-savvy) should send us.")

**Opt-in behind a flag or environment variable.** Rejected: a log
enabled after the bug is empty when the bug happens. Always-on is what
makes the artifact worth having, and it is what forces constraints 3
and 4.

**A logging facade (`tracing` + `tracing-subscriber`).** The obvious
choice, and the one this record reached for first: levels, target
filtering, and a file writer are a commodity, and the measured cost was
modest (2026-08-07, `cargo metadata` diffed against alix's 201-package
graph: 12 new packages, one maintained family, no new compatibility
family, four crates unifying upward).

Rejected on the invariant, not the weight. Three existing dependencies
already emit `log` records: `tiny_http`, `fontdb` (via `ratex-svg`), and
`zopfli` through `zip`. A facade admits them, and `fontdb` logs font file
PATHS while `zip` handles user file names, so constraint 3 would be a
filter configuration we must never break rather than something a
dependency cannot reach. For a promise made to non-technical users about
a file they forward to strangers, structural beats configured, and a
routine dependency bump must not be able to break it silently.

The commodity argument also shrank on inspection: alix needs two targets,
one verbosity switch, the `key=value` formatting it already uses, and a
size-capped writer that has to be hand-written regardless, since
`tracing-appender` rotates by time rather than bytes. What the facade
would buy is largely filter syntax alix does not need.

Accepted cost: `tiny_http`'s own records go nowhere, so a reported server
hang has no evidence from the server itself. Revisit is tracked as
`{#log-facade-revisit}`, triggered by a real hang report we cannot
diagnose, not by taste.

**Logging names but not content.** Rejected as the sharper half of the
same risk: a deck's filename is often more revealing than its cards.

## Compatibility

No persisted format changes. The log is not a format alix parses, so it
carries no version and no compatibility obligation; its shape may change
freely between releases.

`ALIX_HTTP_LOG` is retired into the new facility rather than kept beside
it. Pre-1.0, so the variable simply stops existing, with no alias and no
recognition of the old name.

## Security

This adds a persistence surface and must be recorded in
`docs/security/README.md`. The threat it introduces is disclosure by the
user's own hand: a file designed to be forwarded, written without
supervision, on a machine we cannot inspect. Constraints 1 and 3 are the
mitigation, and they are invariants rather than defaults precisely
because the disclosure path is social rather than technical.

The log inherits the file permissions of the state directory. It does not
weaken any existing boundary: no new port, no new reader, no new
authentication surface.

## Verification

- A regression test drives a review over a deck with distinctive front,
  back, and note strings and greps the whole log for each, expecting no
  match. It runs at the most verbose level, not the default.
- A companion test asserts no deck filename, title, or path component
  appears for a deck whose name is a known distinctive string.
- A test asserts the cap holds: emitting past it leaves the configured
  file count, each within its size bound.
- The absence of transmission is enforced structurally: the logging
  module has no network dependency, and the lean core (built without the
  server) must continue to compile with it.
- Constraint 6 is enforced by there being no logging facade to install:
  adding `log`, `tracing`, or any subscriber as a direct dependency is
  the change that would need this record superseded, and
  `make deps-check` is where an accidental one surfaces.

## Reversal

Evidence that would justify replacing this record: reports arriving with
logs that cannot be diagnosed from ids alone, repeatedly and after the
reporter has been asked which deck they used. That would justify a
superseding ADR relaxing constraint 3, most likely to a per-report opt-in
where the user chooses to include names, rather than a blanket change.

No migration is required to reverse any of this: the log is not read by
alix, so its shape and contents can change or the file can be removed
entirely without touching persisted state.
