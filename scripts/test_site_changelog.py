import importlib.util
import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).resolve().parent / "site-changelog.py"
SPEC = importlib.util.spec_from_file_location("site_changelog", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
SITE_CHANGELOG = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SITE_CHANGELOG)


class SiteChangelogTest(unittest.TestCase):
    def test_up_next_popover_renders_the_same_inline_markdown_as_the_list(self):
        up_next = ["**Sync from anywhere.** Over your own mesh."]
        layout = {
            "nodes": [],
            "ticks": [],
            "up_xs": [70],
            "width": 140,
        }

        timeline = SITE_CHANGELOG.render_timeline_html(layout, up_next)
        visible_list = SITE_CHANGELOG.render_up_next_html(up_next)

        expected = "<strong>Sync from anywhere.</strong> Over your own mesh."
        self.assertIn(expected, visible_list)
        self.assertIn(expected, timeline)
        self.assertNotIn("**Sync from anywhere.**", timeline)


if __name__ == "__main__":
    unittest.main()
