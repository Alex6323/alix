import os
import pathlib
import stat
import subprocess
import tempfile
import tomllib
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent


class DependencyGateTests(unittest.TestCase):
    def write_executable(self, directory, name, body):
        path = directory / name
        path.write_text(body, encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def run_gate(self, cargo_deny_version):
        with tempfile.TemporaryDirectory() as raw:
            directory = pathlib.Path(raw)
            calls = directory / "calls"
            self.write_executable(
                directory,
                "cargo-deny",
                f"""#!/bin/sh
printf 'cargo-deny %s\n' "$*" >>"$DEPENDENCY_GATE_CALLS"
if [ "$1" = --version ]; then
    echo 'cargo-deny {cargo_deny_version}'
fi
""",
            )
            self.write_executable(
                directory,
                "python3",
                """#!/bin/sh
printf 'python3 %s\n' "$*" >>"$DEPENDENCY_GATE_CALLS"
""",
            )
            env = os.environ.copy()
            env["DEPENDENCY_GATE_CALLS"] = str(calls)
            env["PATH"] = f"{directory}:/usr/bin:/bin"
            result = subprocess.run(
                ["make", "--no-print-directory", "deps-check"],
                cwd=ROOT,
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )
            recorded = (
                calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []
            )
            return result, recorded

    def test_the_pinned_cargo_deny_checks_rust_then_the_other_ecosystems(self):
        result, calls = self.run_gate("0.20.2")

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        self.assertEqual(
            [
                "cargo-deny --version",
                "cargo-deny --manifest-path Cargo.toml --locked --offline "
                "--config deny.toml check -D no-license-field -D unmatched-skip "
                "-D unnecessary-skip licenses bans",
                "python3 scripts/check-dependency-policy.py",
            ],
            calls,
        )

    def test_another_cargo_deny_version_is_refused(self):
        result, calls = self.run_gate("0.20.1")

        self.assertEqual(2, result.returncode, result.stdout + result.stderr)
        self.assertIn(
            "deps-check: cargo-deny 0.20.2 is required; found cargo-deny 0.20.1",
            result.stderr,
        )
        self.assertEqual(["cargo-deny --version"], calls)

    def test_the_configuration_records_the_ruled_license_and_duplicate_policy(self):
        with (ROOT / "deny.toml").open("rb") as source:
            config = tomllib.load(source)

        self.assertEqual(
            {
                "0BSD",
                "Apache-2.0",
                "Apache-2.0 WITH LLVM-exception",
                "BSD-2-Clause",
                "BSD-3-Clause",
                "MIT",
                "Unicode-3.0",
                "Unlicense",
                "Zlib",
            },
            set(config["licenses"]["allow"]),
        )
        self.assertEqual(
            [{"crate": "option-ext@0.2.0", "allow": ["MPL-2.0"]}],
            config["licenses"]["exceptions"],
        )
        self.assertEqual(1, len(config["licenses"]["clarify"]))
        clarification = config["licenses"]["clarify"][0]
        self.assertEqual("rs-fsrs@1.2.1", clarification["crate"])
        self.assertEqual("MIT", clarification["expression"])
        self.assertEqual("LICENSE", clarification["license-files"][0]["path"])
        self.assertIsInstance(clarification["license-files"][0]["hash"], int)
        self.assertEqual("deny", config["bans"]["multiple-versions"])
        self.assertFalse((ROOT / "scripts" / "deps-duplicates.txt").exists())


if __name__ == "__main__":
    unittest.main()
