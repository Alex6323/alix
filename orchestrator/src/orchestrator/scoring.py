from __future__ import annotations

from dataclasses import asdict, dataclass


@dataclass(frozen=True)
class BranchScore:
    agent: str
    cross_tests_passed: int
    cross_tests_total: int
    mutants_missed: int
    unresolved_defects: int
    pedantic_warnings: int
    diff_loc: int
    check_ok: bool

    @property
    def cross_tests_failed(self) -> int:
        return max(0, self.cross_tests_total - self.cross_tests_passed)

    @property
    def cross_test_rate(self) -> float:
        if self.cross_tests_total == 0:
            return 1.0
        return self.cross_tests_passed / self.cross_tests_total

    @property
    def penalty(self) -> float:
        return (
            self.unresolved_defects * 10_000
            + self.cross_tests_failed * 10_000
            + self.mutants_missed * 100
            + self.pedantic_warnings * 2
            + self.diff_loc / 1_000
        )

    def to_dict(self) -> dict[str, object]:
        result: dict[str, object] = asdict(self)
        result["cross_test_rate"] = self.cross_test_rate
        result["cross_tests_failed"] = self.cross_tests_failed
        result["penalty"] = self.penalty
        return result


def recommend(scores: list[BranchScore]) -> str | None:
    eligible = [score for score in scores if score.check_ok]
    if not eligible:
        return None
    ordered = sorted(eligible, key=lambda score: (score.penalty, score.agent))
    if len(ordered) > 1 and ordered[0].penalty == ordered[1].penalty:
        return None
    return ordered[0].agent
