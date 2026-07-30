# Implementation phase

Implement the frozen specification at `{spec_path}` in `{worktree_path}`.
{plan_instruction}

Work independently. Do not inspect another agent's branch, worktree, transcript,
or patch. Keep changes inside this worktree. Do not commit; the orchestrator
creates the phase commit. Do not change `.cargo/mutants.toml`, `mutants.toml`,
`Makefile`, or `orchestrator/`, and do not add mutation-test exclusions.

Write tests first for library and error-path behavior. Run `make check` while
iterating and `make gate` before stopping. Finish with a concise result summary.
