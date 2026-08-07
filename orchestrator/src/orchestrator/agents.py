from __future__ import annotations

import json
import threading
import time
from pathlib import Path
from typing import Protocol, cast

from orchestrator.commands import Executor, SubprocessExecutor
from orchestrator.models import AgentName, Backend, Invocation
from orchestrator.storage import append_progress


class Invoker(Protocol):
    def invoke(
        self, agent: AgentName, prompt: str, cwd: Path, timeout: float
    ) -> Invocation: ...


class SubprocessInvoker:
    def __init__(
        self,
        run_dir: Path,
        executor: Executor | None = None,
        models: dict[str, str] | None = None,
        backends: dict[str, str] | None = None,
        heartbeat_seconds: float = 15.0,
    ) -> None:
        self.run_dir = run_dir
        self.executor = executor or SubprocessExecutor()
        self.models = dict(models or {})
        self.backends = dict(backends or {})
        self.heartbeat_seconds = heartbeat_seconds
        self._sequence = max(
            (
                int(path.stem.split("-", 1)[0])
                for path in (self.run_dir / "transcripts").glob("*.txt")
                if path.stem.split("-", 1)[0].isdigit()
            ),
            default=0,
        )
        self._sequence_lock = threading.Lock()

    def backend_for(self, agent: AgentName) -> Backend:
        backend = self.backends.get(agent, "claude")
        if backend not in ("claude", "codex"):
            raise ValueError(f"seat {agent} has an unknown backend {backend!r}")
        return cast(Backend, backend)

    def cancel_all(self) -> None:
        cancel = getattr(self.executor, "cancel_all", None)
        if callable(cancel):
            cancel()

    def invoke(
        self, agent: AgentName, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        with self._sequence_lock:
            self._sequence += 1
            sequence = self._sequence
        backend = self.backend_for(agent)
        stem = f"{sequence:03d}-{agent}-{backend}"
        transcript = self.run_dir / "transcripts" / f"{stem}.txt"
        patch_path = self.run_dir / "patches" / f"{stem}.patch"
        baseline = _git(cwd, "rev-parse", "HEAD").strip()
        started = time.monotonic()
        append_progress(self.run_dir, f"agent {agent} started")
        command = command_for(backend, prompt, self.models.get(agent))
        stopped = threading.Event()

        def heartbeat() -> None:
            while not stopped.wait(self.heartbeat_seconds):
                elapsed = time.monotonic() - started
                append_progress(
                    self.run_dir,
                    f"agent {agent} heartbeat after {elapsed:.0f}s; "
                    f"{_change_summary(cwd)}",
                )

        heartbeat_thread = threading.Thread(target=heartbeat, daemon=True)
        heartbeat_thread.start()
        try:
            result = self.executor.run(command, cwd, timeout)
        except Exception as error:
            append_progress(self.run_dir, f"agent {agent} failed: {error}")
            raise
        finally:
            stopped.set()
            heartbeat_thread.join()
        append_progress(
            self.run_dir,
            f"agent {agent} finished with exit {result.returncode} "
            f"after {result.duration_seconds:.1f}s",
        )
        _git(cwd, "add", "-N", ".")
        patch_path.write_text(
            _git(cwd, "diff", "--binary", baseline) + "\n",
            encoding="utf-8",
        )
        transcript.write_text(
            f"[stdout]\n{result.stdout}\n[stderr]\n{result.stderr}",
            encoding="utf-8",
        )
        message, tokens, cost = _usage(backend, result.stdout)
        return Invocation(
            exit_code=result.returncode,
            transcript_path=str(transcript),
            patch_path=str(patch_path),
            final_message=message,
            duration_seconds=result.duration_seconds,
            tokens=tokens,
            cost_usd=cost,
        )


def _change_summary(cwd: Path) -> str:
    try:
        changed = [
            line for line in _git(cwd, "status", "--short").splitlines() if line
        ]
    except RuntimeError:
        return "change summary unavailable"
    label = "path" if len(changed) == 1 else "paths"
    return f"{len(changed)} changed {label}"


def command_for(backend: Backend, prompt: str, model: str | None = None) -> list[str]:
    # Verified against Claude Code 2.1.220 and Codex CLI 0.145.0 on 2026-07-30.
    # subprocess cwd, rather than a CLI flag, selects the isolated worktree.
    # An unpinned model follows each CLI's ambient default, which makes a run
    # unreproducible and can strand it on an exhausted model's rate limit.
    if backend == "claude":
        command = [
            "claude",
            "--print",
            "--output-format",
            "json",
            "--permission-mode",
            "acceptEdits",
            "--no-session-persistence",
        ]
        if model is not None:
            command += ["--model", model]
        return [*command, prompt]
    command = [
        "codex",
        "--ask-for-approval",
        "never",
        "exec",
        "--json",
        "--ephemeral",
        "--sandbox",
        "workspace-write",
    ]
    if model is not None:
        command += ["--model", model]
    return [*command, prompt]


def _usage(backend: Backend, stdout: str) -> tuple[str, int | None, float | None]:
    if backend == "claude":
        try:
            value = cast(object, json.loads(stdout))
        except json.JSONDecodeError:
            return stdout.strip(), None, None
        # --output-format json emits an array of records; the result is last.
        if isinstance(value, list):
            records = [
                item for item in cast(list[object], value) if isinstance(item, dict)
            ]
            value = records[-1] if records else None
        if not isinstance(value, dict):
            return stdout.strip(), None, None
        data = cast(dict[str, object], value)
        message = data.get("result")
        cost = data.get("total_cost_usd")
        usage = data.get("usage")
        return (
            message if isinstance(message, str) else stdout.strip(),
            _token_total(usage, ()),
            float(cost) if isinstance(cost, (int, float)) else None,
        )
    message = ""
    codex_tokens: int | None = None
    for line in stdout.splitlines():
        try:
            value = cast(object, json.loads(line))
        except json.JSONDecodeError:
            continue
        if not isinstance(value, dict):
            continue
        data = cast(dict[str, object], value)
        item = data.get("item")
        if isinstance(item, dict):
            item_data = cast(dict[object, object], item)
            if item_data.get("type") == "agent_message" and isinstance(
                item_data.get("text"), str
            ):
                message = cast(str, item_data["text"])
        usage = data.get("usage")
        parsed_tokens = _token_total(usage, CODEX_SUBSET_KEYS)
        if parsed_tokens is not None:
            codex_tokens = parsed_tokens
    return message or stdout.strip(), codex_tokens, None


# Codex reports these as portions of input_tokens and output_tokens, so adding
# them counts the same tokens twice. Anthropic's buckets are disjoint.
CODEX_SUBSET_KEYS = ("cached_input_tokens", "reasoning_output_tokens")


def _token_total(value: object, subset_keys: tuple[str, ...]) -> int | None:
    if not isinstance(value, dict):
        return None
    total = 0
    found = False
    for key, item in cast(dict[object, object], value).items():
        if (
            isinstance(key, str)
            and key.endswith("_tokens")
            and key not in subset_keys
            and isinstance(item, int)
            and not isinstance(item, bool)
        ):
            total += item
            found = True
    return total if found else None


def _git(cwd: Path, *args: str) -> str:
    import subprocess

    result = subprocess.run(
        ["git", *args],
        cwd=cwd,
        check=False,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout
