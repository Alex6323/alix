from __future__ import annotations

import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

from orchestrator.commands import CommandResult, SubprocessExecutor


class SubprocessExecutorTests(unittest.TestCase):
    def test_cancel_all_stops_a_running_process_group(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ready = root / "ready"
            executor = SubprocessExecutor()
            results: list[CommandResult] = []

            def run() -> None:
                results.append(
                    executor.run(
                        [
                            sys.executable,
                            "-c",
                            (
                                "from pathlib import Path; import time; "
                                f"Path({str(ready)!r}).write_text('ready'); "
                                "time.sleep(5)"
                            ),
                        ],
                        root,
                        timeout=10,
                    )
                )

            worker = threading.Thread(target=run)
            worker.start()
            deadline = time.monotonic() + 1
            while not ready.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(ready.exists())

            executor.cancel_all()
            worker.join(timeout=2)

            self.assertFalse(worker.is_alive())
            self.assertEqual(1, len(results))
            self.assertNotEqual(0, results[0].returncode)
