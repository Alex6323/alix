from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from orchestrator.review import ReviewManifestError, load_review_candidates


class ReviewManifestTests(unittest.TestCase):
    def test_a_candidate_requires_repro_and_real_user_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            checkout = Path(tmp)
            output = checkout / ".orchestrator-review"
            output.mkdir()
            (output / "F1.patch").write_text("diff --git a/tests/x.rs b/tests/x.rs\n")
            (output / "findings.json").write_text(
                json.dumps(
                    [
                        {
                            "summary": "lost update",
                            "test_name": "two_tabs_keep_both_updates",
                            "test_patch": "F1.patch",
                            "real_user_path": (
                                "Open the same workspace in two supported browser tabs "
                                "and start one session from each."
                            ),
                            "impact": (
                                "The first successful session disappears from recent "
                                "history and disk."
                            ),
                        }
                    ]
                )
            )

            candidates = load_review_candidates(checkout)

            self.assertEqual(1, len(candidates))
            self.assertEqual("two_tabs_keep_both_updates", candidates[0].test_name)
            self.assertIn("two supported browser tabs", candidates[0].real_user_path)
            self.assertEqual((output / "F1.patch").resolve(), candidates[0].patch)

    def test_missing_real_user_path_rejects_the_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            checkout = Path(tmp)
            output = checkout / ".orchestrator-review"
            output.mkdir()
            (output / "F1.patch").write_text("patch")
            (output / "findings.json").write_text(
                json.dumps(
                    [
                        {
                            "summary": "theoretical concern",
                            "test_name": "case",
                            "test_patch": "F1.patch",
                            "impact": "something might happen",
                        }
                    ]
                )
            )

            with self.assertRaisesRegex(ReviewManifestError, "real_user_path"):
                load_review_candidates(checkout)

    def test_patch_paths_cannot_escape_the_review_output(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            checkout = Path(tmp)
            output = checkout / ".orchestrator-review"
            output.mkdir()
            (output / "findings.json").write_text(
                json.dumps(
                    [
                        {
                            "summary": "escape",
                            "test_name": "case",
                            "test_patch": "../outside.patch",
                            "real_user_path": "Run the supported command.",
                            "impact": "The command loses data.",
                        }
                    ]
                )
            )

            with self.assertRaisesRegex(ReviewManifestError, "inside"):
                load_review_candidates(checkout)
