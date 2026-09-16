import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent
CONFIG = ROOT / ".github" / "dependabot.yml"
RELEASING = ROOT / "RELEASING.md"
PINNING_ADR = ROOT / "docs" / "adrs" / "0016-pinned-release-toolchains.md"


def update_blocks(text):
    starts = [match.start() for match in re.finditer(r"(?m)^  - package-ecosystem:", text)]
    blocks = []
    for index, start in enumerate(starts):
        end = starts[index + 1] if index + 1 < len(starts) else len(text)
        block = text[start:end]
        ecosystem = re.search(r'package-ecosystem: "([^"]+)"', block).group(1)
        directory = re.search(r'directory: "([^"]+)"', block).group(1)
        blocks.append(((ecosystem, directory), block))
    return dict(blocks)


class DependabotPolicyTests(unittest.TestCase):
    def setUp(self):
        self.text = CONFIG.read_text(encoding="utf-8")
        self.blocks = update_blocks(self.text)

    def test_every_dependency_root_is_monitored(self):
        self.assertEqual(
            {
                ("cargo", "/"),
                ("cargo", "/fuzz"),
                ("cargo", "/mobile/alix/rust"),
                ("cargo", "/test-support"),
                ("cargo", "/tools/gfm-harness"),
                ("github-actions", "/"),
                ("npm", "/e2e"),
                ("pub", "/mobile/alix"),
                ("pub", "/mobile/alix/rust_builder/cargokit/build_tool"),
                ("uv", "/orchestrator"),
            },
            set(self.blocks),
        )

    def test_updates_are_monthly_with_one_open_pull_request_per_root(self):
        self.assertNotIn('interval: "weekly"', self.text)
        self.assertIn(
            "multi-ecosystem-groups:\n"
            "  flutter-rust-bridge:\n"
            "    schedule:\n"
            '      interval: "monthly"',
            self.text,
        )

        for key, block in self.blocks.items():
            with self.subTest(root=key):
                self.assertIn("open-pull-requests-limit: 1", block)
                if "multi-ecosystem-group:" not in block:
                    self.assertIn('interval: "monthly"', block)

    def test_mobile_rust_and_pub_updates_are_one_lockstep_group(self):
        for key in (("cargo", "/mobile/alix/rust"), ("pub", "/mobile/alix")):
            with self.subTest(root=key):
                block = self.blocks[key]
                self.assertIn('multi-ecosystem-group: "flutter-rust-bridge"', block)
                self.assertRegex(block, r'patterns:\n\s+- "\*"')

    def test_action_updates_remain_reviewed_and_never_auto_merge(self):
        action_block = self.blocks[("github-actions", "/")]
        self.assertRegex(action_block, r'groups:\n\s+github-actions:\n\s+patterns:\n\s+- "\*"')
        self.assertNotRegex(self.text.lower(), r"auto.?merge")

        for workflow in (ROOT / ".github" / "workflows").glob("*.yml"):
            with self.subTest(workflow=workflow.name):
                self.assertNotIn("dependabot", workflow.read_text(encoding="utf-8").lower())

    def test_live_documentation_names_the_monthly_schedule(self):
        for path in (RELEASING, PINNING_ADR):
            with self.subTest(path=path.relative_to(ROOT)):
                text = path.read_text(encoding="utf-8")
                lower = text.lower()
                contexts = [
                    lower[max(0, match.start() - 80) : match.end() + 80]
                    for match in re.finditer("dependabot", lower)
                ]
                self.assertTrue(any("monthly" in context for context in contexts))
                for context in contexts:
                    self.assertNotIn("weekly", context)


if __name__ == "__main__":
    unittest.main()
