#!/usr/bin/env python3
"""Build a validated GitHub Actions matrix for the branch mutation gate."""

import json
import sys


DEFAULT_SHARDS = 4
MAX_SHARDS = 64


def shard_plan(requested: str) -> tuple[int, list[int]]:
    value = requested.strip()
    if not value:
        count = DEFAULT_SHARDS
    elif not value.isascii() or not value.isdecimal():
        raise ValueError("shards must be a decimal integer")
    else:
        count = int(value)
    if not 1 <= count <= MAX_SHARDS:
        raise ValueError(f"shards must be between 1 and {MAX_SHARDS}")
    return count, list(range(count))


def github_outputs(requested: str) -> str:
    count, matrix = shard_plan(requested)
    return f"count={count}\nmatrix={json.dumps(matrix, separators=(',', ':'))}\n"


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        raise SystemExit("usage: mutants_branch_plan.py [shards]")
    try:
        output = github_outputs(argv[1] if len(argv) == 2 else "")
    except ValueError as error:
        raise SystemExit(f"mutants-branch-plan: {error}") from error
    print(output, end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
