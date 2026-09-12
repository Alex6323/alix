import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent


def make_recipe(name):
    text = (ROOT / "Makefile").read_text(encoding="utf-8")
    body = text.split(f"\n{name}:\n", 1)[1]
    return body.split("\n\n", 1)[0]


def workflow_job(name):
    text = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
        encoding="utf-8"
    )
    body = text.split(f"\n  {name}:\n", 1)[1]
    return re.split(r"\n  [A-Za-z0-9_-]+:\n", body, maxsplit=1)[0]


class MobileE2EGateTests(unittest.TestCase):
    def test_local_gate_builds_the_debug_desktop_binary(self):
        self.assertIn(
            "\n\tcargo build\n",
            "\n" + make_recipe("mobile-test") + "\n",
            "sync_e2e_support.dart requires target/debug/alix in a clean checkout",
        )

    def test_ci_gate_builds_the_debug_desktop_binary(self):
        self.assertRegex(
            workflow_job("mobile-e2e"),
            r"(?m)^\s*- run: cargo build\s*$",
            "the blocking mobile job must build target/debug/alix before sync e2e",
        )

    def test_local_gate_launches_each_integration_file_separately(self):
        self.assertIn(
            "for f in integration_test/*_test.dart",
            make_recipe("mobile-test"),
            "Linux cannot launch the second app in one directory-wide Flutter run",
        )

    def test_ci_gate_launches_each_integration_file_separately(self):
        self.assertIn(
            "for f in integration_test/*_test.dart",
            workflow_job("mobile-e2e"),
            "the blocking Linux job must use one Flutter process per integration file",
        )


if __name__ == "__main__":
    unittest.main()
