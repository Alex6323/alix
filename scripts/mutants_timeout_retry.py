#!/usr/bin/env python3
"""Retry each cargo-mutants timeout once as an exact one-mutant run."""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import math
import os
import pathlib
import re
import subprocess
import sys
import time
from collections.abc import Callable, Sequence


@dataclasses.dataclass(frozen=True)
class ParsedOutcome:
    name: str
    summary: str
    runtime_seconds: float
    timeout_phase_seconds: float | None


@dataclasses.dataclass(frozen=True)
class RetryRecord:
    mutant: str
    original_runtime_seconds: float
    focused_runtime_seconds: float
    focused_timeout_seconds: int
    focused_outcome: str
    open: bool
    runner_exit: int
    detail: str = ""


@dataclasses.dataclass(frozen=True)
class FocusedRun:
    exit_code: int
    detail: str = ""


Runner = Callable[[str, int, pathlib.Path], int | FocusedRun]


MUTANT_LOCATOR = re.compile(
    r"^(?P<file>[^:\r\n]+):(?P<line>\d+):\d+: ",
)


def read_lines(path: pathlib.Path) -> list[str]:
    try:
        return [
            line.strip()
            for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
    except OSError:
        return []


def focused_command(
    mutant: str,
    timeout_seconds: int,
    selection_diff: pathlib.Path | None = None,
    *,
    list_only: bool = False,
) -> list[str]:
    """Build the cargo-mutants command for exactly one named mutant."""
    command = [
        "cargo",
        "mutants",
        "--baseline",
        "skip",
        "--jobs",
        "1",
        "--timeout",
        str(timeout_seconds),
        "--re",
        f"^(?:{re.escape(mutant)})$",
    ]
    if selection_diff is not None:
        command.extend(["--in-diff", str(selection_diff)])
    if list_only:
        command.extend(["--list", "--colors", "never"])
    return command


def _mutant_name(scenario: object) -> str | None:
    if not isinstance(scenario, dict):
        return None
    mutant = scenario.get("Mutant")
    if not isinstance(mutant, dict):
        return None
    name = mutant.get("name")
    return name if isinstance(name, str) else None


def _parse_outcomes(path: pathlib.Path) -> dict[str, ParsedOutcome]:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(
            f"cannot read cargo-mutants outcomes {path}: {error}"
        ) from error
    raw_outcomes = document.get("outcomes") if isinstance(document, dict) else None
    if not isinstance(raw_outcomes, list):
        raise ValueError(f"cargo-mutants outcomes have no list at {path}")

    parsed = {}
    for raw in raw_outcomes:
        if not isinstance(raw, dict):
            continue
        name = _mutant_name(raw.get("scenario"))
        summary = raw.get("summary")
        phases = raw.get("phase_results")
        if (
            name is None
            or not isinstance(summary, str)
            or not isinstance(phases, list)
        ):
            continue
        runtime = 0.0
        timeout_phase = None
        for phase in phases:
            if not isinstance(phase, dict):
                continue
            duration = phase.get("duration")
            if isinstance(duration, (int, float)):
                runtime += float(duration)
                if phase.get("process_status") == "Timeout":
                    timeout_phase = float(duration)
        parsed[name] = ParsedOutcome(name, summary, runtime, timeout_phase)
    return parsed


def _write_lines(path: pathlib.Path, lines: Sequence[str]) -> None:
    contents = "".join(f"{line}\n" for line in lines)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(contents, encoding="utf-8")
    os.replace(temporary, path)


def _append_unique(existing: list[str], additions: Sequence[str]) -> list[str]:
    seen = set(existing)
    for addition in additions:
        if addition not in seen:
            existing.append(addition)
            seen.add(addition)
    return existing


def _retry_directory(out_dir: pathlib.Path, index: int, mutant: str) -> pathlib.Path:
    digest = hashlib.sha256(mutant.encode("utf-8")).hexdigest()[:16]
    return out_dir / "retries" / f"{index:04d}-{digest}"


def _selection_diff(
    mutant: str,
    out_dir: pathlib.Path,
    source_root: pathlib.Path,
) -> pathlib.Path:
    """Admit one source line before the name regex sees generated mutants.

    cargo-mutants 27.1 lets delete-field mutants bypass --re. Its --in-diff
    filter applies earlier, so a no-op diff of the target line bounds that
    structural family too. The list precheck still proves the final cardinality.
    """
    match = MUTANT_LOCATOR.match(mutant)
    if match is None:
        raise ValueError(f"mutant name has no source locator: {mutant}")
    relative = pathlib.PurePosixPath(match.group("file"))
    if relative.is_absolute() or ".." in relative.parts:
        raise ValueError(f"mutant source path escapes the repository: {relative}")
    root = source_root.resolve()
    source = root.joinpath(*relative.parts).resolve()
    if root != source and root not in source.parents:
        raise ValueError(f"mutant source path escapes the repository: {relative}")
    try:
        lines = source.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as error:
        raise ValueError(f"cannot read mutant source {relative}: {error}") from error
    line_number = int(match.group("line"))
    if line_number == 0 or line_number > len(lines):
        raise ValueError(f"mutant source line is outside {relative}: {line_number}")
    source_line = lines[line_number - 1]
    contents = (
        f"diff --git a/{relative} b/{relative}\n"
        f"--- a/{relative}\n"
        f"+++ b/{relative}\n"
        f"@@ -{line_number} +{line_number} @@\n"
        f"-{source_line}\n"
        f"+{source_line}\n"
    )
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / "selection.diff"
    path.write_text(contents, encoding="utf-8")
    return path


def run_focused_mutant(
    mutant: str,
    timeout_seconds: int,
    out_dir: pathlib.Path,
    source_root: pathlib.Path | None = None,
) -> FocusedRun:
    root = (source_root or pathlib.Path.cwd()).resolve()
    try:
        selection_diff = _selection_diff(mutant, out_dir, root)
    except ValueError as error:
        return FocusedRun(1, str(error))

    list_environment = os.environ.copy()
    list_environment.pop("CARGO_MUTANTS_OUTPUT", None)
    # Listing performs no build and proves the identical run command selects one.
    list_result = subprocess.run(
        focused_command(mutant, timeout_seconds, selection_diff, list_only=True),
        check=False,
        cwd=root,
        env=list_environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if list_result.returncode != 0:
        detail = list_result.stderr.strip()
        suffix = f": {detail}" if detail else ""
        return FocusedRun(
            list_result.returncode,
            f"focused selection list exited {list_result.returncode}{suffix}",
        )
    selected = [
        line.strip()
        for line in list_result.stdout.splitlines()
        if line.strip()
    ]
    if selected != [mutant]:
        return FocusedRun(
            1,
            f"focused selection expected one exact mutant, listed {len(selected)}: "
            f"{selected[:5]!r}",
        )

    environment = os.environ.copy()
    environment["CARGO_MUTANTS_OUTPUT"] = str(out_dir)
    result = subprocess.run(
        focused_command(mutant, timeout_seconds, selection_diff),
        check=False,
        cwd=root,
        env=environment,
    )
    return FocusedRun(result.returncode)


def _focused_record(
    mutant: str,
    original: ParsedOutcome,
    retry_out: pathlib.Path,
    timeout_seconds: int,
    run: FocusedRun,
    wall_seconds: float,
) -> RetryRecord:
    if run.detail:
        return RetryRecord(
            mutant=mutant,
            original_runtime_seconds=original.runtime_seconds,
            focused_runtime_seconds=wall_seconds,
            focused_timeout_seconds=timeout_seconds,
            focused_outcome="error",
            open=True,
            runner_exit=run.exit_code,
            detail=run.detail,
        )
    try:
        # --output names a parent; cargo-mutants creates mutants.out below it.
        focused = _parse_outcomes(retry_out / "mutants.out" / "outcomes.json").get(
            mutant
        )
        if focused is None:
            raise ValueError("the exact mutant has no focused outcome")
    except ValueError as error:
        return RetryRecord(
            mutant=mutant,
            original_runtime_seconds=original.runtime_seconds,
            focused_runtime_seconds=wall_seconds,
            focused_timeout_seconds=timeout_seconds,
            focused_outcome="error",
            open=True,
            runner_exit=run.exit_code,
            detail=str(error),
        )

    names = {
        "CaughtMutant": "caught",
        "MissedMutant": "missed",
        "Timeout": "timeout",
        "Unviable": "unviable",
    }
    outcome = names.get(focused.summary, "error")
    detail = "" if outcome != "error" else f"unexpected summary {focused.summary}"
    return RetryRecord(
        mutant=mutant,
        original_runtime_seconds=original.runtime_seconds,
        focused_runtime_seconds=focused.runtime_seconds,
        focused_timeout_seconds=timeout_seconds,
        focused_outcome=outcome,
        open=outcome != "caught",
        runner_exit=run.exit_code,
        detail=detail,
    )


def run_timeout_retries(
    out_dir: pathlib.Path,
    runner: Runner = run_focused_mutant,
) -> list[RetryRecord]:
    """Retry initial timeouts and reclassify only conclusive focused results."""
    timeouts = read_lines(out_dir / "timeout.txt")
    if not timeouts:
        return []
    if len(timeouts) != len(set(timeouts)):
        raise ValueError("timeout.txt contains a duplicate mutant")
    initial_path = out_dir / "timeout-initial.txt"
    report_path = out_dir / "timeout-retries.json"
    if initial_path.exists() or report_path.exists():
        raise ValueError("timeout retries have already been recorded for this output")

    originals = _parse_outcomes(out_dir / "outcomes.json")
    records = []
    for index, mutant in enumerate(timeouts, start=1):
        original = originals.get(mutant)
        if original is None or original.summary != "Timeout":
            raise ValueError(f"timeout outcome missing for {mutant}")
        if original.timeout_phase_seconds is None:
            raise ValueError(f"timeout phase duration missing for {mutant}")
        focused_timeout = max(1, math.ceil(original.timeout_phase_seconds * 2))
        retry_out = _retry_directory(out_dir, index, mutant)
        started = time.monotonic()
        runner_result = runner(mutant, focused_timeout, retry_out)
        run = (
            runner_result
            if isinstance(runner_result, FocusedRun)
            else FocusedRun(runner_result)
        )
        wall_seconds = time.monotonic() - started
        records.append(
            _focused_record(
                mutant,
                original,
                retry_out,
                focused_timeout,
                run,
                wall_seconds,
            )
        )

    _write_lines(initial_path, timeouts)
    caught = read_lines(out_dir / "caught.txt")
    missed = read_lines(out_dir / "missed.txt")
    caught = _append_unique(
        caught,
        [record.mutant for record in records if record.focused_outcome == "caught"],
    )
    missed = _append_unique(
        missed,
        [record.mutant for record in records if record.focused_outcome == "missed"],
    )
    open_timeouts = [
        record.mutant
        for record in records
        if record.focused_outcome not in {"caught", "missed"}
    ]
    _write_lines(out_dir / "caught.txt", caught)
    _write_lines(out_dir / "missed.txt", missed)
    _write_lines(out_dir / "timeout.txt", open_timeouts)
    report_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "retries": [dataclasses.asdict(record) for record in records],
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    return records


def _summary_cell(text: str) -> str:
    return text.replace("|", r"\|").replace("`", "'")


def render_summary(records: Sequence[RetryRecord]) -> str:
    lines = [
        "## Mutation timeout retries",
        "",
        "| mutant | original runtime | focused budget | focused outcome "
        "| focused runtime | open |",
        "|---|---:|---:|---|---:|---|",
    ]
    for record in records:
        lines.append(
            f"| `{_summary_cell(record.mutant)}` "
            f"| {record.original_runtime_seconds:.3f}s "
            f"| {record.focused_timeout_seconds}s "
            f"| {record.focused_outcome} "
            f"| {record.focused_runtime_seconds:.3f}s "
            f"| {'yes' if record.open else 'no'} |"
        )
        if record.detail:
            lines.append(f"| detail | {_summary_cell(record.detail)} | | | | |")
    return "\n".join(lines) + "\n"


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out_dir", type=pathlib.Path)
    arguments = parser.parse_args(argv)
    try:
        records = run_timeout_retries(arguments.out_dir)
    except (OSError, ValueError) as error:
        print(f"mutants-timeout-retry: {error}", file=sys.stderr)
        return 1
    if not records:
        print("mutants-timeout-retry: no timeouts to retry")
        return 0

    summary = render_summary(records)
    print(summary, end="")
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with pathlib.Path(summary_path).open("a", encoding="utf-8") as stream:
            stream.write(summary)
    return 1 if any(record.focused_outcome == "error" for record in records) else 0


if __name__ == "__main__":
    sys.exit(main())
