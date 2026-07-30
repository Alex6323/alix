from __future__ import annotations

import unittest

from orchestrator.agents import command_for


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
