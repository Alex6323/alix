import re
import subprocess
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class GfmGateContractTests(unittest.TestCase):
    def test_harness_dependency_resolution_is_committed(self):
        lock = ROOT / "tools" / "gfm-harness" / "Cargo.lock"
        self.assertTrue(
            lock.is_file(),
            "the baseline generator must not resolve a fresh dependency graph",
        )
        tracked = subprocess.run(
            ["git", "ls-files", "--error-unmatch", str(lock.relative_to(ROOT))],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(0, tracked.returncode, tracked.stderr)

    def test_harness_dependency_resolution_is_locked(self):
        makefile = (ROOT / "Makefile").read_text()
        self.assertTrue(
            "cargo run --locked --manifest-path tools/gfm-harness/Cargo.toml"
            in makefile,
            "make gfm-measure must refuse to update the reviewed lock",
        )

    def test_harness_uses_the_production_yaml_parser(self):
        def package_version(path, name):
            with path.open("rb") as source:
                packages = tomllib.load(source)["package"]
            return {package["version"] for package in packages if package["name"] == name}

        harness_lock = ROOT / "tools" / "gfm-harness" / "Cargo.lock"
        self.assertTrue(harness_lock.is_file(), "the harness lock is missing")
        self.assertEqual(
            package_version(ROOT / "Cargo.lock", "yaml-rust2"),
            package_version(harness_lock, "yaml-rust2"),
        )

    def test_harness_uses_the_production_alix_dependency_tree(self):
        def alix_tree(*extra):
            completed = subprocess.run(
                [
                    "cargo",
                    "tree",
                    "--locked",
                    "--edges",
                    "normal",
                    "--prefix",
                    "none",
                    "--format",
                    "{p}",
                    "-p",
                    "alix",
                    *extra,
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            self.assertEqual(0, completed.returncode, completed.stderr)
            return {
                package.removesuffix(" (*)")
                for package in completed.stdout.splitlines()
                if not package.startswith("alix v")
            }

        production = alix_tree("--no-default-features")
        measured = alix_tree(
            "--manifest-path",
            "tools/gfm-harness/Cargo.toml",
        )
        self.assertEqual(
            production,
            measured,
            "the corpus gate must measure the production lean dependency graph"
            " (remedy: scripts/sync_gfm_harness_lock.py)",
        )

    def test_scope_includes_inputs_that_can_change_the_measured_binary(self):
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
        match = re.search(r"grep -qE '([^']+)'", workflow)
        self.assertIsNotNone(match, "the GFM job's path classifier is missing")
        pattern = re.compile(match.group(1))
        for path in ("Cargo.toml", "Cargo.lock", "Makefile"):
            self.assertRegex(
                path,
                pattern,
                f"{path} can change the measured binary or command",
            )


if __name__ == "__main__":
    unittest.main()
