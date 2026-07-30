from __future__ import annotations

import os
import signal
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol


@dataclass(frozen=True)
class CommandResult:
    returncode: int
    stdout: str
    stderr: str
    duration_seconds: float


class Executor(Protocol):
    def run(
        self, args: list[str], cwd: Path, timeout: float | None = None
    ) -> CommandResult: ...


class SubprocessExecutor:
    def run(
        self, args: list[str], cwd: Path, timeout: float | None = None
    ) -> CommandResult:
        started = time.monotonic()
        process = subprocess.Popen(
            args,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=os.name != "nt",
        )
        try:
            stdout, stderr = process.communicate(timeout=timeout)
            returncode = process.returncode
        except subprocess.TimeoutExpired:
            _terminate(process)
            stdout, stderr = process.communicate()
            stderr = f"{stderr}\ncommand timed out after {timeout} seconds".lstrip()
            returncode = 124
        return CommandResult(
            returncode=returncode,
            stdout=stdout,
            stderr=stderr,
            duration_seconds=time.monotonic() - started,
        )


def _terminate(process: subprocess.Popen[str]) -> None:
    if os.name != "nt":
        try:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=2)
            return
        except (ProcessLookupError, subprocess.TimeoutExpired):
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    else:
        process.kill()
