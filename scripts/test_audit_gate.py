import os
import pathlib
import shlex
import stat
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent


class AuditGateTests(unittest.TestCase):
    def write_executable(self, directory, name, body):
        path = directory / name
        path.write_text(body, encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def test_both_lockfiles_deny_unsound_and_yanked_advisories(self):
        with tempfile.TemporaryDirectory() as raw:
            directory = pathlib.Path(raw)
            log = directory / "calls"
            self.write_executable(directory, "cargo-audit", "#!/bin/sh\nexit 0\n")
            self.write_executable(
                directory,
                "cargo",
                """#!/bin/sh
printf '%s\n' "$*" >> "$AUDIT_CALLS"
unsound=0
yanked=0
for arg in "$@"; do
    [ "$arg" = unsound ] && unsound=1
    [ "$arg" = yanked ] && yanked=1
done
[ "$unsound" -eq 1 ] && [ "$yanked" -eq 1 ] || exit 42
""",
            )
            env = os.environ.copy()
            env["AUDIT_CALLS"] = str(log)
            env["PATH"] = f"{directory}:/usr/bin:/bin"

            result = subprocess.run(
                ["make", "--no-print-directory", "audit"],
                cwd=ROOT,
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertEqual(0, result.returncode, result.stdout + result.stderr)
            calls = [shlex.split(line) for line in log.read_text(encoding="utf-8").splitlines()]
            self.assertEqual(
                [
                    ["audit", "--deny", "unsound", "--deny", "yanked"],
                    [
                        "audit",
                        "--file",
                        "mobile/alix/rust/Cargo.lock",
                        "--deny",
                        "unsound",
                        "--deny",
                        "yanked",
                    ],
                ],
                calls,
            )

    def test_a_missing_cargo_audit_executable_fails_closed(self):
        with tempfile.TemporaryDirectory() as raw:
            directory = pathlib.Path(raw)
            self.write_executable(directory, "cargo", "#!/bin/sh\nexit 0\n")
            env = os.environ.copy()
            env["PATH"] = f"{directory}:/usr/bin:/bin"

            result = subprocess.run(
                ["make", "--no-print-directory", "audit"],
                cwd=ROOT,
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertNotEqual(0, result.returncode)
            self.assertIn("cargo-audit not found", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
