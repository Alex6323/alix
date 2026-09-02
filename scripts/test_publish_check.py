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
    return subprocess.run(
        ["git", *args], cwd=repo, env=GIT_ENV, check=True, capture_output=True, text=True
    ).stdout.strip()


def commit(repo, name, content):
    (pathlib.Path(repo) / name).write_text(content, encoding="utf-8")
    git(repo, "add", name)
    git(repo, "commit", "-q", "-m", name)


def released_repo(tag="v0.8.0", pushed=True):
    workdir = tempfile.mkdtemp(dir=os.environ.get("TMPDIR"))
    origin = os.path.join(workdir, "origin.git")
    repo = os.path.join(workdir, "clone")
    subprocess.run(["git", "init", "-q", "--bare", origin], env=GIT_ENV, check=True)
    os.mkdir(repo)
    git(repo, "init", "-q", "-b", "main")
    git(repo, "remote", "add", "origin", origin)
    commit(repo, "Cargo.toml", MANIFEST)
    if tag:
        git(repo, "tag", tag)
    git(repo, "push", "-q", "origin", "main")
    if tag and pushed:
        git(repo, "push", "-q", "origin", tag)
    return repo


def run_check(repo):
    return subprocess.run(
        ["sh", str(SCRIPT)], cwd=repo, env=GIT_ENV, capture_output=True, text=True, check=False
    )


class PublishCheckTest(unittest.TestCase):
    def test_a_clean_tree_with_the_pushed_manifest_tag_in_its_history_passes(self):
        result = run_check(released_repo())
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("OK (v0.8.0 = ", result.stdout)

    def test_a_commit_past_the_tag_still_passes_because_the_recipe_checks_the_tag_out(self):
        repo = released_repo()
        commit(repo, "later.txt", "codex work\n")
        result = run_check(repo)
        self.assertEqual(0, result.returncode, result.stderr)

    def test_a_missing_tag_fails_naming_the_manifest_version(self):
        result = run_check(released_repo(tag=None))
        self.assertEqual(1, result.returncode)
        self.assertIn("no tag v0.8.0", result.stderr)
        self.assertIn("0.8.0", result.stderr)

    def test_a_tag_for_another_version_does_not_count(self):
        result = run_check(released_repo(tag="v0.7.0"))
        self.assertEqual(1, result.returncode)
        self.assertIn("no tag v0.8.0", result.stderr)

    def test_a_tag_that_only_matches_the_version_as_a_pattern_does_not_count(self):
        result = run_check(released_repo(tag="v0x8x0"))
        self.assertEqual(1, result.returncode)
        self.assertIn("no tag v0.8.0", result.stderr)

    def test_a_tag_absent_from_origin_fails(self):
        result = run_check(released_repo(pushed=False))
        self.assertEqual(1, result.returncode)
        self.assertIn("origin has no tag v0.8.0", result.stderr)

    def test_an_unreachable_origin_fails_as_a_listing_error_not_a_missing_tag(self):
        repo = released_repo()
        git(repo, "remote", "set-url", "origin", os.path.join(repo, "no-such-origin.git"))
        result = run_check(repo)
        self.assertEqual(1, result.returncode)
        self.assertIn("could not list origin's tags", result.stderr)
        self.assertNotIn("origin has no tag", result.stderr)

    def test_a_tag_moved_locally_after_the_push_fails_naming_both_commits(self):
        repo = released_repo()
        commit(repo, "later.txt", "retagged\n")
        git(repo, "tag", "-f", "v0.8.0")
        result = run_check(repo)
        self.assertEqual(1, result.returncode)
        self.assertIn("on origin", result.stderr)
        self.assertIn(git(repo, "rev-parse", "HEAD"), result.stderr)

    def test_a_tag_outside_head_history_fails(self):
        repo = released_repo()
        git(repo, "checkout", "-q", "--orphan", "elsewhere")
        commit(repo, "elsewhere.txt", "a root commit that never saw the tag\n")
        self.assertNotEqual(git(repo, "rev-parse", "HEAD"), git(repo, "rev-parse", "v0.8.0"))
        result = run_check(repo)
        self.assertEqual(1, result.returncode)
        self.assertIn("not in HEAD's history", result.stderr)

    def test_a_modified_file_fails_listing_the_change(self):
        repo = released_repo()
        (pathlib.Path(repo) / "Cargo.toml").write_text(MANIFEST + "\n", encoding="utf-8")
        result = run_check(repo)
        self.assertEqual(1, result.returncode)
        self.assertIn("the tree is dirty", result.stderr)
        self.assertIn("Cargo.toml", result.stderr)

    def test_an_untracked_file_counts_as_dirty(self):
        repo = released_repo()
        (pathlib.Path(repo) / "scratch.log").write_text("", encoding="utf-8")
        result = run_check(repo)
        self.assertEqual(1, result.returncode)
        self.assertIn("scratch.log", result.stderr)


if __name__ == "__main__":
    unittest.main()
