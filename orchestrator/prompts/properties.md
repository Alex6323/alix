# Independent property-author phase

Using only the frozen specification at `{spec_path}` and the public API stub in
`{worktree_path}`, create property and invariant tests under `tests/`.
{plan_instruction}

You have zero implementation visibility. Do not seek or inspect the implementer
worktree. The stub is the complete public contract; if it is insufficient, make
no speculative API choice and finish with `SPEC BUG:` followed by the missing
contract. Do not edit the stub or Cargo manifest. Do not commit.

Use proptest for general laws and focused unit cases for named boundaries.
Run `cargo test --no-run` before stopping.
