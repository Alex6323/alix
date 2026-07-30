from __future__ import annotations

import json
import subprocess
import tempfile
import threading
import time
import unittest
from pathlib import Path

from orchestrator.agents import AgentName, SubprocessInvoker, _usage, command_for
from orchestrator.commands import CommandResult

AGENTS: tuple[AgentName, ...] = ("claude", "codex")


class AgentCommandTests(unittest.TestCase):
    def test_claude_invocation_is_noninteractive_json_in_the_supplied_cwd(self) -> None:
        command = command_for("claude", "implement this")

        self.assertEqual(
            [
                "claude",
                "--print",
                "--output-format",
                "json",
                "--permission-mode",
                "acceptEdits",
                "--no-session-persistence",
                "implement this",
            ],
            command,
        )
    def test_codex_invocation_is_ephemeral_noninteractive_json(self) -> None:
        command = command_for("codex", "implement this")

        self.assertEqual(
            [
                "codex",
                "--ask-for-approval",
                "never",
                "exec",
                "--json",
                "--ephemeral",
                "--sandbox",
                "workspace-write",
                "implement this",
            ],
            command,
        )

    def test_a_pinned_model_reaches_both_clis_and_the_prompt_stays_last(self) -> None:
        for agent in AGENTS:
            command = command_for(agent, "implement this", "a-model")

            self.assertEqual(agent, command[0])
            self.assertEqual("implement this", command[-1])
            index = command.index("--model")
            self.assertEqual("a-model", command[index + 1])

    def test_an_unpinned_model_passes_no_model_flag(self) -> None:
        for agent in AGENTS:
            self.assertNotIn("--model", command_for(agent, "implement this"))


class UsageParsingTests(unittest.TestCase):
    # Claude Code 2.1.220 --output-format json emits an array of records whose
    # last entry is the result; Codex emits one JSON object per line.
    CLAUDE_STDOUT = json.dumps(
        [
            {"type": "system", "subtype": "init"},
            {
                "type": "result",
                "subtype": "success",
                "result": "implemented it",
                "total_cost_usd": 1.25,
                "usage": {
                    "input_tokens": 21,
                    "cache_creation_input_tokens": 100,
                    "cache_read_input_tokens": 200,
                    "output_tokens": 50,
                },
            },
        ]
    )
    CODEX_STDOUT = "\n".join(
        [
            json.dumps({"item": {"type": "agent_message", "text": "implemented it"}}),
            json.dumps(
                {
                    "usage": {
                        "input_tokens": 1000,
                        "cached_input_tokens": 900,
                        "cache_write_input_tokens": 0,
                        "output_tokens": 100,
                        "reasoning_output_tokens": 40,
                    }
                }
            ),
        ]
    )

    def test_claude_usage_survives_the_streamed_array_shape(self) -> None:
        message, tokens, cost = _usage("claude", self.CLAUDE_STDOUT)

        self.assertEqual("implemented it", message)
        self.assertEqual(1.25, cost)
        # Anthropic reports four disjoint buckets, so they sum.
        self.assertEqual(371, tokens)

    def test_codex_cached_and_reasoning_subsets_are_not_counted_twice(self) -> None:
        message, tokens, cost = _usage("codex", self.CODEX_STDOUT)

        self.assertEqual("implemented it", message)
        self.assertIsNone(cost)
        # cached_input_tokens is part of input_tokens, and
        # reasoning_output_tokens is part of output_tokens.
        self.assertEqual(1100, tokens)


class _EditingExecutor:
    """Stands in for the agent CLI: edits a file instead of calling a model."""

    def __init__(self) -> None:
        self.commands: list[list[str]] = []

    def run(
        self, args: list[str], cwd: Path, timeout: float | None = None
    ) -> CommandResult:
        self.commands.append(args)
        (cwd / "edited.txt").write_text("agent change\n", encoding="utf-8")
        return CommandResult(
            returncode=0, stdout="done", stderr="", duration_seconds=0.01
        )


class _BlockingExecutor(_EditingExecutor):
    def __init__(self) -> None:
        super().__init__()
        self.started = threading.Event()
        self.release = threading.Event()

    def run(
        self, args: list[str], cwd: Path, timeout: float | None = None
    ) -> CommandResult:
        self.started.set()
        if not self.release.wait(timeout=1):
            raise AssertionError("test did not release the fake invocation")
        return super().run(args, cwd, timeout)


def _seed_repo(root: Path) -> Path:
    repo = root / "repo"
    repo.mkdir()
    for args in (
        ("init", "-b", "main"),
        ("config", "user.email", "t@example.invalid"),
        ("config", "user.name", "t"),
        ("config", "commit.gpgsign", "false"),
    ):
        subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True)
    (repo / "seed.txt").write_text("seed\n", encoding="utf-8")
    subprocess.run(["git", "add", "."], cwd=repo, check=True, capture_output=True)
    subprocess.run(
        ["git", "commit", "-m", "seed"], cwd=repo, check=True, capture_output=True
    )
    return repo


class SubprocessInvokerTests(unittest.TestCase):
    def test_invoke_records_start_and_heartbeat_before_the_agent_returns(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = _seed_repo(root)
            run_dir = root / "run"
            (run_dir / "transcripts").mkdir(parents=True)
            (run_dir / "patches").mkdir(parents=True)
            executor = _BlockingExecutor()
            invoker = SubprocessInvoker(
                run_dir,
                executor=executor,
                heartbeat_seconds=0.01,
            )
            completed = threading.Event()

            def invoke() -> None:
                invoker.invoke("claude", "do the thing", repo, 5.0)
                completed.set()

            worker = threading.Thread(target=invoke)
            worker.start()
            self.assertTrue(executor.started.wait(timeout=1))
            progress = run_dir / "progress.log"
            deadline = time.monotonic() + 1
            while (
                "heartbeat" not in progress.read_text(encoding="utf-8")
                and time.monotonic() < deadline
            ):
                time.sleep(0.01)

            self.assertFalse(completed.is_set())
            self.assertIn("started", progress.read_text(encoding="utf-8"))
            self.assertIn("heartbeat", progress.read_text(encoding="utf-8"))
            executor.release.set()
            worker.join(timeout=1)
            self.assertTrue(completed.is_set())
            self.assertIn("finished", progress.read_text(encoding="utf-8"))

    def test_invoke_diffs_the_patch_against_the_pre_invocation_head(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = _seed_repo(root)
            run_dir = root / "run"
            (run_dir / "transcripts").mkdir(parents=True)
            (run_dir / "patches").mkdir(parents=True)

            invoker = SubprocessInvoker(run_dir, executor=_EditingExecutor())
            invocation = invoker.invoke("claude", "do the thing", repo, 5.0)

            patch = Path(invocation.patch_path).read_text(encoding="utf-8")
            self.assertIn("edited.txt", patch)
            self.assertEqual(0, invocation.exit_code)

    def test_invoke_pins_the_configured_model_for_that_agent(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = _seed_repo(root)
            run_dir = root / "run"
            (run_dir / "transcripts").mkdir(parents=True)
            (run_dir / "patches").mkdir(parents=True)

            executor = _EditingExecutor()
            invoker = SubprocessInvoker(
                run_dir, executor=executor, models={"claude": "opus"}
            )
            invoker.invoke("claude", "do the thing", repo, 5.0)
            invoker.invoke("codex", "do the thing", repo, 5.0)

            claude_command, codex_command = executor.commands
            self.assertIn("--model", claude_command)
            self.assertEqual(
                "opus", claude_command[claude_command.index("--model") + 1]
            )
            self.assertNotIn("--model", codex_command)
