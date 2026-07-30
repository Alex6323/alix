from __future__ import annotations

import re


def next_symmetric_phase(
    phase: str, max_fix_rounds: int, has_defects: bool
) -> str:
    if phase == "IMPLEMENT":
        return "REVIEW_ROUND_1" if max_fix_rounds > 0 else "SCORE"
    match = re.fullmatch(r"REVIEW_ROUND_(\d+)", phase)
    if match is not None:
        round_number = int(match.group(1))
        if has_defects:
            return f"FIX_ROUND_{round_number}"
        if round_number < max_fix_rounds:
            return f"REVIEW_ROUND_{round_number + 1}"
        return "SCORE"
    match = re.fullmatch(r"FIX_ROUND_(\d+)", phase)
    if match is not None:
        round_number = int(match.group(1))
        if round_number < max_fix_rounds:
            return f"REVIEW_ROUND_{round_number + 1}"
        return "SCORE"
    raise ValueError(f"invalid symmetric phase {phase!r}")


def next_asymmetric_phase(
    phase: str, max_fix_rounds: int, suite_passed: bool
) -> str:
    if phase == "IMPLEMENT_PROPERTIES":
        return "RUN"
    if phase == "RUN":
        if suite_passed or max_fix_rounds == 0:
            return "SCORE"
        return "FIX_ROUND_1"
    match = re.fullmatch(r"FIX_ROUND_(\d+)", phase)
    if match is not None:
        return f"RUN_ROUND_{match.group(1)}"
    match = re.fullmatch(r"RUN_ROUND_(\d+)", phase)
    if match is not None:
        round_number = int(match.group(1))
        if suite_passed or round_number >= max_fix_rounds:
            return "SCORE"
        return f"FIX_ROUND_{round_number + 1}"
    raise ValueError(f"invalid asymmetric phase {phase!r}")
