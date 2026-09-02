import os
import pathlib
import subprocess
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parent / "changelog-check.sh"

HEAD = "# Changelog\n\n"
RELEASED = "## [0.1.0] - 2026-01-01\n\n### Added\n- The first entry.\n"
SKELETON = "## [Unreleased]\n\n### Added\n\n### Changed\n\n### Fixed\n\n"


def run_check(changelog):
    workdir = tempfile.mkdtemp(dir=os.environ.get("TMPDIR"))
    (pathlib.Path(workdir) / "CHANGELOG.md").write_text(changelog, encoding="utf-8")
    return subprocess.run(
        ["sh", str(SCRIPT)], cwd=workdir, capture_output=True, text=True, check=False
    )


class ChangelogCheckTest(unittest.TestCase):
    def test_a_fresh_unreleased_skeleton_passes(self):
        result = run_check(HEAD + SKELETON + RELEASED)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("skeleton intact", result.stdout)

    def test_a_bare_unreleased_heading_fails_naming_every_missing_subsection(self):
        result = run_check(HEAD + "## [Unreleased]\n\n" + RELEASED)
        self.assertEqual(1, result.returncode)
        self.assertIn("lacks its release skeleton", result.stderr)
        for name in ("### Added", "### Changed", "### Fixed"):
            self.assertIn(name, result.stderr)

    def test_a_partial_skeleton_names_only_what_is_missing(self):
        result = run_check(HEAD + "## [Unreleased]\n\n### Added\n\n### Fixed\n\n" + RELEASED)
        self.assertEqual(1, result.returncode)
        self.assertIn("### Changed", result.stderr)
        self.assertNotIn("### Added", result.stderr)

    def test_a_skeleton_entry_counts_even_when_filled(self):
        filled = "## [Unreleased]\n\n### Added\n- New.\n\n### Changed\n- Moved.\n\n### Fixed\n- Fixed.\n\n"
        result = run_check(HEAD + filled + RELEASED)
        self.assertEqual(0, result.returncode, result.stderr)


if __name__ == "__main__":
    unittest.main()
