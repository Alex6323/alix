from __future__ import annotations

import json
from pathlib import Path
from typing import Literal, Protocol, cast

from orchestrator.commands import Executor, SubprocessExecutor
from orchestrator.models import Invocation

AgentName = Literal["claude", "codex"]


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
    ) -> None:
        self.run_dir = run_dir
        self.executor = executor or SubprocessExecutor()
        self.models = dict(models or {})

    def invoke(
        self, agent: AgentName, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        sequence = len(list((self.run_dir / "transcripts").glob("*.txt"))) + 1
        stem = f"{sequence:03d}-{agent}"
        transcript = self.run_dir / "transcripts" / f"{stem}.txt"
        patch_path = self.run_dir / "patches" / f"{stem}.patch"
        baseline = _git(cwd, "rev-parse", "HEAD").strip()
        result = self.executor.run(
            command_for(agent, prompt, self.models.get(agent)), cwd, timeout
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
        message, tokens, cost = _usage(agent, result.stdout)
        return Invocation(
            exit_code=result.returncode,
            transcript_path=str(transcript),
            patch_path=str(patch_path),
            final_message=message,
            duration_seconds=result.duration_seconds,
            tokens=tokens,
            cost_usd=cost,
        )


def command_for(agent: AgentName, prompt: str, model: str | None = None) -> list[str]:
    # Verified against Claude Code 2.1.220 and Codex CLI 0.145.0 on 2026-07-30.
    # subprocess cwd, rather than a CLI flag, selects the isolated worktree.
    # An unpinned model follows each CLI's ambient default, which makes a run
    # unreproducible and can strand it on an exhausted model's rate limit.
    if agent == "claude":
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


def _usage(agent: AgentName, stdout: str) -> tuple[str, int | None, float | None]:
    if agent == "claude":
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
