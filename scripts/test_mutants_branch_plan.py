import pathlib
import unittest

from mutants_branch_plan import github_outputs, shard_plan


ROOT = pathlib.Path(__file__).resolve().parent.parent


class MutantsBranchWorkflowTests(unittest.TestCase):
    def test_empty_input_preserves_the_four_shard_push_default(self):
        self.assertEqual((4, [0, 1, 2, 3]), shard_plan(""))

    def test_manual_input_builds_every_requested_zero_based_shard(self):
        self.assertEqual((1, [0]), shard_plan("1"))
        self.assertEqual((6, [0, 1, 2, 3, 4, 5]), shard_plan("6"))
        self.assertEqual("count=3\nmatrix=[0,1,2]\n", github_outputs("3"))

    def test_invalid_shard_counts_fail_before_starting_expensive_jobs(self):
        for requested in ["0", "65", "four", "1.5", "-1"]:
            with self.subTest(requested=requested):
                with self.assertRaises(ValueError):
                    shard_plan(requested)

    def test_manual_shard_count_drives_the_matrix_and_each_shard_denominator(self):
        workflow = (ROOT / ".github/workflows/mutants-branch.yml").read_text(encoding="utf-8")

        self.assertIn('REQUESTED_SHARDS: ${{ inputs.shards }}', workflow)
        self.assertIn('shard: ${{ fromJSON(needs.plan.outputs.matrix) }}', workflow)
        self.assertIn('MUTANTS_SHARD=${{ matrix.shard }}/${{ needs.plan.outputs.count }}', workflow)


if __name__ == "__main__":
    unittest.main()
