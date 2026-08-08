#!/usr/bin/env python3
"""Build the nightly whole-tree mutation matrix."""

import json
import sys


TOTAL_SHARDS = 36
SHARDS_PER_NIGHT = 4
ROTATION_NIGHTS = TOTAL_SHARDS // SHARDS_PER_NIGHT


def rotation_plan(day_of_year: int, requested: str) -> tuple[int, list[int]]:
    if not 1 <= day_of_year <= 366:
        raise ValueError("day of year must be between 1 and 366")

    value = requested.strip()
    if value:
        if not value.isascii() or not value.isdecimal():
            raise ValueError("shard must be a decimal integer")
        shard = int(value)
        if not 0 <= shard < TOTAL_SHARDS:
            raise ValueError(f"shard must be between 0 and {TOTAL_SHARDS - 1}")
        return TOTAL_SHARDS, [shard]

    first = (day_of_year % ROTATION_NIGHTS) * SHARDS_PER_NIGHT
    return TOTAL_SHARDS, list(range(first, first + SHARDS_PER_NIGHT))


def github_outputs(day_of_year: int, requested: str) -> str:
    count, matrix = rotation_plan(day_of_year, requested)
    return f"count={count}\nmatrix={json.dumps(matrix, separators=(',', ':'))}\n"


def main(argv: list[str]) -> int:
    if not 2 <= len(argv) <= 3:
        raise SystemExit("usage: mutants_rotation_plan.py <day-of-year> [shard]")
    try:
        day_of_year = int(argv[1])
        output = github_outputs(day_of_year, argv[2] if len(argv) == 3 else "")
    except ValueError as error:
        raise SystemExit(f"mutants-rotation-plan: {error}") from error
    print(output, end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
