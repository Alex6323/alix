from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import cast


class ReviewManifestError(ValueError):
    pass


@dataclass(frozen=True)
class ReviewCandidate:
    summary: str
    test_name: str
    patch: Path
    real_user_path: str
    impact: str


def load_review_candidates(checkout: Path) -> list[ReviewCandidate]:
    output = (checkout / ".orchestrator-review").resolve()
    manifest = output / "findings.json"
    if not manifest.is_file():
        raise ReviewManifestError(
            "review did not produce `.orchestrator-review/findings.json`"
        )
    with manifest.open(encoding="utf-8") as handle:
        raw = cast(object, json.load(handle))
    if not isinstance(raw, list):
        raise ReviewManifestError("review findings manifest must be a JSON array")
    candidates: list[ReviewCandidate] = []
    for index, value in enumerate(cast(list[object], raw), start=1):
        if not isinstance(value, dict) or not all(
            isinstance(key, str) for key in value
        ):
            raise ReviewManifestError(f"finding {index} must be a JSON object")
        data = cast(dict[str, object], value)
        summary = _required(data, "summary", index)
        test_name = _required(data, "test_name", index)
        patch_name = _required(data, "test_patch", index)
        real_user_path = _required(data, "real_user_path", index)
        impact = _required(data, "impact", index)
        patch = (output / patch_name).resolve()
        if not patch.is_relative_to(output):
            raise ReviewManifestError(
                f"finding {index} test_patch must stay inside `.orchestrator-review`"
            )
        if not patch.is_file():
            raise ReviewManifestError(f"finding {index} test_patch does not exist")
        candidates.append(
            ReviewCandidate(
                summary=summary,
                test_name=test_name,
                patch=patch,
                real_user_path=real_user_path,
                impact=impact,
            )
        )
    return candidates


def _required(data: dict[str, object], key: str, index: int) -> str:
    value = data.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ReviewManifestError(
            f"finding {index} requires a non-empty {key!r} string"
        )
    return value.strip()
