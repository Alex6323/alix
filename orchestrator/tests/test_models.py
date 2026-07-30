from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from orchestrator.models import AgentState, Finding, PhaseHistory, RunState
from orchestrator.storage import load_state, save_state


class StatePersistenceTests(unittest.TestCase):
    def test_state_round_trips_every_resumption_field(self) -> None:
        state = RunState(
            run_id="20260730T120000Z-symmetric",
            mode="symmetric",
            repo="/repo",
            run_dir="/runs/example",
            base="main",
            base_sha="abc123",
            phase="REVIEW_ROUND_1",
            agents={
                "claude": AgentState("/wt/claude", "agent/claude/run", "c1"),
                "codex": AgentState("/wt/codex", "agent/codex/run", "d1"),
            },
            rounds_completed=1,
            max_fix_rounds=2,
            implementer="claude",
            spec_hash="sha256-spec",
            prompt_hashes={"implement": "sha256-prompt"},
            findings=[
                Finding(
                    id="F1",
                    author="codex",
                    against="claude",
                    kind="defect",
                    test_patch="findings/F1.patch",
                    verified=True,
                    resolved=False,
                    summary="repro",
                )
            ],
            history=[
                PhaseHistory(
                    phase="IMPLEMENT",
                    started="2026-07-30T12:00:00Z",
                    ended="2026-07-30T12:03:00Z",
                    ok=True,
                )
            ],
        )
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "state.json"

            save_state(path, state)

            self.assertEqual(state, load_state(path))
            self.assertFalse(path.with_suffix(".json.tmp").exists())
            payload = json.loads(path.read_text())
            self.assertEqual(1, payload["schema_version"])

    def test_load_rejects_an_unknown_schema_version(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "state.json"
            path.write_text('{"schema_version": 99}\n')

            with self.assertRaisesRegex(ValueError, "schema version"):
                load_state(path)
