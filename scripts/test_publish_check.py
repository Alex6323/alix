import os
import pathlib
import subprocess
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parent / "publish-check.sh"

MANIFEST = '[package]\nname = "alix"\nversion = "0.8.0"\n'

GIT_ENV = {
    **os.environ,
    "GIT_CONFIG_GLOBAL": "/dev/null",
    "GIT_CONFIG_SYSTEM": "/dev/null",
    "GIT_AUTHOR_NAME": "fixture",
    "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
    "GIT_COMMITTER_NAME": "fixture",
    "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
}


def git(repo, *args):
    subprocess.run(
        ["git", *args], cwd=repo, env=GIT_ENV, check=True, capture_output=True, text=True
    )


def tagged_repo(tags=("v0.8.0",)):
    repo = tempfile.mkdtemp(dir=os.environ.get("TMPDIR"))
    git(repo, "init", "-q")
    (pathlib.Path(repo) / "Cargo.toml").write_text(MANIFEST, encoding="utf-8")
    git(repo, "add", "Cargo.toml")
    git(repo, "commit", "-q", "-m", "release")
    for tag in tags:
        git(repo, "tag", tag)
    return repo


def run_check(repo):
    return subprocess.run(
        ["sh", str(SCRIPT)], cwd=repo, env=GIT_ENV, capture_output=True, text=True, check=False
    )


class PublishCheckTest(unittest.TestCase):
    def test_a_clean_checkout_of_the_manifest_tag_passes(self):
        result = run_check(tagged_repo())
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("OK (HEAD is v0.8.0", result.stdout)

    def test_a_second_tag_on_the_same_commit_does_not_hide_the_release_tag(self):
        result = run_check(tagged_repo(tags=("mobile-v0.3.0", "v0.8.0")))
        self.assertEqual(0, result.returncode, result.stderr)

    def test_a_commit_past_the_tag_fails_naming_the_tag_to_check_out(self):
        repo = tagged_repo()
        (pathlib.Path(repo) / "later.txt").write_text("codex work\n", encoding="utf-8")
        git(repo, "add", "later.txt")
        git(repo, "commit", "-q", "-m", "later")
        result = run_check(repo)
        self.assertEqual(1, result.returncode)
        self.assertIn("HEAD carries no tag v0.8.0", result.stderr)
        self.assertIn("git checkout v0.8.0", result.stderr)

    def test_a_tag_for_another_version_fails_listing_what_head_carries(self):
        result = run_check(tagged_repo(tags=("v0.7.0",)))
        self.assertEqual(1, result.returncode)
        self.assertIn("no tag v0.8.0", result.stderr)
        self.assertIn("tags on HEAD: v0.7.0", result.stderr)

    def test_a_dirty_tree_at_the_tag_fails_listing_the_changes(self):
        repo = tagged_repo()
        (pathlib.Path(repo) / "Cargo.toml").write_text(MANIFEST + "\n", encoding="utf-8")
        result = run_check(repo)
        self.assertEqual(1, result.returncode)
        self.assertIn("the tree is dirty", result.stderr)
        self.assertIn("Cargo.toml", result.stderr)

    def test_an_untracked_file_counts_as_dirty(self):
        repo = tagged_repo()
        (pathlib.Path(repo) / "scratch.log").write_text("", encoding="utf-8")
        result = run_check(repo)
        self.assertEqual(1, result.returncode)
        self.assertIn("scratch.log", result.stderr)


if __name__ == "__main__":
    unittest.main()
