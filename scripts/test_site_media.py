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

    def test_an_img_string_inside_a_textarea_is_not_a_served_page_reference(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f'<textarea><img src="img/{SHOT}" alt=""></textarea>\n',
                encoding="utf-8",
            )

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)

    def test_a_picture_source_srcset_is_a_served_page_reference(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f'<picture><source srcset="img/{SHOT}" type="image/webp">'
                '<img src="img/fallback.png" alt=""></picture>\n',
                encoding="utf-8",
            )

        result = self.run_check(change)

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_a_shot_prefixed_subdirectory_is_not_a_served_media_path(self):
        nested = f"shot-old/{SHOT}"

        def change(root):
            (root / "site" / "img" / "shot-old").mkdir()
            (root / "site" / "img" / SHOT).rename(root / "site" / "img" / nested)
            (root / "site" / "index.html").write_text(
                f'<img src="img/{nested}" alt="">\n', encoding="utf-8"
            )
            (root / "README.md").write_text(
                f"![shot](https://alix.study/img/{nested})\n", encoding="utf-8"
            )
            (root / "e2e" / "shots" / "capture.cjs").write_text(
                f'const SHOTS = [\n  [1, "{nested}", shot1],\n];\n',
                encoding="utf-8",
            )

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)

    def test_src_and_srcset_on_one_img_do_not_repeat_the_shot(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f'<img src="img/{SHOT}" srcset="img/{SHOT} 1x" alt="">\n',
                encoding="utf-8",
            )

        result = self.run_check(change)

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_a_script_src_is_not_a_served_image_reference(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f'<script src="img/{SHOT}"></script>\n', encoding="utf-8"
            )

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)

    def test_a_video_source_src_is_not_a_served_image_reference(self):
        def change(root):
            (root / "site" / "index.html").write_text(
                f'<video><source src="img/{SHOT}" type="video/webm"></video>\n',
                encoding="utf-8",
            )

        result = self.run_check(change)

        self.assertEqual(1, result.returncode, result.stdout)


if __name__ == "__main__":
    unittest.main()
