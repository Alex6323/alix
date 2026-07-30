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
    gate_ok: bool
    base_pedantic_warnings: int = 0
    check_ok: bool = True

    @property
    def cross_test_rate(self) -> float:
        if self.cross_tests_total == 0:
            return 1.0
        return self.cross_tests_passed / self.cross_tests_total

    @property
    def pedantic_warnings_added(self) -> int:
        return max(0, self.pedantic_warnings - self.base_pedantic_warnings)

    @property
    def cross_test_penalty(self) -> float:
        return (1.0 - self.cross_test_rate) * 1_000

    @property
    def mutant_penalty(self) -> int:
        return self.mutants_missed * 100

    @property
    def ineligible_reasons(self) -> tuple[str, ...]:
        reasons: list[str] = []
        if not self.check_ok:
            reasons.append("check failed")
        if self.unresolved_defects:
            reasons.append("unresolved verified defects")
        return tuple(reasons)

    @property
    def eligible(self) -> bool:
        return not self.ineligible_reasons

    @property
    def penalty(self) -> float:
        return (
            self.cross_test_penalty
            + self.mutant_penalty
            + self.pedantic_warnings_added * 2
            + self.diff_loc / 1_000
        )

    def to_dict(self) -> dict[str, object]:
        result: dict[str, object] = asdict(self)
        result["cross_test_rate"] = self.cross_test_rate
        result["cross_test_penalty"] = self.cross_test_penalty
        result["mutant_penalty"] = self.mutant_penalty
        result["pedantic_warnings_added"] = self.pedantic_warnings_added
        result["eligible"] = self.eligible
        result["ineligible_reasons"] = list(self.ineligible_reasons)
        result["penalty"] = self.penalty
        return result


def recommend(scores: list[BranchScore]) -> str | None:
    eligible = [score for score in scores if score.eligible]
    if not eligible:
        return None
    if len(scores) > 1 and len(eligible) != len(scores):
        return None
    ordered = sorted(eligible, key=lambda score: (score.penalty, score.agent))
    if len(ordered) > 1 and ordered[0].penalty == ordered[1].penalty:
        return None
    return ordered[0].agent
