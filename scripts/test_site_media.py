import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent
CHECK = ROOT / "scripts" / "check-site-media.py"
SHOT = "shot-1-verify.webp"


class SiteMediaTests(unittest.TestCase):
    def run_check(self, change=None):
        with tempfile.TemporaryDirectory() as raw:
            root = pathlib.Path(raw) / "repo"
            (root / "scripts").mkdir(parents=True)
            shutil.copy2(CHECK, root / "scripts" / CHECK.name)
            (root / "site" / "img").mkdir(parents=True)
            (root / "site" / "img" / SHOT).write_bytes(b"webp")
            (root / "site" / "index.html").write_text(
                f'<img src="img/{SHOT}" alt="">\n', encoding="utf-8"
            )
            (root / "README.md").write_text(
                f"![shot](https://alix.study/img/{SHOT})\n", encoding="utf-8"
            )
            (root / "e2e" / "shots").mkdir(parents=True)
            (root / "e2e" / "shots" / "capture.cjs").write_text(
                f'const SHOTS = [\n  [1, "{SHOT}", shot1],\n];\n', encoding="utf-8"
            )
            if change is not None:
                change(root)
            return subprocess.run(
                [sys.executable, str(root / "scripts" / CHECK.name)],
                cwd=root,
                check=False,
                capture_output=True,
                text=True,
            )

    def test_a_complete_media_set_passes(self):
        result = self.run_check()

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_a_shot_moved_out_of_the_served_path_is_denied(self):
        def change(root):
            (root / "site" / "img" / "old").mkdir()
            (root / "site" / "img" / SHOT).rename(root / "site" / "img" / "old" / SHOT)

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)
        self.assertIn(f"missing carousel screenshots: {SHOT}", result.stderr)

    def test_a_page_reference_into_a_subdirectory_is_denied(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f'<img src="img/old/{SHOT}" alt="">\n', encoding="utf-8"
            )

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)

    def test_a_readme_reference_into_a_subdirectory_is_denied(self):
        def change(root):
            (root / "README.md").write_text(
                f"![shot](https://alix.study/img/old/{SHOT})\n", encoding="utf-8"
            )

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)
        self.assertIn(f"old/{SHOT}", result.stderr)

    def test_a_percent_encoded_url_cannot_stand_for_a_literal_media_path(self):
        encoded = "shot-%31-verify.webp"

        def change(root):
            (root / "site" / "img" / SHOT).rename(
                root / "site" / "img" / encoded
            )
            (root / "site" / "index.html").write_text(
                f'<img src="img/{encoded}" alt="">\n', encoding="utf-8"
            )
            (root / "README.md").write_text(
                f"![shot](https://alix.study/img/{encoded})\n", encoding="utf-8"
            )
            (root / "e2e" / "shots" / "capture.cjs").write_text(
                f'const SHOTS = [\n  [1, "{encoded}", shot1],\n];\n',
                encoding="utf-8",
            )

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)

    def test_a_single_quoted_page_reference_is_the_same_served_path(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f"<img src='img/{SHOT}' alt=''>\n", encoding="utf-8"
            )

        result = self.run_check(change)

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_a_data_src_attribute_is_not_a_served_page_reference(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f'<img data-src="img/{SHOT}" alt="">\n', encoding="utf-8"
            )

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)

    def test_whitespace_around_src_equals_is_a_served_page_reference(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f'<img src = "img/{SHOT}" alt="">\n', encoding="utf-8"
            )

        result = self.run_check(change)

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_a_commented_src_attribute_is_not_a_served_page_reference(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f'<!-- <img src="img/{SHOT}" alt=""> -->\n', encoding="utf-8"
            )

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)

    def test_an_uppercase_src_attribute_is_a_served_page_reference(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f'<img SRC="img/{SHOT}" alt="">\n', encoding="utf-8"
            )

        result = self.run_check(change)

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
