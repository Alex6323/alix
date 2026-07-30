from __future__ import annotations

from dataclasses import dataclass, field
from typing import Literal, cast

AgentName = Literal["claude", "codex"]
Mode = Literal["symmetric", "asymmetric"]
FindingKind = Literal["defect", "preference"]


@dataclass(frozen=True)
class Invocation:
    exit_code: int
    transcript_path: str
    patch_path: str
    final_message: str
    duration_seconds: float
    tokens: int | None = None
    cost_usd: float | None = None


@dataclass
class AgentState:
    worktree: str
    branch: str
    last_sha: str

    def to_dict(self) -> dict[str, object]:
        return {
            "worktree": self.worktree,
            "branch": self.branch,
            "last_sha": self.last_sha,
        }

    @classmethod
    def from_dict(cls, value: object) -> AgentState:
        data = _dict(value, "agent")
        return cls(
            worktree=_string(data, "worktree"),
            branch=_string(data, "branch"),
            last_sha=_string(data, "last_sha"),
        )


@dataclass
class Finding:
    id: str
    author: AgentName
    against: AgentName
    kind: FindingKind
    test_patch: str
    verified: bool
    resolved: bool
    summary: str
    test_name: str = ""
    real_user_path: str = ""
    impact: str = ""
    observed: str = ""
    patch_sha256: str = ""
    target_sha: str = ""

    def to_dict(self) -> dict[str, object]:
        return {
            "id": self.id,
            "author": self.author,
            "against": self.against,
            "kind": self.kind,
            "test_patch": self.test_patch,
            "verified": self.verified,
            "resolved": self.resolved,
            "summary": self.summary,
            "test_name": self.test_name,
            "real_user_path": self.real_user_path,
            "impact": self.impact,
            "observed": self.observed,
            "patch_sha256": self.patch_sha256,
            "target_sha": self.target_sha,
        }

    @classmethod
    def from_dict(cls, value: object) -> Finding:
        data = _dict(value, "finding")
        author = _string(data, "author")
        against = _string(data, "against")
        kind = _string(data, "kind")
        if author not in ("claude", "codex") or against not in ("claude", "codex"):
            raise ValueError("finding has an unknown agent")
        if kind not in ("defect", "preference"):
            raise ValueError("finding has an unknown kind")
        return cls(
            id=_string(data, "id"),
            author=cast(AgentName, author),
            against=cast(AgentName, against),
            kind=cast(FindingKind, kind),
            test_patch=_string(data, "test_patch"),
            verified=_bool(data, "verified"),
            resolved=_bool(data, "resolved"),
            summary=_string(data, "summary"),
            test_name=_string(data, "test_name"),
            real_user_path=_string(data, "real_user_path"),
            impact=_string(data, "impact"),
            observed=_string(data, "observed"),
            patch_sha256=_string(data, "patch_sha256"),
            target_sha=_string(data, "target_sha"),
        )


@dataclass
class PhaseHistory:
    phase: str
    started: str
    ended: str
    ok: bool
    duration_seconds: float = 0.0
    detail: str | None = None

    def to_dict(self) -> dict[str, object]:
        result: dict[str, object] = {
            "phase": self.phase,
            "started": self.started,
            "ended": self.ended,
            "ok": self.ok,
            "duration_seconds": self.duration_seconds,
        }
        if self.detail is not None:
            result["detail"] = self.detail
        return result

    @classmethod
    def from_dict(cls, value: object) -> PhaseHistory:
        data = _dict(value, "history item")
        duration = data.get("duration_seconds", 0.0)
        if not isinstance(duration, (int, float)):
            raise ValueError("history duration_seconds must be a number")
        detail = data.get("detail")
        if detail is not None and not isinstance(detail, str):
            raise ValueError("history detail must be a string")
        return cls(
            phase=_string(data, "phase"),
            started=_string(data, "started"),
            ended=_string(data, "ended"),
            ok=_bool(data, "ok"),
            duration_seconds=float(duration),
            detail=detail,
        )


@dataclass
class RunState:
    run_id: str
    mode: Mode
    repo: str
    run_dir: str
    base: str
    base_sha: str
    phase: str
    agents: dict[str, AgentState]
    rounds_completed: int
    max_fix_rounds: int
    implementer: AgentName
    spec_hash: str
    prompt_hashes: dict[str, str]
    findings: list[Finding] = field(default_factory=list)
    history: list[PhaseHistory] = field(default_factory=list)
    token_usage: dict[str, int] = field(default_factory=dict)
    costs_usd: dict[str, float] = field(default_factory=dict)
    plan_path: str | None = None
    spec_bug: str | None = None
    property_suite_passed: bool | None = None
    property_failure: str | None = None
    test_bug_claim: str | None = None
    property_test_paths: list[str] = field(default_factory=list)
    completed_steps: list[str] = field(default_factory=list)
    schema_version: int = field(default=1, init=False)

    def to_dict(self) -> dict[str, object]:
        result: dict[str, object] = {
            "schema_version": self.schema_version,
            "run_id": self.run_id,
            "mode": self.mode,
            "repo": self.repo,
            "run_dir": self.run_dir,
            "base": self.base,
            "base_sha": self.base_sha,
            "phase": self.phase,
            "agents": {name: agent.to_dict() for name, agent in self.agents.items()},
            "rounds_completed": self.rounds_completed,
            "max_fix_rounds": self.max_fix_rounds,
            "implementer": self.implementer,
            "spec_hash": self.spec_hash,
            "prompt_hashes": self.prompt_hashes,
            "findings": [finding.to_dict() for finding in self.findings],
            "history": [item.to_dict() for item in self.history],
            "token_usage": self.token_usage,
            "costs_usd": self.costs_usd,
            "property_test_paths": self.property_test_paths,
            "completed_steps": self.completed_steps,
        }
        if self.plan_path is not None:
            result["plan_path"] = self.plan_path
        if self.spec_bug is not None:
            result["spec_bug"] = self.spec_bug
        if self.property_suite_passed is not None:
            result["property_suite_passed"] = self.property_suite_passed
        if self.property_failure is not None:
            result["property_failure"] = self.property_failure
        if self.test_bug_claim is not None:
            result["test_bug_claim"] = self.test_bug_claim
        return result

    @classmethod
    def from_dict(cls, value: object) -> RunState:
        data = _dict(value, "state")
        version = data.get("schema_version")
        if version != 1:
            raise ValueError(f"unsupported state schema version {version!r}")
        mode = _string(data, "mode")
        implementer = _string(data, "implementer")
        if mode not in ("symmetric", "asymmetric"):
            raise ValueError("state has an unknown mode")
        if implementer not in ("claude", "codex"):
            raise ValueError("state has an unknown implementer")
        raw_agents = _dict(data.get("agents"), "agents")
        raw_findings = _list(data, "findings")
        raw_history = _list(data, "history")
        plan_path = data.get("plan_path")
        spec_bug = data.get("spec_bug")
        property_suite_passed = data.get("property_suite_passed")
        property_failure = data.get("property_failure")
        test_bug_claim = data.get("test_bug_claim")
        if plan_path is not None and not isinstance(plan_path, str):
            raise ValueError("state plan_path must be a string")
        if spec_bug is not None and not isinstance(spec_bug, str):
            raise ValueError("state spec_bug must be a string")
        if property_suite_passed is not None and not isinstance(
            property_suite_passed, bool
        ):
            raise ValueError("state property_suite_passed must be a boolean")
        if property_failure is not None and not isinstance(property_failure, str):
            raise ValueError("state property_failure must be a string")
        if test_bug_claim is not None and not isinstance(test_bug_claim, str):
            raise ValueError("state test_bug_claim must be a string")
        return cls(
            run_id=_string(data, "run_id"),
            mode=cast(Mode, mode),
            repo=_string(data, "repo"),
            run_dir=_string(data, "run_dir"),
            base=_string(data, "base"),
            base_sha=_string(data, "base_sha"),
            phase=_string(data, "phase"),
            agents={
                name: AgentState.from_dict(agent) for name, agent in raw_agents.items()
            },
            rounds_completed=_int(data, "rounds_completed"),
            max_fix_rounds=_int(data, "max_fix_rounds"),
            implementer=cast(AgentName, implementer),
            spec_hash=_string(data, "spec_hash"),
            prompt_hashes=_string_map(data, "prompt_hashes"),
            findings=[Finding.from_dict(item) for item in raw_findings],
            history=[PhaseHistory.from_dict(item) for item in raw_history],
            token_usage=_int_map(data, "token_usage"),
            costs_usd=_float_map(data, "costs_usd"),
            plan_path=plan_path,
            spec_bug=spec_bug,
            property_suite_passed=property_suite_passed,
            property_failure=property_failure,
            test_bug_claim=test_bug_claim,
            property_test_paths=[
                _string_value(item, "property_test_paths")
                for item in _list(data, "property_test_paths")
            ],
            completed_steps=[
                _string_value(item, "completed_steps")
                for item in _list(data, "completed_steps")
            ],
        )


def _dict(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value):
        raise ValueError(f"{label} must be an object")
    return cast(dict[str, object], value)


def _list(data: dict[str, object], key: str) -> list[object]:
    value = data.get(key, [])
    if not isinstance(value, list):
        raise ValueError(f"{key} must be a list")
    return cast(list[object], value)


def _string(data: dict[str, object], key: str) -> str:
    value = data.get(key)
    if not isinstance(value, str):
        raise ValueError(f"{key} must be a string")
    return value


def _bool(data: dict[str, object], key: str) -> bool:
    value = data.get(key)
    if not isinstance(value, bool):
        raise ValueError(f"{key} must be a boolean")
    return value


def _int(data: dict[str, object], key: str) -> int:
    value = data.get(key)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{key} must be an integer")
    return value


def _string_map(data: dict[str, object], key: str) -> dict[str, str]:
    values = _dict(data.get(key, {}), key)
    if not all(isinstance(value, str) for value in values.values()):
        raise ValueError(f"{key} values must be strings")
    return cast(dict[str, str], values)


def _int_map(data: dict[str, object], key: str) -> dict[str, int]:
    values = _dict(data.get(key, {}), key)
    if not all(isinstance(value, int) and not isinstance(value, bool) for value in values.values()):
        raise ValueError(f"{key} values must be integers")
    return cast(dict[str, int], values)


def _float_map(data: dict[str, object], key: str) -> dict[str, float]:
    values = _dict(data.get(key, {}), key)
    result: dict[str, float] = {}
    for name, value in values.items():
        if not isinstance(value, (int, float)) or isinstance(value, bool):
            raise ValueError(f"{key} values must be numbers")
        result[name] = float(value)
    return result


def _string_value(value: object, label: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{label} values must be strings")
    return value
