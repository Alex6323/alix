# Verified-defect fix phase

Fix the verified regression tests already applied in `{worktree_path}`.
The frozen specification is at `{spec_path}`.

The tests are evidence supplied verbatim. Do not edit, move, or delete them.
Fix the production defect without restructuring toward another implementation.
Do not inspect another agent's code. Do not commit. Run `make check` while
iterating and `make gate` before stopping.

Verified failures:
{failure_summary}
