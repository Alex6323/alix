from __future__ import annotations

import hashlib
import json
import re
import secrets
import shutil
import subprocess
import tarfile
import time
import tomllib
from concurrent.futures import Future, ThreadPoolExecutor
from contextlib import contextmanager
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime
from pathlib import Path
from typing import Iterator, cast

from orchestrator.agents import Invoker
from orchestrator.commands import Executor
from orchestrator.models import (
    AgentName,
    AgentState,
    Finding,
    Invocation,
    Mode,
    PhaseHistory,
    RunState,
)
from orchestrator.report import render_report
from orchestrator.review import (
    ReviewCandidate,
    ReviewManifestError,
    load_review_candidates,
)
from orchestrator.protocol import next_asymmetric_phase, next_symmetric_phase
from orchestrator.spec_api import ApiContractError, extract_rust_api
from orchestrator.storage import save_state
from orchestrator.validation import patch_paths, validate_patch
from orchestrator.scoring import BranchScore, recommend

AGENT_TIMEOUT_SECONDS = 7_200.0


@dataclass(frozen=True)
class ReviewVerification:
    verified: bool
    observed: str
    reason: str


@dataclass(frozen=True)
class RunOptions:
    mode: Mode
    spec: Path
    plan: Path | None
    repo: Path
    base: str
    run_root: Path
    max_fix_rounds: int
    implementer: AgentName
    models: dict[str, str] = field(default_factory=dict)


def initialize_run(options: RunOptions, run_id: str | None = None) -> RunState:
    repo = options.repo.resolve(strict=True)
    spec = options.spec.resolve(strict=True)
    plan = options.plan.resolve(strict=True) if options.plan is not None else None
    if options.max_fix_rounds < 0:
        raise ValueError("max fix rounds cannot be negative")
    _git(repo, "rev-parse", "--show-toplevel")
    base_sha = _git(repo, "rev-parse", f"{options.base}^{{commit}}")
    identifier = run_id or _new_run_id(options.mode)
    run_root = options.run_root.resolve()
    if run_root == repo or run_root.is_relative_to(repo):
        raise ValueError("the run directory must be outside the target repository")
    run_dir = run_root / identifier
    if run_dir.exists():
        raise ValueError(f"run directory already exists: {run_dir}")
    for name in ("transcripts", "patches", "findings"):
        (run_dir / name).mkdir(parents=True, exist_ok=True)
    shutil.copyfile(spec, run_dir / "spec.md")
    if plan is not None:
        shutil.copyfile(plan, run_dir / "plan.md")

    state = RunState(
        run_id=identifier,
        mode=options.mode,
        repo=str(repo),
        run_dir=str(run_dir),
        base=options.base,
        base_sha=base_sha,
        phase="IMPLEMENT" if options.mode == "symmetric" else "IMPLEMENT_PROPERTIES",
        agents={},
        rounds_completed=0,
        max_fix_rounds=options.max_fix_rounds,
        implementer=options.implementer,
        spec_hash=_sha256(run_dir / "spec.md"),
        prompt_hashes=_prompt_hashes(),
        models=dict(options.models),
        plan_path="plan.md" if plan is not None else None,
    )
    save_state(run_dir / "state.json", state)

    worktree_root = run_root / ".worktrees" / identifier
    worktree_root.mkdir(parents=True, exist_ok=True)
    if options.mode == "symmetric":
        for agent in ("claude", "codex"):
            state.agents[agent] = _create_target_worktree(
                repo, worktree_root, identifier, agent, base_sha
            )
    else:
        implementer = options.implementer
        property_author: AgentName = "codex" if implementer == "claude" else "claude"
        state.agents[implementer] = _create_target_worktree(
            repo, worktree_root, identifier, implementer, base_sha
        )
        try:
            api = extract_rust_api((run_dir / "spec.md").read_text(encoding="utf-8"))
        except ApiContractError as error:
            state.phase = "COMPLETE"
            state.spec_bug = str(error)
            save_state(run_dir / "state.json", state)
            (run_dir / "report.md").write_text(
                render_report(state, []), encoding="utf-8"
            )
            return state
        state.agents[property_author] = _create_property_worktree(
            repo,
            worktree_root,
            identifier,
            property_author,
            api,
        )
    save_state(run_dir / "state.json", state)
    return state


def run_implementation_phase(state: RunState, invoker: Invoker) -> None:
    if state.phase != "IMPLEMENT" or state.mode != "symmetric":
        raise ValueError("implementation phase requires symmetric IMPLEMENT state")
    started_wall = _now()
    started = time.monotonic()
    details: list[str] = []
    pending: list[tuple[AgentName, str, str]] = []
    for agent in ("claude", "codex"):
        step = f"IMPLEMENT:{agent}"
        if step in state.completed_steps:
            continue
        prompt = _render_prompt(
            state,
            "implement",
            Path(state.agents[agent].worktree),
            failure_summary="",
        )
        pending.append((agent, step, prompt))
    with _invocation_pool(invoker, max(1, len(pending))) as pool:
        futures = {
            agent: pool.submit(
                _run_agent_change,
                state,
                agent,
                prompt,
                invoker,
                f"[orchestrator] implement {agent}",
            )
            for agent, _, prompt in pending
        }
        errors: list[Exception] = []
        for agent, step, _ in pending:
            try:
                details.extend(futures[agent].result())
                state.completed_steps.append(step)
                save_state(Path(state.run_dir) / "state.json", state)
            except Exception as error:
                errors.append(error)
        if errors:
            raise errors[0]
    state.phase = "REVIEW_ROUND_1" if state.max_fix_rounds > 0 else "SCORE"
    state.completed_steps.clear()
    state.history.append(
        PhaseHistory(
            phase="IMPLEMENT",
            started=started_wall,
            ended=_now(),
            ok=True,
            duration_seconds=time.monotonic() - started,
            detail="; ".join(details) or None,
        )
    )
    save_state(Path(state.run_dir) / "state.json", state)


def run_asymmetric_implementation_phase(
    state: RunState,
    invoker: Invoker,
    executor: Executor,
) -> None:
    if state.phase != "IMPLEMENT_PROPERTIES" or state.mode != "asymmetric":
        raise ValueError(
            "asymmetric implementation requires IMPLEMENT_PROPERTIES state"
        )
    started_wall = _now()
    started = time.monotonic()
    implementer = state.implementer
    property_author: AgentName = (
        "codex" if implementer == "claude" else "claude"
    )
    implementer_worktree = Path(state.agents[implementer].worktree)
    implementation_prompt = _render_prompt(
        state,
        "implement",
        implementer_worktree,
        failure_summary="",
    )
    implement_step = f"IMPLEMENT_PROPERTIES:{implementer}"
    property_worktree = Path(state.agents[property_author].worktree)
    properties_prompt = _render_prompt(
        state,
        "properties",
        property_worktree,
        failure_summary="",
    )
    property_step = f"IMPLEMENT_PROPERTIES:{property_author}"
    completed = True
    tasks: dict[AgentName, tuple[str, Future[object]]] = {}
    with _invocation_pool(invoker, 2) as pool:
        if implement_step not in state.completed_steps:
            tasks[implementer] = (
                implement_step,
                pool.submit(
                    _run_agent_change,
                    state,
                    implementer,
                    implementation_prompt,
                    invoker,
                    f"[orchestrator] implement {implementer}",
                ),
            )
        if property_step not in state.completed_steps:
            tasks[property_author] = (
                property_step,
                pool.submit(
                    _run_property_change,
                    state,
                    property_author,
                    properties_prompt,
                    invoker,
                    executor,
                ),
            )
        errors: list[Exception] = []
        for agent in (implementer, property_author):
            task = tasks.get(agent)
            if task is None:
                continue
            step, future = task
            try:
                result = future.result()
                if agent == property_author:
                    completed = cast(bool, result)
                    if not completed:
                        continue
                state.completed_steps.append(step)
                save_state(Path(state.run_dir) / "state.json", state)
            except Exception as error:
                errors.append(error)
        if errors:
            raise errors[0]
    phase = state.phase
    if completed:
        state.phase = "RUN"
        state.completed_steps.clear()
    state.history.append(
        PhaseHistory(
            phase=phase,
            started=started_wall,
            ended=_now(),
            ok=completed,
            duration_seconds=time.monotonic() - started,
            detail=state.spec_bug,
        )
    )
    if not completed:
        state.phase = "COMPLETE"
        (Path(state.run_dir) / "report.md").write_text(
            render_report(state, []),
            encoding="utf-8",
        )
    save_state(Path(state.run_dir) / "state.json", state)


def run_asymmetric_test_phase(
    state: RunState,
    executor: Executor,
    invoker: Invoker | None = None,
) -> None:
    if state.mode != "asymmetric" or not (
        state.phase == "RUN" or re.fullmatch(r"RUN_ROUND_\d+", state.phase)
    ):
        raise ValueError("property run requires an asymmetric RUN state")
    phase_name = state.phase
    started_wall = _now()
    started = time.monotonic()
    implementer = state.implementer
    property_author: AgentName = (
        "codex" if implementer == "claude" else "claude"
    )
    if phase_name == "RUN":
        install_step = "RUN:install-properties"
        if install_step not in state.completed_steps:
            _install_property_tests(state, implementer, property_author)
            state.completed_steps.append(install_step)
            save_state(Path(state.run_dir) / "state.json", state)
    worktree = Path(state.agents[implementer].worktree)
    result = executor.run(["cargo", "nextest", "run"], worktree)
    transcript = Path(state.run_dir) / "transcripts" / f"{phase_name.lower()}.txt"
    transcript.write_text(
        f"[stdout]\n{result.stdout}\n[stderr]\n{result.stderr}",
        encoding="utf-8",
    )
    state.property_suite_passed = result.returncode == 0
    state.property_failure = (
        None if state.property_suite_passed else _output(result.stdout, result.stderr)
    )
    state.phase = next_asymmetric_phase(
        phase_name,
        state.max_fix_rounds,
        state.property_suite_passed,
    )
    if (
        state.phase == "SCORE"
        and not state.property_suite_passed
        and state.test_bug_claim
        and invoker is not None
    ):
        _adjudicate_property_claim(state, invoker, executor)
    state.completed_steps.clear()
    state.history.append(
        PhaseHistory(
            phase=phase_name,
            started=started_wall,
            ended=_now(),
            ok=state.property_suite_passed,
            duration_seconds=time.monotonic() - started,
            detail=str(transcript.relative_to(Path(state.run_dir))),
        )
    )
    save_state(Path(state.run_dir) / "state.json", state)


def _adjudicate_property_claim(
    state: RunState,
    invoker: Invoker,
    executor: Executor,
) -> None:
    implementer = state.implementer
    property_author: AgentName = (
        "codex" if implementer == "claude" else "claude"
    )
    property_state = state.agents[property_author]
    property_worktree = Path(property_state.worktree)
    _reset_worktree(property_worktree, property_state.last_sha)
    prompt = _render_prompt(
        state,
        "property_test_bug",
        property_worktree,
        failure_summary=state.property_failure or "property suite failed",
        test_bug_claim=state.test_bug_claim or "",
    )
    invocation = invoker.invoke(
        property_author,
        prompt,
        property_worktree,
        AGENT_TIMEOUT_SECONDS,
    )
    _record_usage(
        state,
        property_author,
        invocation.tokens,
        invocation.cost_usd,
    )
    if invocation.exit_code != 0:
        raise RuntimeError(
            f"{property_author} test adjudication failed; see "
            f"{invocation.transcript_path}"
        )
    patch = Path(invocation.patch_path).read_text(encoding="utf-8")
    paths = patch_paths(patch)
    if any(not path.startswith("tests/") for path in paths):
        raise RuntimeError("property adjudication may change only tests/")
    compiled = executor.run(["cargo", "test", "--no-run"], property_worktree)
    if compiled.returncode != 0:
        raise RuntimeError(
            "adjudicated property suite does not compile: "
            + _output(compiled.stdout, compiled.stderr)
        )
    _uniform_commit(
        property_worktree,
        property_state.last_sha,
        f"[orchestrator] adjudicate properties {property_author}",
    )
    property_state.last_sha = _git(property_worktree, "rev-parse", "HEAD")
    _sync_property_tests(state, implementer, property_author)
    implementation = Path(state.agents[implementer].worktree)
    rerun = executor.run(["cargo", "nextest", "run"], implementation)
    state.property_suite_passed = rerun.returncode == 0
    state.property_failure = (
        None
        if state.property_suite_passed
        else _output(rerun.stdout, rerun.stderr)
    )


def _sync_property_tests(
    state: RunState,
    implementer: AgentName,
    property_author: AgentName,
) -> None:
    implementation = Path(state.agents[implementer].worktree)
    properties = Path(state.agents[property_author].worktree)
    _reset_worktree(implementation, state.agents[implementer].last_sha)
    names = [
        name
        for name in _git(properties, "ls-files", "tests").splitlines()
        if name
    ]
    for old in state.property_test_paths:
        if old not in names and (implementation / old).is_file():
            (implementation / old).unlink()
    for name in names:
        target = implementation / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(properties / name, target)
    state.property_test_paths = names
    _uniform_commit(
        implementation,
        state.agents[implementer].last_sha,
        "[orchestrator] adjudicated property suite",
    )
    state.agents[implementer].last_sha = _git(
        implementation, "rev-parse", "HEAD"
    )


def run_asymmetric_fix_phase(state: RunState, invoker: Invoker) -> None:
    match = re.fullmatch(r"FIX_ROUND_(\d+)", state.phase)
    if state.mode != "asymmetric" or match is None:
        raise ValueError("asymmetric fix requires a FIX_ROUND state")
    round_number = int(match.group(1))
    phase_name = state.phase
    started_wall = _now()
    started = time.monotonic()
    implementer = state.implementer
    agent_state = state.agents[implementer]
    worktree = Path(agent_state.worktree)
    step = f"{phase_name}:{implementer}"
    if step in state.completed_steps:
        state.phase = next_asymmetric_phase(
            phase_name,
            state.max_fix_rounds,
            suite_passed=False,
        )
        state.completed_steps.clear()
        save_state(Path(state.run_dir) / "state.json", state)
        return
    prompt = _render_prompt(
        state,
        "fix",
        worktree,
        failure_summary=state.property_failure or "property suite failed",
    )
    rejection: str | None = None
    for attempt in range(2):
        _reset_worktree(worktree, agent_state.last_sha)
        expected = {
            path: (worktree / path).read_bytes()
            for path in state.property_test_paths
            if (worktree / path).is_file()
        }
        current_prompt = prompt
        if rejection is not None:
            current_prompt += (
                "\n\nYour previous fix changed the independent test suite or "
                "failed the patch guard:\n"
                f"{rejection}\nStart again from the clean fix baseline."
            )
        invocation = invoker.invoke(
            implementer,
            current_prompt,
            worktree,
            AGENT_TIMEOUT_SECONDS,
        )
        _record_usage(
            state,
            implementer,
            invocation.tokens,
            invocation.cost_usd,
        )
        if invocation.exit_code != 0:
            raise RuntimeError(
                f"{implementer} fix failed; see {invocation.transcript_path}"
            )
        changed_tests = [
            path
            for path, content in expected.items()
            if not (worktree / path).is_file()
            or (worktree / path).read_bytes() != content
        ]
        validation = validate_patch(
            Path(invocation.patch_path).read_text(encoding="utf-8")
        )
        reasons = list(validation.reasons)
        if changed_tests:
            reasons.append(
                "independent property tests changed: " + ", ".join(changed_tests)
            )
        if reasons:
            rejection = "; ".join(reasons)
            if attempt == 0:
                continue
            raise RuntimeError(
                f"{implementer} asymmetric fix rejected twice: {rejection}"
            )
        marker = "TEST_BUG:"
        if marker in invocation.final_message:
            state.test_bug_claim = invocation.final_message.split(marker, 1)[1].strip()
        _uniform_commit(
            worktree,
            agent_state.last_sha,
            f"[orchestrator] fix round {round_number} {implementer}",
        )
        agent_state.last_sha = _git(worktree, "rev-parse", "HEAD")
        break
    else:
        raise AssertionError("unreachable retry loop")
    state.completed_steps.append(step)
    save_state(Path(state.run_dir) / "state.json", state)
    state.phase = next_asymmetric_phase(
        phase_name,
        state.max_fix_rounds,
        suite_passed=False,
    )
    state.completed_steps.clear()
    state.rounds_completed = round_number
    state.history.append(
        PhaseHistory(
            phase=phase_name,
            started=started_wall,
            ended=_now(),
            ok=True,
            duration_seconds=time.monotonic() - started,
            detail=(
                "implementer claims a test bug"
                if state.test_bug_claim
                else "production fix applied"
            ),
        )
    )
    save_state(Path(state.run_dir) / "state.json", state)


def run_score_phase(state: RunState, executor: Executor) -> list[BranchScore]:
    if state.phase != "SCORE":
        raise ValueError("score phase requires SCORE state")
    started_wall = _now()
    started = time.monotonic()
    if state.mode == "symmetric":
        names: list[AgentName] = ["claude", "codex"]
    else:
        names = [state.implementer]
    scores: list[BranchScore] = []
    for agent in names:
        worktree = Path(state.agents[agent].worktree)
        if state.mode == "symmetric":
            opponent: AgentName = "codex" if agent == "claude" else "claude"
            cross_passed, cross_total = _cross_tests(
                state,
                agent,
                opponent,
                executor,
            )
        else:
            cross_total = 1
            cross_passed = 1 if state.property_suite_passed else 0
        check = executor.run(["make", "check"], worktree)
        check_transcript = (
            Path(state.run_dir) / "transcripts" / f"score-check-{agent}.txt"
        )
        check_transcript.write_text(
            f"[stdout]\n{check.stdout}\n[stderr]\n{check.stderr}",
            encoding="utf-8",
        )
        mutants = (
            executor.run(["make", "mutants"], worktree)
            if check.returncode == 0
            else None
        )
        gate_transcript = (
            Path(state.run_dir) / "transcripts" / f"score-gate-{agent}.txt"
        )
        gate_transcript.write_text(
            f"[check stdout]\n{check.stdout}\n[check stderr]\n{check.stderr}\n"
            + (
                f"[mutants stdout]\n{mutants.stdout}\n"
                f"[mutants stderr]\n{mutants.stderr}"
                if mutants is not None
                else "[mutants] skipped: make check failed"
            ),
            encoding="utf-8",
        )
        scores.append(
            BranchScore(
                agent=agent,
                cross_tests_passed=cross_passed,
                cross_tests_total=cross_total,
                mutants_missed=(
                    _missed_mutants(mutants.stdout + "\n" + mutants.stderr)
                    if mutants is not None
                    else 0
                ),
                unresolved_defects=sum(
                    finding.verified
                    and not finding.resolved
                    and finding.against == agent
                    for finding in state.findings
                ),
                pedantic_warnings=_pedantic_warnings(executor, worktree),
                diff_loc=_diff_loc(
                    worktree,
                    state.base_sha,
                    state.agents[agent].last_sha,
                ),
                check_ok=check.returncode == 0,
            )
        )
    baseline_worktree = Path(state.agents[names[0]].worktree)
    baseline_tip = state.agents[names[0]].last_sha
    _git(baseline_worktree, "reset", "--hard", state.base_sha)
    try:
        base_pedantic_warnings = _pedantic_warnings(executor, baseline_worktree)
    finally:
        _git(baseline_worktree, "reset", "--hard", baseline_tip)
    scores = [
        replace(score, base_pedantic_warnings=base_pedantic_warnings)
        for score in scores
    ]
    _save_scores(Path(state.run_dir) / "scores.json", scores)
    winner = recommend(scores)
    state.phase = "LAND" if winner is not None else "COMPLETE"
    state.history.append(
        PhaseHistory(
            phase="SCORE",
            started=started_wall,
            ended=_now(),
            ok=winner is not None,
            duration_seconds=time.monotonic() - started,
            detail=f"recommendation: {winner or 'human decision'}",
        )
    )
    save_state(Path(state.run_dir) / "state.json", state)
    (Path(state.run_dir) / "report.md").write_text(
        render_report(state, scores, divergence_notes=_divergence_notes(state)),
        encoding="utf-8",
    )
    return scores


def run_land_phase(state: RunState) -> None:
    if state.phase != "LAND":
        raise ValueError("land phase requires LAND state")
    scores = _load_scores(Path(state.run_dir) / "scores.json")
    winner_name = recommend(scores)
    if winner_name not in ("claude", "codex"):
        raise RuntimeError("landing requires one passing machine recommendation")
    winner = cast(AgentName, winner_name)
    repo = Path(state.repo)
    started_wall = _now()
    started = time.monotonic()
    landing_branch = f"orchestrator/land/{state.run_id}"
    current_branch = _git(repo, "symbolic-ref", "--short", "HEAD")
    if current_branch != state.base:
        raise RuntimeError(
            f"target repository must have base branch {state.base!r} checked out"
        )
    if _git(repo, "status", "--porcelain"):
        raise RuntimeError("target repository must be clean before landing")
    current_base = _git(repo, "rev-parse", f"{state.base}^{{commit}}")
    if current_base != state.base_sha:
        landing_ref = f"refs/heads/{landing_branch}"
        landed_by_this_run = subprocess.run(
            ["git", "show-ref", "--verify", "--quiet", landing_ref],
            cwd=repo,
            check=False,
        ).returncode == 0 and _git(repo, "rev-parse", landing_ref) == current_base
        if not landed_by_this_run:
            raise RuntimeError(
                f"base {state.base!r} moved after setup; rerun or rebase explicitly"
            )
        state.phase = "COMPLETE"
        state.history.append(
            PhaseHistory(
                phase="LAND",
                started=started_wall,
                ended=_now(),
                ok=True,
                duration_seconds=time.monotonic() - started,
                detail=f"recognized already-landed {winner} after resume",
            )
        )
        save_state(Path(state.run_dir) / "state.json", state)
        (Path(state.run_dir) / "report.md").write_text(
            render_report(
                state,
                scores,
                divergence_notes=_divergence_notes(state),
            ),
            encoding="utf-8",
        )
        return
    worktree_root = Path(next(iter(state.agents.values())).worktree).parent
    landing = worktree_root / "landing"
    if landing.exists():
        _git(repo, "worktree", "remove", "--force", str(landing))
    branch_exists = subprocess.run(
        ["git", "show-ref", "--verify", "--quiet", f"refs/heads/{landing_branch}"],
        cwd=repo,
        check=False,
    ).returncode == 0
    if branch_exists:
        _git(repo, "branch", "-D", landing_branch)
    _git(
        repo,
        "worktree",
        "add",
        "-b",
        landing_branch,
        str(landing),
        state.base_sha,
    )

    for agent in state.agents:
        if state.mode == "asymmetric" and agent != state.implementer:
            continue
        agent_state = state.agents[agent]
        patch_text = _git(
            Path(agent_state.worktree),
            "diff",
            "--binary",
            state.base_sha,
            agent_state.last_sha,
            "--",
            "tests",
        )
        if patch_text.strip():
            patch = (
                Path(state.run_dir) / "patches" / f"land-tests-{agent}.patch"
            )
            patch.write_text(patch_text + "\n", encoding="utf-8")
            _apply_union_patch(landing, patch)
    for finding in state.findings:
        if not finding.verified or not finding.test_patch:
            continue
        _apply_union_patch(landing, Path(state.run_dir) / finding.test_patch)
    _commit_if_changes(landing, "[orchestrator] land tests")

    winner_state = state.agents[winner]
    implementation_patch = Path(state.run_dir) / "patches" / "land-implementation.patch"
    implementation_patch.write_text(
        _git(
            Path(winner_state.worktree),
            "diff",
            "--binary",
            state.base_sha,
            winner_state.last_sha,
            "--",
            ".",
            ":(exclude)tests/**",
        )
        + "\n",
        encoding="utf-8",
    )
    if implementation_patch.read_text(encoding="utf-8").strip():
        _apply_union_patch(landing, implementation_patch)
    _commit_if_changes(
        landing,
        f"[orchestrator] land implementation {winner}",
    )
    _git(repo, "merge", "--ff-only", landing_branch)
    state.phase = "COMPLETE"
    state.history.append(
        PhaseHistory(
            phase="LAND",
            started=started_wall,
            ended=_now(),
            ok=True,
            duration_seconds=time.monotonic() - started,
            detail=f"landed {winner} after union tests",
        )
    )
    save_state(Path(state.run_dir) / "state.json", state)
    (Path(state.run_dir) / "report.md").write_text(
        render_report(state, scores, divergence_notes=_divergence_notes(state)),
        encoding="utf-8",
    )


def _apply_union_patch(worktree: Path, patch: Path) -> None:
    check = subprocess.run(
        ["git", "apply", "--check", str(patch)],
        cwd=worktree,
        check=False,
        text=True,
        capture_output=True,
    )
    if check.returncode == 0:
        _git(worktree, "apply", str(patch))
        return
    reverse = subprocess.run(
        ["git", "apply", "--reverse", "--check", str(patch)],
        cwd=worktree,
        check=False,
        text=True,
        capture_output=True,
    )
    if reverse.returncode == 0:
        return
    raise RuntimeError(
        f"test union conflict for {patch}: {check.stderr.strip()}"
    )


def _commit_if_changes(worktree: Path, message: str) -> None:
    _git(worktree, "add", "-A")
    if not _git(worktree, "status", "--porcelain"):
        return
    _git(
        worktree,
        "-c",
        "user.name=orchestrator",
        "-c",
        "user.email=orchestrator@invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-m",
        message,
    )


def _load_scores(path: Path) -> list[BranchScore]:
    with path.open(encoding="utf-8") as handle:
        raw = cast(object, json.load(handle))
    if not isinstance(raw, list):
        raise ValueError("scores.json must contain an array")
    scores: list[BranchScore] = []
    for value in cast(list[object], raw):
        if not isinstance(value, dict):
            raise ValueError("score must be an object")
        data = cast(dict[object, object], value)
        scores.append(
            BranchScore(
                agent=_score_string(data, "agent"),
                cross_tests_passed=_score_int(data, "cross_tests_passed"),
                cross_tests_total=_score_int(data, "cross_tests_total"),
                mutants_missed=_score_int(data, "mutants_missed"),
                unresolved_defects=_score_int(data, "unresolved_defects"),
                pedantic_warnings=_score_int(data, "pedantic_warnings"),
                diff_loc=_score_int(data, "diff_loc"),
                check_ok=_score_bool(data, "check_ok"),
                base_pedantic_warnings=_score_optional_int(
                    data,
                    "base_pedantic_warnings",
                    0,
                ),
            )
        )
    return scores


def _score_string(data: dict[object, object], key: str) -> str:
    value = data.get(key)
    if not isinstance(value, str):
        raise ValueError(f"score {key} must be a string")
    return value


def _score_int(data: dict[object, object], key: str) -> int:
    value = data.get(key)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"score {key} must be an integer")
    return value


def _score_optional_int(
    data: dict[object, object],
    key: str,
    default: int,
) -> int:
    value = data.get(key, default)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"score {key} must be an integer")
    return value


def _score_bool(data: dict[object, object], key: str) -> bool:
    value = data.get(key)
    if not isinstance(value, bool):
        raise ValueError(f"score {key} must be a boolean")
    return value


def _score_optional_bool(
    data: dict[object, object],
    key: str,
    default: bool,
) -> bool:
    value = data.get(key, default)
    if not isinstance(value, bool):
        raise ValueError(f"score {key} must be a boolean")
    return value


def _pedantic_warnings(executor: Executor, worktree: Path) -> int:
    result = executor.run(
        [
            "cargo",
            "clippy",
            "--all-targets",
            "--",
            "-W",
            "clippy::pedantic",
        ],
        worktree,
    )
    return len(
        re.findall(
            r"(?m)^warning:",
            result.stdout + "\n" + result.stderr,
        )
    )


def _cross_tests(
    state: RunState,
    agent: AgentName,
    opponent: AgentName,
    executor: Executor,
) -> tuple[int, int]:
    opponent_worktree = Path(state.agents[opponent].worktree)
    names = [
        name
        for name in _git(
            opponent_worktree,
            "diff",
            "--name-only",
            "--diff-filter=AM",
            state.base_sha,
            state.agents[opponent].last_sha,
            "--",
            "tests",
        ).splitlines()
        if name
    ]
    passed = 0
    total = 0
    scratch_root = (
        Path(next(iter(state.agents.values())).worktree).parent
        / "cross-tests"
        / agent
    )
    for index, name in enumerate(names, start=1):
        scratch = scratch_root / str(index)
        _export_tree(
            Path(state.repo),
            state.agents[agent].last_sha,
            scratch,
        )
        source = opponent_worktree / name
        target = scratch / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        test_target = Path(name).stem
        compiled = executor.run(
            ["cargo", "nextest", "run", "--no-run", "--test", test_target],
            scratch,
        )
        if compiled.returncode != 0:
            continue
        total += 1
        result = executor.run(
            ["cargo", "nextest", "run", "--test", test_target],
            scratch,
        )
        if result.returncode == 0:
            passed += 1
    return passed, total


def _missed_mutants(output: str) -> int:
    matches = re.findall(r"(?i)\b(\d+)\s+missed\b", output)
    if matches:
        return int(matches[-1])
    return len(re.findall(r"(?m)^.*\bMISSED\b.*$", output))


def _diff_loc(worktree: Path, base_sha: str, tip: str) -> int:
    total = 0
    for line in _git(worktree, "diff", "--numstat", base_sha, tip).splitlines():
        fields = line.split("\t", 2)
        if len(fields) < 2 or "-" in fields[:2]:
            continue
        total += int(fields[0]) + int(fields[1])
    return total


def _save_scores(path: Path, scores: list[BranchScore]) -> None:
    temporary = path.with_name(f"{path.name}.tmp")
    temporary.write_text(
        json.dumps([score.to_dict() for score in scores], indent=2, sort_keys=True)
        + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def _divergence_notes(state: RunState) -> list[str]:
    if state.mode == "asymmetric":
        if state.property_suite_passed:
            return ["The independent property suite passed."]
        return [state.property_failure or "The independent property suite failed."]
    claude = set(
        _git(
            Path(state.agents["claude"].worktree),
            "diff",
            "--name-only",
            state.base_sha,
            state.agents["claude"].last_sha,
        ).splitlines()
    )
    codex = set(
        _git(
            Path(state.agents["codex"].worktree),
            "diff",
            "--name-only",
            state.base_sha,
            state.agents["codex"].last_sha,
        ).splitlines()
    )
    notes: list[str] = []
    if claude - codex:
        notes.append("Claude-only files: " + ", ".join(sorted(claude - codex)))
    if codex - claude:
        notes.append("Codex-only files: " + ", ".join(sorted(codex - claude)))
    return notes or ["Both branches changed the same file set."]


def _run_property_change(
    state: RunState,
    agent: AgentName,
    prompt: str,
    invoker: Invoker,
    executor: Executor,
) -> bool:
    agent_state = state.agents[agent]
    worktree = Path(agent_state.worktree)
    rejection: str | None = None
    for attempt in range(2):
        _reset_worktree(worktree, agent_state.last_sha)
        current_prompt = prompt
        if rejection is not None:
            current_prompt += (
                "\n\nYour previous property suite was rejected:\n"
                f"{rejection}\nStart again from the unchanged API stub."
            )
        invocation = invoker.invoke(
            agent,
            current_prompt,
            worktree,
            AGENT_TIMEOUT_SECONDS,
        )
        _record_usage(state, agent, invocation.tokens, invocation.cost_usd)
        if invocation.exit_code != 0:
            raise RuntimeError(
                f"{agent} property invocation failed; see "
                f"{invocation.transcript_path}"
            )
        if invocation.final_message.strip().startswith("SPEC BUG:"):
            state.spec_bug = invocation.final_message.strip()
            return False
        patch = Path(invocation.patch_path).read_text(encoding="utf-8")
        validation = validate_patch(patch)
        paths = patch_paths(patch)
        reasons = list(validation.reasons)
        if not paths or any(not path.startswith("tests/") for path in paths):
            reasons.append("property author may change only files under tests/")
        compiled = executor.run(["cargo", "test", "--no-run"], worktree)
        if compiled.returncode != 0:
            reasons.append(
                "property suite does not compile: "
                + _output(compiled.stdout, compiled.stderr)
            )
        if reasons:
            rejection = "; ".join(reasons)
            if attempt == 0:
                continue
            raise RuntimeError(f"{agent} property suite rejected twice: {rejection}")
        _uniform_commit(
            worktree,
            agent_state.last_sha,
            f"[orchestrator] properties {agent}",
        )
        agent_state.last_sha = _git(worktree, "rev-parse", "HEAD")
        return True
    raise AssertionError("unreachable retry loop")


def _install_property_tests(
    state: RunState,
    implementer: AgentName,
    property_author: AgentName,
) -> None:
    implementation = Path(state.agents[implementer].worktree)
    properties = Path(state.agents[property_author].worktree)
    _reset_worktree(implementation, state.agents[implementer].last_sha)
    names = [
        name
        for name in _git(properties, "ls-files", "tests").splitlines()
        if name
    ]
    if not names:
        raise RuntimeError("property author produced no tracked tests")
    state.property_test_paths = names
    for name in names:
        source = properties / name
        target = implementation / name
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.exists() and target.read_bytes() != source.read_bytes():
            raise RuntimeError(f"property test collides with implementer file {name}")
        shutil.copyfile(source, target)
    _uniform_commit(
        implementation,
        state.agents[implementer].last_sha,
        "[orchestrator] independent property suite",
    )
    state.agents[implementer].last_sha = _git(
        implementation, "rev-parse", "HEAD"
    )


def run_review_phase(
    state: RunState,
    invoker: Invoker,
    executor: Executor,
) -> None:
    match = re.fullmatch(r"REVIEW_ROUND_(\d+)", state.phase)
    if state.mode != "symmetric" or match is None:
        raise ValueError("review phase requires a symmetric REVIEW_ROUND state")
    round_number = int(match.group(1))
    phase_name = state.phase
    started_wall = _now()
    started = time.monotonic()
    findings_before = len(state.findings)
    review_root = (
        Path(next(iter(state.agents.values())).worktree).parent
        / "reviews"
        / f"round-{round_number}"
    )
    pending: list[
        tuple[AgentName, AgentName, str, Path, str]
    ] = []
    for review_index, (reviewer_name, target_name) in enumerate(
        (("claude", "codex"), ("codex", "claude")),
        start=1,
    ):
        reviewer = cast(AgentName, reviewer_name)
        target = cast(AgentName, target_name)
        step = f"{phase_name}:{reviewer}"
        if step in state.completed_steps:
            continue
        checkout = review_root / f"candidate-{review_index}"
        _export_tree(Path(state.repo), state.agents[target].last_sha, checkout)
        prompt = _render_prompt(state, "review", checkout, failure_summary="")
        pending.append((reviewer, target, step, checkout, prompt))
    invocations: dict[AgentName, Invocation] = {}
    errors: list[Exception] = []
    with _invocation_pool(invoker, max(1, len(pending))) as pool:
        futures = {
            reviewer: pool.submit(
                invoker.invoke,
                reviewer,
                prompt,
                checkout,
                AGENT_TIMEOUT_SECONDS,
            )
            for reviewer, _, _, checkout, prompt in pending
        }
        for reviewer, _, _, _, _ in pending:
            try:
                invocations[reviewer] = futures[reviewer].result()
            except Exception as error:
                errors.append(error)
    for reviewer, target, step, checkout, _ in pending:
        invocation = invocations.get(reviewer)
        if invocation is None:
            continue
        _record_usage(state, reviewer, invocation.tokens, invocation.cost_usd)
        if invocation.exit_code != 0:
            errors.append(
                RuntimeError(
                    f"{reviewer} review failed; see {invocation.transcript_path}"
                )
            )
            continue
        dirty = [
            line
            for line in _git(checkout, "status", "--porcelain").splitlines()
            if ".orchestrator-review/" not in line
        ]
        if dirty:
            _record_invalid_review(
                state,
                reviewer,
                target,
                "reviewer left edits outside `.orchestrator-review/`",
            )
            state.completed_steps.append(step)
            save_state(Path(state.run_dir) / "state.json", state)
            continue
        try:
            candidates = load_review_candidates(checkout)
        except (ReviewManifestError, json.JSONDecodeError) as error:
            _record_invalid_review(state, reviewer, target, str(error))
            state.completed_steps.append(step)
            save_state(Path(state.run_dir) / "state.json", state)
            continue
        for candidate in candidates:
            finding_id = f"F{len(state.findings) + 1}"
            relative_patch = Path("findings") / f"{finding_id}.patch"
            saved_patch = Path(state.run_dir) / relative_patch
            shutil.copyfile(candidate.patch, saved_patch)
            saved_candidate = ReviewCandidate(
                summary=candidate.summary,
                test_name=candidate.test_name,
                patch=saved_patch,
                real_user_path=candidate.real_user_path,
                impact=candidate.impact,
            )
            verification = verify_review_candidate(
                saved_candidate,
                Path(state.repo),
                state.agents[target].last_sha,
                review_root / "verification" / finding_id,
                executor,
            )
            state.findings.append(
                Finding(
                    id=finding_id,
                    author=reviewer,
                    against=target,
                    kind="defect" if verification.verified else "preference",
                    test_patch=str(relative_patch),
                    verified=verification.verified,
                    resolved=False,
                    summary=candidate.summary,
                    test_name=candidate.test_name,
                    real_user_path=candidate.real_user_path,
                    impact=candidate.impact,
                    observed=verification.observed or verification.reason,
                    patch_sha256=_sha256(saved_patch),
                    target_sha=state.agents[target].last_sha,
                )
            )
        state.completed_steps.append(step)
        save_state(Path(state.run_dir) / "state.json", state)
    if errors:
        raise errors[0]
    new_findings = state.findings[findings_before:]
    has_defects = any(
        finding.verified and not finding.resolved for finding in state.findings
    )
    state.rounds_completed = round_number
    state.phase = next_symmetric_phase(
        phase_name,
        state.max_fix_rounds,
        has_defects,
    )
    state.completed_steps.clear()
    state.history.append(
        PhaseHistory(
            phase=phase_name,
            started=started_wall,
            ended=_now(),
            ok=True,
            duration_seconds=time.monotonic() - started,
            detail=f"{sum(finding.verified for finding in new_findings)} verified",
        )
    )
    save_state(Path(state.run_dir) / "state.json", state)


def run_fix_phase(
    state: RunState,
    invoker: Invoker,
    executor: Executor,
) -> None:
    match = re.fullmatch(r"FIX_ROUND_(\d+)", state.phase)
    if state.mode != "symmetric" or match is None:
        raise ValueError("fix phase requires a symmetric FIX_ROUND state")
    round_number = int(match.group(1))
    phase_name = state.phase
    started_wall = _now()
    started = time.monotonic()
    pending: list[
        tuple[AgentName, str, list[Finding], Path, str, str]
    ] = []
    for agent in ("claude", "codex"):
        step = f"{phase_name}:{agent}"
        if step in state.completed_steps:
            continue
        findings = [
            finding
            for finding in state.findings
            if finding.against == agent
            and finding.verified
            and not finding.resolved
        ]
        if not findings:
            state.completed_steps.append(step)
            save_state(Path(state.run_dir) / "state.json", state)
            continue
        failure_summary = "\n\n".join(
            f"{finding.id}\n"
            f"Test: {finding.test_name}\n"
            f"Observed red failure:\n{finding.observed}"
            for finding in findings
        )
        worktree = Path(state.agents[agent].worktree)
        prompt = _render_prompt(
            state,
            "fix",
            worktree,
            failure_summary=failure_summary,
        )
        pending.append(
            (
                agent,
                step,
                findings,
                worktree,
                prompt,
                f"[orchestrator] fix round {round_number} {agent}",
            )
        )
    with _invocation_pool(invoker, max(1, len(pending))) as pool:
        futures = {
            agent: pool.submit(
                _run_fix_change,
                state,
                agent,
                findings,
                prompt,
                invoker,
                commit_message,
            )
            for agent, _, findings, _, prompt, commit_message in pending
        }
        errors: list[Exception] = []
        successful: set[AgentName] = set()
        for agent, _, _, _, _, _ in pending:
            try:
                futures[agent].result()
                successful.add(agent)
            except Exception as error:
                errors.append(error)
        for agent, step, findings, worktree, _, _ in pending:
            if agent not in successful:
                continue
            for finding in findings:
                result = executor.run(
                    [
                        "cargo",
                        "nextest",
                        "run",
                        "--filter-expr",
                        f"test(={finding.test_name})",
                    ],
                    worktree,
                )
                finding.resolved = result.returncode == 0
                if not finding.resolved:
                    finding.observed = _output(result.stdout, result.stderr)
            state.completed_steps.append(step)
            save_state(Path(state.run_dir) / "state.json", state)
        if errors:
            raise errors[0]
    state.rounds_completed = round_number
    state.phase = next_symmetric_phase(
        phase_name,
        state.max_fix_rounds,
        has_defects=False,
    )
    state.completed_steps.clear()
    unresolved = sum(
        finding.verified and not finding.resolved for finding in state.findings
    )
    state.history.append(
        PhaseHistory(
            phase=phase_name,
            started=started_wall,
            ended=_now(),
            ok=True,
            duration_seconds=time.monotonic() - started,
            detail=f"{unresolved} verified defects unresolved",
        )
    )
    save_state(Path(state.run_dir) / "state.json", state)


def _run_fix_change(
    state: RunState,
    agent: AgentName,
    findings: list[Finding],
    prompt: str,
    invoker: Invoker,
    commit_message: str,
) -> None:
    agent_state = state.agents[agent]
    worktree = Path(agent_state.worktree)
    rejection: str | None = None
    for attempt in range(2):
        _reset_worktree(worktree, agent_state.last_sha)
        test_paths: set[str] = set()
        for finding in findings:
            patch = Path(state.run_dir) / finding.test_patch
            patch_text = patch.read_text(encoding="utf-8")
            test_paths.update(patch_paths(patch_text))
            applied = subprocess.run(
                ["git", "apply", "--check", str(patch)],
                cwd=worktree,
                check=False,
                text=True,
                capture_output=True,
            )
            if applied.returncode != 0:
                raise RuntimeError(
                    f"{finding.id} no longer applies to {agent}: "
                    f"{applied.stderr.strip()}"
                )
            _git(worktree, "apply", str(patch))
        expected = {
            path: (worktree / path).read_bytes()
            for path in test_paths
            if (worktree / path).is_file()
        }
        current_prompt = prompt
        if rejection is not None:
            current_prompt += (
                "\n\nYour previous fix patch was rejected:\n"
                f"{rejection}\nStart again without changing the supplied tests."
            )
        invocation = invoker.invoke(
            agent,
            current_prompt,
            worktree,
            AGENT_TIMEOUT_SECONDS,
        )
        _record_usage(state, agent, invocation.tokens, invocation.cost_usd)
        if invocation.exit_code != 0:
            raise RuntimeError(
                f"{agent} fix failed; see {invocation.transcript_path}"
            )
        changed_tests = [
            path
            for path, content in expected.items()
            if not (worktree / path).is_file()
            or (worktree / path).read_bytes() != content
        ]
        validation = validate_patch(
            Path(invocation.patch_path).read_text(encoding="utf-8")
        )
        reasons = list(validation.reasons)
        if changed_tests:
            reasons.append(
                "supplied repro tests changed: " + ", ".join(changed_tests)
            )
        if reasons:
            rejection = "; ".join(reasons)
            if attempt == 0:
                continue
            raise RuntimeError(f"{agent} fix rejected twice: {rejection}")
        _uniform_commit(worktree, agent_state.last_sha, commit_message)
        agent_state.last_sha = _git(worktree, "rev-parse", "HEAD")
        return
    raise AssertionError("unreachable retry loop")


def verify_review_candidate(
    candidate: ReviewCandidate,
    repo: Path,
    target_sha: str,
    scratch: Path,
    executor: Executor,
) -> ReviewVerification:
    patch = candidate.patch.read_text(encoding="utf-8")
    paths = patch_paths(patch)
    if not paths or any(not path.startswith("tests/") for path in paths):
        return ReviewVerification(
            False,
            "",
            "the repro patch must change only files under tests/",
        )
    if scratch.exists():
        shutil.rmtree(scratch)
    _export_tree(repo, target_sha, scratch)
    existing_paths = [path for path in paths if (scratch / path).exists()]
    if existing_paths:
        return ReviewVerification(
            False,
            "",
            "the repro patch must add new test files; existing paths changed: "
            + ", ".join(existing_paths),
        )
    applied = subprocess.run(
        ["git", "apply", "--check", str(candidate.patch)],
        cwd=scratch,
        check=False,
        text=True,
        capture_output=True,
    )
    if applied.returncode != 0:
        return ReviewVerification(
            False,
            applied.stderr.strip(),
            "the repro patch does not apply cleanly",
        )
    _git(scratch, "apply", str(candidate.patch))
    compile_result = executor.run(
        ["cargo", "nextest", "run", "--no-run"],
        scratch,
    )
    if compile_result.returncode != 0:
        _reset_scratch(scratch)
        return ReviewVerification(
            False,
            _output(compile_result.stdout, compile_result.stderr),
            "the repro test does not compile",
        )
    red = executor.run(
        [
            "cargo",
            "nextest",
            "run",
            "--filter-expr",
            f"test(={candidate.test_name})",
        ],
        scratch,
    )
    _reset_scratch(scratch)
    baseline = executor.run(["cargo", "nextest", "run"], scratch)
    if red.returncode == 0:
        return ReviewVerification(
            False,
            _output(red.stdout, red.stderr),
            "the claimed regression test stayed green",
        )
    if baseline.returncode != 0:
        return ReviewVerification(
            False,
            _output(baseline.stdout, baseline.stderr),
            "the untouched reviewed baseline is not green",
        )
    return ReviewVerification(
        True,
        _output(red.stdout, red.stderr),
        "compiled, failed red, and reverted baseline passed",
    )


def _run_agent_change(
    state: RunState,
    agent: AgentName,
    prompt: str,
    invoker: Invoker,
    commit_message: str,
) -> list[str]:
    agent_state = state.agents[agent]
    worktree = Path(agent_state.worktree)
    rejection: str | None = None
    for attempt in range(2):
        _reset_worktree(worktree, agent_state.last_sha)
        current_prompt = prompt
        if rejection is not None:
            current_prompt += (
                "\n\nYour previous patch was rejected by the machine guard:\n"
                f"{rejection}\nStart again from the clean phase baseline."
            )
        invocation = invoker.invoke(
            agent,
            current_prompt,
            worktree,
            AGENT_TIMEOUT_SECONDS,
        )
        _record_usage(state, agent, invocation.tokens, invocation.cost_usd)
        if invocation.exit_code != 0:
            raise RuntimeError(
                f"{agent} invocation failed; see {invocation.transcript_path}"
            )
        patch = Path(invocation.patch_path).read_text(encoding="utf-8")
        validation = validate_patch(patch)
        if validation.rejected:
            rejection = "; ".join(validation.reasons)
            if attempt == 0:
                continue
            raise RuntimeError(f"{agent} patch rejected twice: {rejection}")
        _uniform_commit(worktree, agent_state.last_sha, commit_message)
        agent_state.last_sha = _git(worktree, "rev-parse", "HEAD")
        return list(validation.review_flags)
    raise AssertionError("unreachable retry loop")


@contextmanager
def _invocation_pool(
    invoker: Invoker,
    max_workers: int,
) -> Iterator[ThreadPoolExecutor]:
    pool = ThreadPoolExecutor(max_workers=max_workers)
    try:
        yield pool
    except BaseException:
        cancel = getattr(invoker, "cancel_all", None)
        if callable(cancel):
            cancel()
        pool.shutdown(wait=True, cancel_futures=True)
        raise
    else:
        pool.shutdown(wait=True)


def _uniform_commit(worktree: Path, last_sha: str, message: str) -> None:
    _git(worktree, "reset", "--soft", last_sha)
    _git(worktree, "add", "-A")
    status = _git(worktree, "status", "--porcelain")
    if not status:
        return
    _git(
        worktree,
        "-c",
        "user.name=orchestrator",
        "-c",
        "user.email=orchestrator@invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-m",
        message,
    )


def _reset_worktree(worktree: Path, sha: str) -> None:
    _git(worktree, "reset", "--hard", sha)
    _git(worktree, "clean", "-fdx")


def _render_prompt(
    state: RunState,
    name: str,
    worktree: Path,
    failure_summary: str,
    test_bug_claim: str = "",
) -> str:
    template = (_prompt_dir() / f"{name}.md").read_text(encoding="utf-8")
    plan_instruction = (
        f"Follow the frozen plan at `{Path(state.run_dir) / state.plan_path}`."
        if state.plan_path is not None
        else "No separate implementation plan was supplied."
    )
    return template.format(
        spec_path=Path(state.run_dir) / "spec.md",
        worktree_path=worktree,
        plan_instruction=plan_instruction,
        failure_summary=failure_summary,
        test_bug_claim=test_bug_claim,
    )


def _record_invalid_review(
    state: RunState,
    reviewer: AgentName,
    target: AgentName,
    reason: str,
) -> None:
    finding_id = f"F{len(state.findings) + 1}"
    state.findings.append(
        Finding(
            id=finding_id,
            author=reviewer,
            against=target,
            kind="preference",
            test_patch="",
            verified=False,
            resolved=False,
            summary="invalid review submission",
            observed=reason,
        )
    )


def _record_usage(
    state: RunState,
    agent: AgentName,
    tokens: int | None,
    cost: float | None,
) -> None:
    if tokens is not None:
        state.token_usage[agent] = state.token_usage.get(agent, 0) + tokens
    if cost is not None:
        state.costs_usd[agent] = state.costs_usd.get(agent, 0.0) + cost


def _export_tree(repo: Path, sha: str, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.is_symlink() or destination.is_file():
        destination.unlink()
    elif destination.exists():
        shutil.rmtree(destination)
    archive = destination.parent / f"{destination.name}.tar"
    result = subprocess.run(
        ["git", "archive", "--format=tar", "-o", str(archive), sha],
        cwd=repo,
        check=False,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "git archive failed")
    destination.mkdir()
    with tarfile.open(archive) as bundle:
        bundle.extractall(destination, filter="data")
    archive.unlink()
    _git(destination, "init", "-b", "review")
    _git(destination, "add", "-A")
    _git(
        destination,
        "-c",
        "user.name=orchestrator",
        "-c",
        "user.email=orchestrator@invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-m",
        "[orchestrator] neutral review target",
    )


def _reset_scratch(scratch: Path) -> None:
    _git(scratch, "reset", "--hard", "HEAD")
    _git(scratch, "clean", "-fdx")


def _output(stdout: str, stderr: str) -> str:
    combined = "\n".join(part.strip() for part in (stdout, stderr) if part.strip())
    return combined[-4_000:]


def _now() -> str:
    return datetime.now(UTC).isoformat().replace("+00:00", "Z")


def _create_target_worktree(
    repo: Path,
    worktree_root: Path,
    run_id: str,
    agent: AgentName,
    base_sha: str,
) -> AgentState:
    branch = f"agent/{agent}/{run_id}"
    worktree = worktree_root / agent
    _git(repo, "worktree", "add", "-b", branch, str(worktree), base_sha)
    return AgentState(str(worktree), branch, base_sha)


def _create_property_worktree(
    repo: Path,
    worktree_root: Path,
    run_id: str,
    agent: AgentName,
    api: str,
) -> AgentState:
    branch = f"agent/{agent}/{run_id}"
    worktree = worktree_root / agent
    worktree.mkdir()
    _git(worktree, "init", "-b", branch)
    package_name = _package_name(repo)
    (worktree / "src").mkdir()
    (worktree / "tests").mkdir()
    (worktree / "src/lib.rs").write_text(api, encoding="utf-8")
    (worktree / "Cargo.toml").write_text(
        "[package]\n"
        f'name = "{package_name}"\n'
        'version = "0.0.0"\n'
        'edition = "2024"\n\n'
        "[dev-dependencies]\n"
        'proptest = "1"\n',
        encoding="utf-8",
    )
    _git(worktree, "add", "Cargo.toml", "src/lib.rs")
    _git(
        worktree,
        "-c",
        "user.name=orchestrator",
        "-c",
        "user.email=orchestrator@invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-m",
        "[orchestrator] property API stub",
    )
    sha = _git(worktree, "rev-parse", "HEAD")
    return AgentState(str(worktree), branch, sha)


def _package_name(repo: Path) -> str:
    with (repo / "Cargo.toml").open("rb") as handle:
        manifest = cast(dict[str, object], tomllib.load(handle))
    package = manifest.get("package")
    if isinstance(package, dict):
        name = cast(dict[object, object], package).get("name")
        if isinstance(name, str):
            return name.replace("-", "_")
    return "target_under_test"


def _prompt_hashes() -> dict[str, str]:
    return {
        path.stem: _sha256(path)
        for path in sorted(_prompt_dir().glob("*.md"))
    }


def _prompt_dir() -> Path:
    packaged = Path(__file__).resolve().parent / "prompts"
    if packaged.is_dir():
        return packaged
    return Path(__file__).resolve().parents[2] / "prompts"


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _new_run_id(mode: Mode) -> str:
    stamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    return f"{stamp}-{mode}-{secrets.token_hex(3)}"


def _git(cwd: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=cwd,
        check=False,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout.strip()
