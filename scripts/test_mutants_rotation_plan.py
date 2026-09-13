import datetime
import pathlib
import unittest

from mutants_rotation_plan import github_outputs, rotation_plan


ROOT = pathlib.Path(__file__).resolve().parent.parent


class MutantsRotationWorkflowTests(unittest.TestCase):
    def test_adjacent_utc_days_run_four_adjacent_thirty_sixths(self):
        self.assertEqual((36, [12, 13, 14, 15]), rotation_plan(219, ""))
        self.assertEqual((36, [16, 17, 18, 19]), rotation_plan(220, ""))

    def test_nine_consecutive_nights_cover_every_shard_once(self):
        shards = [
            shard
            for day in range(1, 10)
            for shard in rotation_plan(day, "")[1]
        ]

        self.assertEqual(list(range(36)), sorted(shards))

    def test_nine_nights_crossing_new_year_cover_every_shard_once(self):
        epoch = datetime.date(1970, 1, 1)
        start = datetime.date(2026, 12, 24)
        nights = [start + datetime.timedelta(days=offset) for offset in range(9)]
        shards = [
            shard
            for night in nights
            for shard in rotation_plan((night - epoch).days, "")[1]
        ]

        self.assertEqual(list(range(36)), sorted(shards))

    def test_manual_dispatch_runs_one_requested_physical_shard(self):
        self.assertEqual((36, [0]), rotation_plan(220, "0"))
        self.assertEqual((36, [35]), rotation_plan(220, "35"))
        self.assertEqual("count=36\nmatrix=[7]\n", github_outputs(220, "7"))

    def test_invalid_manual_shards_fail_before_starting_expensive_jobs(self):
        for requested in ["-1", "36", "four", "1.5"]:
            with self.subTest(requested=requested):
                with self.assertRaises(ValueError):
                    rotation_plan(220, requested)

        with self.assertRaises(ValueError):
            rotation_plan(-1, "")

    def test_workflow_uses_the_plan_for_matrix_denominator_and_artifacts(self):
        workflow = (ROOT / ".github/workflows/mutants-rotation.yml").read_text(
            encoding="utf-8"
        )
        nightly = (ROOT / ".github/workflows/mutants-nightly.yml").read_text(
            encoding="utf-8"
        )
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")

        self.assertIn("shard: ${{ fromJSON(needs.plan.outputs.matrix) }}", workflow)
        self.assertIn('day=$(( $(date -u +%s) / 86400 ))', workflow)
        self.assertIn(
            "MUTANTS_SHARD=${{ matrix.shard }}/${{ needs.plan.outputs.count }}",
            workflow,
        )
        self.assertIn(
            "mutants-rotation-shard-${{ matrix.shard }}-of-${{ needs.plan.outputs.count }}",
            workflow,
        )
        self.assertIn(
            "| initial timeout | $(count timeout-initial.txt) |",
            workflow,
            "rotation reports the initial timeout count",
        )
        self.assertIn(
            "| open after retry | $(count timeout.txt) |",
            workflow,
            "rotation reports the remaining open count",
        )
        self.assertIn(
            "make mutants", nightly, "nightly uses the shared retry-enabled target"
        )
        self.assertIn(
            "python3 scripts/mutants_timeout_retry.py mutants.out",
            makefile,
            "the shared mutation target invokes the timeout helper",
        )
        self.assertLess(
            makefile.index("python3 scripts/mutants_timeout_retry.py mutants.out"),
            makefile.index("python3 scripts/mutants_allowed.py mutants.out"),
            "timeouts are reclassified before survivor reconciliation",
        )


if __name__ == "__main__":
    unittest.main()
