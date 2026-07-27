# 0015: Frozen source snapshots

- Status: Superseded by
  [ADR 0021](0021-deck-owned-frozen-assets.md)
- Recorded: 2026-07-24
- Retrospective: Yes
- Refined by: [ADR 0020](0020-source-excerpt-integrity.md)

## Decision history

Commit `a14d919` froze trace excerpts into generated workspaces on 2026-06-21.
Commit `6f7e92e` extended snapshotting to cited fact cards on 2026-06-23.
Commit `a27a643` made the tutor use the frozen excerpt as its anchor while
reading live source for context and detecting drift. Commit `ab35532` hardened
workspace snapshot resolution on 2026-07-01.

The later Markdown deck migration preserved the same `source`, `origin`, and
`<!-- at: ... -->` semantics.

## Context

Source-grounded cards and trace checkpoints cite exact lines. If review reads
only the live file, an edit can silently change the evidence under an existing
question or make a locator point at unrelated content. Copying an entire
repository would make workspaces heavy and could capture material the learner
did not intend to include.

A generated workspace must remain portable and reviewable offline while still
being able to tell the learner when its source has evolved.

## Decision

When Alix builds a source-grounded workspace, it copies each cited contiguous
excerpt into the workspace's `assets/` directory and rewrites the card's
`<!-- at: ... -->` locator to that frozen asset. Fact cards and trace
checkpoints use the same snapshot mechanism.

The frozen excerpt is the learning anchor. It travels with the workspace and
does not change when the live source changes.

The workspace records `origin`, the live source root from which the snapshots
were made. When available:

- the tutor may read live source for surrounding context while keeping the
  frozen excerpt authoritative for the card under discussion; and
- doctor compares frozen evidence with live source and reports drift.

Snapshotting copies cited excerpts rather than complete source trees. A cited
deck whose local source cannot be frozen fails or reports the missing evidence
loudly instead of producing a silently empty `assets/` directory.

## Consequences

- Generated workspaces are self-contained and work offline.
- Review questions retain the evidence they were authored against.
- Workspaces duplicate small source excerpts.
- Live-source changes do not silently rewrite learning material.
- Drift is visible but does not mutate the frozen card automatically.
- Tutor context can be current while the learner still sees the historical
  anchor.
- Sharing a workspace also shares the captured excerpts.

## Alternatives considered

### Resolve every citation against live source

This avoids copies but makes review non-reproducible and turns line edits into
silent evidence changes.

### Copy the complete source tree

Whole-tree copies improve surrounding context but increase size, duplicate
irrelevant or sensitive files, and weaken data minimization.

### Store snapshots in personal progress

The evidence belongs to the portable learning workspace, not to one learner's
review history. Hiding it in a progress document would break sharing and ordinary
inspection.

### Store remote URLs without captured evidence

Links may change, disappear, or be unavailable offline. They do not preserve
the exact material used to construct the card.

### Refresh snapshots automatically

Automatic refresh would erase the historical anchor and could make the answer
change without explicit review.

## Compatibility

`source`, `origin`, frozen `assets/`, and `<!-- at: ... -->` locators form a
portable workspace contract. Exact generated asset names are implementation
details, but moving or rewriting snapshots must preserve locator resolution
and the live-origin relationship.

## Security

Snapshotting is a data-export boundary. Path resolution must stay within the
intended source scope, and generated workspaces must capture cited excerpts
rather than arbitrary surrounding files. A learner must understand that
sharing the workspace shares the frozen source text.

Live tutor grounding grants the configured backend read access to the origin
root. The frozen excerpt remains the bounded evidence supplied directly in the
prompt.

## Verification

- `src/explore.rs` snapshots cited fact and trace material and reports missing
  sources.
- `src/trace_ai.rs` performs excerpt copying and locator rewriting.
- `src/source.rs` resolves cited excerpts and frozen-source provenance.
- `src/trace.rs` compares frozen trace evidence with live origins.
- `src/ask.rs` anchors tutor prompts in frozen evidence and adds live context
  deliberately.
- Doctor and workspace tests cover missing snapshots, locator resolution, and
  drift reporting.
- The book documents freezing and sharing behavior for workspaces and traces.

## Reversal

Replace snapshots when another mechanism preserves exact evidence, offline
portability, data minimization, and drift visibility with less cost. A
migration must keep existing locators resolvable and must not silently replace
the evidence a learner studied.
