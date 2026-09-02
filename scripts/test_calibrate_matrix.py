import unittest

import calibrate_matrix

LOG = """\
running 2 tests
test a_complete_correct_proof_passes ... calibrate: probe=math_proof_full backend=claude requested=cli-default observed=unreported verdict=Pass
calibrate: probe=math_proof_full backend=codex requested=effort-none observed=unreported verdict=Pass
ok
test an_empty_answer_does_not_pass ... calibrate: probe=empty_answer backend=claude requested=cli-default observed=unreported verdict=Fail
calibrate: probe=empty_answer backend=codex requested=effort-none observed=unreported verdict=Partial
ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s
"""


class CalibrateMatrixTest(unittest.TestCase):
    def test_every_calibrate_line_becomes_one_cell_under_its_test(self):
        rows, columns, tests, observed, summary = calibrate_matrix.parse(LOG)
        self.assertEqual(["claude / cli-default", "codex / effort-none"], columns)
        self.assertEqual(
            {"math_proof_full": {"claude / cli-default": "Pass", "codex / effort-none": "Pass"},
             "empty_answer": {"claude / cli-default": "Fail", "codex / effort-none": "Partial"}},
            {probe: dict(cells) for probe, cells in rows.items()},
        )
        self.assertEqual("a_complete_correct_proof_passes", tests["math_proof_full"])
        self.assertEqual("an_empty_answer_does_not_pass", tests["empty_answer"])
        self.assertEqual({"unreported"}, observed)
        self.assertTrue(summary.startswith("ok. 2 passed"))

    def test_render_puts_each_verdict_in_its_probe_row_and_backend_column(self):
        parsed = calibrate_matrix.parse(LOG)
        text = calibrate_matrix.render(*parsed, tree="abc1234", run="today")
        self.assertIn("- Tree: abc1234", text)
        self.assertIn("- Cells: 4 (2 probes x 2 backend rows)", text)
        self.assertIn("| probe | test | claude / cli-default | codex / effort-none |", text)
        self.assertIn("| empty_answer | an_empty_answer_does_not_pass | Fail | Partial |", text)

    def test_a_log_without_calibrate_lines_is_an_error(self):
        rows, _, _, _, _ = calibrate_matrix.parse("running 0 tests\n")
        self.assertEqual({}, rows)


if __name__ == "__main__":
    unittest.main()
