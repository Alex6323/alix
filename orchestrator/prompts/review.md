# Differential review phase

Review the neutral checkout at `{worktree_path}` against the frozen
specification at `{spec_path}`. Authorship and branch history are intentionally
absent.

Report only deterministic user-relevant defects. For each defect, create the
smallest standalone test-only patch that compiles but fails against this exact
checkout. Run its exact test by itself. Do not edit production code or existing
tests. Add new integration tests only under `tests/`. If no such defect exists,
emit an empty findings array.

The orchestrator will mechanically apply your test patch, compile it, prove it
fails, revert it, and prove the baseline passes. Preferences and non-reproduced
concerns are recorded but never actioned. A finding is also invalid without an
ordinary supported user action or concurrency/filesystem path that triggers it
and the concrete user-visible consequence. Be candid when a path is
lower-frequency. Do not present an architectural preference or adversarial-only
case as a user defect.

Do not leave test edits in the checkout. Write each test patch under
`.orchestrator-review/`, plus `.orchestrator-review/findings.json` with this
shape:

```json
[
  {{
    "summary": "one line",
    "test_name": "exact_test_name",
    "test_patch": "F1.patch",
    "real_user_path": "ordinary trigger and why users encounter it",
    "impact": "concrete user-visible consequence"
  }}
]
```

Every patch must apply independently to the untouched checkout. Do not commit.
