from __future__ import annotations

import os
import signal
import subprocess
import threading
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
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._processes: set[subprocess.Popen[str]] = set()

    def run(
        self, args: list[str], cwd: Path, timeout: float | None = None
    ) -> CommandResult:
        started = time.monotonic()
        with self._lock:
            process = subprocess.Popen(
                args,
                cwd=cwd,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=os.name != "nt",
            )
            self._processes.add(process)
        try:
            try:
                stdout, stderr = process.communicate(timeout=timeout)
                returncode = process.returncode
            except subprocess.TimeoutExpired:
                _terminate(process)
                stdout, stderr = process.communicate()
                stderr = f"{stderr}\ncommand timed out after {timeout} seconds".lstrip()
                returncode = 124
        finally:
            with self._lock:
                self._processes.discard(process)
        return CommandResult(
            returncode=returncode,
            stdout=stdout,
            stderr=stderr,
            duration_seconds=time.monotonic() - started,
        )

    def cancel_all(self) -> None:
        with self._lock:
            processes = tuple(self._processes)
        for process in processes:
            _terminate(process)


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
