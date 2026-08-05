# Mutation residuals registry

Mutants the rotation and branch gates report as MISSED whose kill is
argued impossible or wrong to write. Every entry carries the argument
and the sweep that confirmed it; an entry without both does not belong
here. When a listed line moves or its code changes, the entry must be
re-argued, not assumed. Remove an entry the moment a legitimate law
kills its mutant.

This registry documents; it does not filter. The nightly rotation
still reports these as missed, and the morning read reconciles the
report against this file. Automating that reconciliation is
{#rotation-residual-filter} on the roadmap.

## Shard 0 (closed 2026-08-05; sweep on aef7b2d: 634 mutants, 539 caught, 24 missed)

Arguments from the codex/rotation-misses-shard0 rounds, accepted
2026-08-05.

### src/ask.rs — random_uuid mixer internals (13 sites)

Lines 74, 75, 79, 80, 81, 86, 87 (xor/or/and and shift substitutions).
The UUID byte mixer is deliberately nondeterministic private state; the
laws pin the contract (RFC version and variant bits, format,
uniqueness over 4,096 draws, every payload bit reachable, avalanche
lower bound), and several mixer mutants sit inside that contract's
tolerance. Pinning exact mixer bytes would test the implementation,
not the behavior. Two former members of this class fell to the 4,096-id
collision and avalanche laws; the rest survive them by construction.

### src/ask.rs:373 — parse_drafted_card `||` to `&&`

Defense in depth: the parser rejects either empty side before this
condition, so no single-empty state reaches it.

### src/ask.rs:1329 — test helper drops `model: None`

`AskConfig::default().model` is `None`; the mutant is byte-equivalent
test code.

### src/answer.rs:121 — hint `-` to `+`

The mutated private count is re-capped by the following
`.take(remaining)` and reset on every edit; no observable state
differs.

### src/cli/deck.rs:390 — augment_cmd line arithmetic (2 mutants)

`synthesize_virtual` stores the line only on the `Card`, and
`WarmItem::from_card` discards it before any output; the value has no
consumer on this path.

### src/cli/launch.rs:135, 188 — LAN announcement (2 sites)

Both require a live host routing-table result; the test-only scope has
no deterministic IP-injection seam, and a network-conditional test
would not be a valid law.

### src/cli/profile.rs:327, 330 — launch_all spawn/wait (4 sites)

Observable only after a real child spawn, which tests must never drive:
the spawn at src/cli/profile.rs:314-318 re-enters the test binary and
fork-bombs (see {#profile-launch-test-fork-bomb}). Kill becomes
possible only after that design decision.

## Table-cards branch gate (2026-08-05 run: 251 mutants, 222 caught, 4 missed)

Recorded when the branch's final gate ran; re-confirm at merge.

### src/parser/mod.rs:507 — table end_line init arithmetic (2 mutants)

The init value is observable only for a table with no rows and no
directive comments, and such a table mints no container id; every
other table overwrites the init before use.

### src/stamp.rs:448 — line_terminator `>` to `>=`

The differing case needs an empty anchor line at file start; anchor
lines are non-empty content lines by construction.

### src/token.rs:30 — mint_row bit packing

Same class as the UUID mixer: random input with no injection seam; the
output stays six valid, varying alphabet characters under the mutant.

## Timeout-detected mutants (ruling pending)

Mutants stopped only by the test timeout (hang classes such as
`mint_row_unique` with its guard deleted, `StudyState::select`).
Whether detection-by-hang counts as caught or missed is an open ruling;
until then they are listed in sweep summaries but not argued here.
