# 0004: FSRS as the single scheduler

- Status: Accepted
- Recorded: 2026-07-24
- Retrospective: Yes

## Decision history

Commit `5fc6799` added the `rs-fsrs` engine on 2026-07-04. Commit `8cd5a51`
made FSRS the sole scheduler with short-term learning enabled and removed the
Leitner and SM-2 paths. Commits `43d4476`, `f4171e2`, and `5aaf166` aligned
graduation, retirement, and exam readiness with FSRS state. Commit `5f5618c`
removed the last stored Leitner stage on 2026-07-06.

## Context

Maintaining several scheduling algorithms would multiply persisted states,
product explanations, readiness rules, and migration paths. It would also make
results hard to compare because each scheduler would interact differently with
Alix's acquisition, review depths, exams, and retirement rules.

FSRS provides an evidence-based memory model and short-term learning steps
without requiring a parallel stage ladder.

## Decision

Alix uses FSRS-5 through `rs-fsrs` as its sole spaced-repetition scheduler.
Short-term learning is enabled. There is no user-selectable scheduler kind.

Alix maps its three review outcomes to FSRS ratings:

- Failed becomes Again.
- Partial becomes Hard.
- Passed becomes Good.

Alix does not emit Easy. Acquisition requires two full Good outcomes before a
new card graduates to Review; a failure resets that acquisition count.

Maturity, due-ness, retirement, and exam readiness derive from persisted FSRS
state and intervals. Alix does not maintain a parallel Leitner or SM-2 stage
ladder.

## Consequences

- Scheduling behavior has one implementation and one persisted model.
- Product concepts can be explained through due dates, stability, and
  intervals instead of competing scheduler vocabularies.
- Upgrading FSRS or changing its parameters can alter future schedules and
  must be treated as a data migration.
- Alix-specific acquisition and depth rules remain explicit around the FSRS
  engine.
- Users cannot switch algorithms to tune behavior per deck.

## Alternatives considered

### Keep Leitner as a selectable scheduler

This retained familiar stages but required parallel scheduling, retirement,
and persistence behavior. It was removed rather than maintained as a second
product.

### Keep SM-2 as a selectable scheduler

SM-2 would add another model and migration surface without evidence that the
choice improves Alix's learning workflow.

### Let each client schedule independently

Client scheduling would drift across web, CLI, and mobile and would violate
the shared-core decisions in ADRs 0006 and 0007.

## Compatibility

`FsrsState` in each deck's progress document, including stability, difficulty, state,
intervals, timestamps, and Alix's learning-Good count, is persisted learner
state. The store owns a representation independent of the dependency's Rust
type so a crate update does not silently redefine the file format.

## Security

This decision adds no remote trust boundary. Scheduler inputs remain
untrusted in the ordinary data-integrity sense: malformed or incompatible
stored state must fail or migrate explicitly rather than produce fabricated
progress.

## Verification

- `src/scheduler.rs` owns grade mapping, short-term scheduling, graduation,
  due-ness, and retirement tests.
- `src/store.rs` owns the serialized FSRS representation and compatibility
  defaults.
- `Cargo.toml` pins the `rs-fsrs` dependency used by the engine.
- Session and exam tests verify that readiness derives from scheduling state.

## Reversal

A replacement requires measured evidence that another scheduler better serves
Alix's learning goals, a deterministic migration for every stored schedule,
and validation of acquisition, exams, maturity, retirement, and all clients.
It is an architectural replacement, not a configuration toggle.
