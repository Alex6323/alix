# Implementation phase

Implement the frozen specification at `{spec_path}` in `{worktree_path}`.
{plan_instruction}

Work independently. Do not inspect another agent's branch, worktree, transcript,
or patch. Keep changes inside this worktree. Do not commit; the orchestrator
creates the phase commit. Do not change `.cargo/mutants.toml`, `mutants.toml`,
`Makefile`, or `orchestrator/`, and do not add mutation-test exclusions.

Every item in the specification's `## API` section must be reachable from the
crate root under the name the specification gives it. Place the code wherever
fits the codebase and re-export as needed: an independently written suite
imports those names from the crate root and cannot see your module layout.

Write tests first for library and error-path behavior. Run `make check` while
iterating and `make gate` before stopping. Finish with a concise result summary.
