import os
import pathlib
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent
CHECK = ROOT / "scripts" / "adr-check.sh"


class AdrCheckTests(unittest.TestCase):
    def run_check(self, records, files=None):
        files = files or {}
        with tempfile.TemporaryDirectory() as raw:
            directory = pathlib.Path(raw)
            adrs = directory / "docs" / "adrs"
            adrs.mkdir(parents=True)
            for name, text in records.items():
                (adrs / name).write_text(text, encoding="utf-8")
            for name, text in files.items():
                path = directory / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8")
            env = os.environ.copy()
            env["TMPDIR"] = raw
            return subprocess.run(
                ["sh", str(CHECK)],
                cwd=directory,
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )

    def test_an_accepted_record_without_evidence_fails_closed(self):
        result = self.run_check(
            {
                "0001-missing.md": "# Missing evidence\n\n- Status: Accepted\n",
            }
        )

        self.assertNotEqual(0, result.returncode)
        self.assertIn("0001-missing.md", result.stderr)
        self.assertIn("name no evidence", result.stderr)

    def test_named_and_explicitly_absent_evidence_are_accepted(self):
        result = self.run_check(
            {
                "0001-named.md": (
                    "# Named evidence\n\n"
                    "- Status: Accepted\n"
                    "- Evidence: load_bearing_marker in src/model.rs\n"
                ),
                "0002-policy.md": (
                    "# Policy\n\n"
                    "- Status: Accepted\n"
                    "- Evidence: none, this constrains an intentional absence\n"
                ),
            },
            {"src/model.rs": "fn load_bearing_marker() {}\n"},
        )

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        self.assertIn("every named marker is present", result.stdout)


if __name__ == "__main__":
    unittest.main()
