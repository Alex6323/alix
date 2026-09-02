import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / ".github" / "workflows"

# Every release workflow promises its GitHub Release notes come from a
# changelog section (RELEASING.md); each row is the line that makes that true.
NOTES_SOURCES = [
    ("release.yml", "taiki-e/create-gh-release-action@", "changelog: CHANGELOG.md"),
    ("mobile-release.yml", "Extract the release notes", "mobile/alix/CHANGELOG.md"),
]


def step_block(text, anchor):
    lines = text.splitlines()
    start = next(i for i, line in enumerate(lines) if anchor in line)
    block = [lines[start]]
    for line in lines[start + 1:]:
        if line.startswith("      - ") or not line.strip():
            break
        block.append(line)
    return "\n".join(block)


class ReleaseNotesTest(unittest.TestCase):
    def test_every_release_workflow_names_the_changelog_its_notes_come_from(self):
        for workflow, anchor, needle in NOTES_SOURCES:
            with self.subTest(workflow=workflow):
                text = (WORKFLOWS / workflow).read_text(encoding="utf-8")
                self.assertIn(
                    needle,
                    step_block(text, anchor),
                    f"{workflow}: the step at '{anchor}' must name {needle}",
                )


if __name__ == "__main__":
    unittest.main()
