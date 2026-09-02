from pathlib import Path
import unittest


class CalibrationDocumentation(unittest.TestCase):
    def test_module_overview_names_the_effort_floor_row(self):
        source = Path("tests/calibrate.rs").read_text()
        overview = source.split("use alix", 1)[0].lower()
        self.assertIn("effort", overview)

    def test_release_gate_names_effort_as_a_possible_floor(self):
        source = Path("RELEASING.md").read_text()
        section = source.split("**Live grader calibration.**", 1)[1].split("4b.", 1)[0].lower()
        self.assertIn("effort", section)


if __name__ == "__main__":
    unittest.main()
