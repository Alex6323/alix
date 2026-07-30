from __future__ import annotations

from dataclasses import dataclass
from pathlib import PurePosixPath


@dataclass(frozen=True)
class PatchValidation:
    reasons: tuple[str, ...]
    review_flags: tuple[str, ...]

    @property
    def rejected(self) -> bool:
        return bool(self.reasons)

    @property
    def needs_human_review(self) -> bool:
        return bool(self.review_flags)


def validate_patch(patch: str) -> PatchValidation:
    reasons: list[str] = []
    review_flags: list[str] = []
    files = _split_files(patch)
    for path, body in files:
        if _forbidden(path):
            reasons.append(f"patch changes forbidden path {path}")
        added = [line[1:] for line in body.splitlines() if line.startswith("+") and not line.startswith("+++")]
        removed = [line[1:] for line in body.splitlines() if line.startswith("-") and not line.startswith("---")]
        if any("mutants::skip" in line or "#![mutants::skip]" in line for line in added):
            reasons.append(f"patch adds mutants::skip in {path}")
        in_tests = path.startswith("tests/") or "#[cfg(test)]" in body
        if in_tests and len(removed) > len(added):
            review_flags.append(f"patch removes unreplaced test lines in {path}")
    return PatchValidation(tuple(dict.fromkeys(reasons)), tuple(review_flags))


def patch_paths(patch: str) -> list[str]:
    return [path for path, _body in _split_files(patch)]


def _forbidden(path: str) -> bool:
    pure = PurePosixPath(path)
    return (
        path in (".cargo/mutants.toml", "mutants.toml", "Makefile")
        or pure.name == "mutants.toml"
        or path == "orchestrator"
        or path.startswith("orchestrator/")
    )


def _split_files(patch: str) -> list[tuple[str, str]]:
    result: list[tuple[str, str]] = []
    path: str | None = None
    lines: list[str] = []
    for line in patch.splitlines():
        if line.startswith("diff --git a/"):
            if path is not None:
                result.append((path, "\n".join(lines)))
            parts = line.split(" b/", 1)
            path = parts[1] if len(parts) == 2 else ""
            lines = [line]
        elif path is not None:
            lines.append(line)
    if path is not None:
        result.append((path, "\n".join(lines)))
    return result
