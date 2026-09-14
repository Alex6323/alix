import contextlib
import io
import pathlib
import tempfile
import unittest

from mutants_allowed import check_pins


ROOT = pathlib.Path(__file__).resolve().parent.parent


class MutantsAllowlistTests(unittest.TestCase):
    def test_unreadable_allowlist_fails_closed(self):
        missing = pathlib.Path("/path/that/does/not/exist/mutants-allowlist.txt")

        error = io.StringIO()
        with contextlib.redirect_stderr(error):
            self.assertEqual(1, check_pins([], missing))
        self.assertIn("allowlist: cannot read", error.getvalue())

    def test_only_stale_line_pins_fail_the_source_list_check(self):
        with tempfile.TemporaryDirectory() as tempdir:
            allowlist = pathlib.Path(tempdir) / "allowlist.txt"
            allowlist.write_text(
                "\n".join(
                    [
                        "src/current.rs:10: replace + with - in current",
                        "src/broad.rs: replace == with != in broad",
                        "src/stale.rs:20: replace - with + in stale",
                    ]
                ),
                encoding="utf-8",
            )
            listed = [
                "src/current.rs:10:7: replace + with - in current",
                "src/stale.rs:21:9: replace - with + in stale",
            ]

            error = io.StringIO()
            with contextlib.redirect_stderr(error):
                self.assertEqual(1, check_pins(listed, allowlist))
            self.assertEqual(
                "allowlist: src/stale.rs:20: replace - with + in stale "
                "names no current mutant\n",
                error.getvalue(),
            )

            listed.append("src/stale.rs:20:9: replace - with + in stale")
            error = io.StringIO()
            with contextlib.redirect_stderr(error):
                self.assertEqual(0, check_pins(listed, allowlist))
            self.assertEqual("", error.getvalue())

    def test_make_mutants_checks_pins_once_before_selecting_a_mode(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        command = "python3 scripts/mutants_allowed.py --check-pins"

        self.assertEqual(1, makefile.count(command))
        self.assertLess(
            makefile.index(command),
            makefile.index('if [ -n "$(MUTANTS_ALL)" ]'),
        )
        self.assertLess(
            makefile.index("cargo mutants --list --colors never"),
            makefile.index(command),
        )


if __name__ == "__main__":
    unittest.main()
