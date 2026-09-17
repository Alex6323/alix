import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent
CHECK = ROOT / "scripts" / "check-example-media.py"
SETS = ("shapes", "syntax")

WEBP = b"RIFF" + (26).to_bytes(4, "little") + b"WEBPVP8L" + bytes(18)


class ExampleMediaTests(unittest.TestCase):
    def run_check(self, change=None):
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw) / "repo"
            (root / "scripts").mkdir(parents=True)
            shutil.copy2(CHECK, root / "scripts" / CHECK.name)
            for name in SETS:
                directory = root / "docs" / "examples" / name
                directory.mkdir(parents=True)
                (directory / "one.md").write_text("# One\n", encoding="utf-8")
                (directory / "one.webp").write_bytes(WEBP)
            if change is not None:
                change(root)
            return subprocess.run(
                [sys.executable, str(root / "scripts" / CHECK.name)],
                cwd=root,
                check=False,
                capture_output=True,
                text=True,
            )

    def test_a_complete_set_of_examples_passes(self):
        result = self.run_check()

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_an_example_without_an_image_is_denied(self):
        def change(root):
            (root / "docs" / "examples" / "shapes" / "one.webp").unlink()

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)
        self.assertIn("has no image", result.stderr)

    def test_an_interrupted_capture_leaving_an_empty_file_is_denied(self):
        def change(root):
            (root / "docs" / "examples" / "shapes" / "one.webp").write_bytes(b"")

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)
        self.assertIn("one.webp", result.stderr)

    def test_a_file_that_is_not_a_webp_is_denied(self):
        def change(root):
            (root / "docs" / "examples" / "syntax" / "one.webp").write_bytes(
                b"\x89PNG\r\n\x1a\n" + bytes(16)
            )

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)
        self.assertIn("one.webp", result.stderr)


if __name__ == "__main__":
    unittest.main()
