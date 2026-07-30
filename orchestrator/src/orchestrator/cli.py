from __future__ import annotations

import argparse
import sys
from pathlib import Path

from orchestrator.agents import Invoker, SubprocessInvoker
from orchestrator.commands import Executor, SubprocessExecutor
from orchestrator.engine import (
    RunOptions,
    initialize_run,
    run_asymmetric_fix_phase,
    run_asymmetric_implementation_phase,
    run_asymmetric_test_phase,
    run_fix_phase,
    run_implementation_phase,
    run_land_phase,
    run_review_phase,
    run_score_phase,
)
from orchestrator.models import AgentName, Mode, PhaseHistory, RunState
from orchestrator.report import render_report
from orchestrator.storage import load_state, save_state


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="orchestrate")
    commands = parser.add_subparsers(dest="command", required=True)

    run = commands.add_parser("run", help="start and execute a frozen experiment")
    run.add_argument("--mode", choices=("symmetric", "asymmetric"), required=True)
    run.add_argument("--spec", type=Path, required=True)
    run.add_argument("--plan", type=Path)
    run.add_argument("--repo", type=Path, required=True)
    run.add_argument("--base", default="main")
    run.add_argument("--run-dir", type=Path)
    run.add_argument(
        "--max-fix-rounds",
        type=_nonnegative,
        default=2,
    )
    run.add_argument(
        "--implementer",
        choices=("claude", "codex"),
        default="claude",
        help="asymmetric implementer (default: claude)",
    )
    run.add_argument(
        "--claude-model",
        help="pin Claude Code's model for this run (default: the CLI's own)",
    )
    run.add_argument(
        "--codex-model",
        help="pin Codex's model for this run (default: the CLI's own)",
    )

    resume = commands.add_parser("resume", help="resume from state.json")
    resume.add_argument("--run-dir", type=Path, required=True)

    report = commands.add_parser("report", help="print the current report")
    report.add_argument("--run-dir", type=Path, required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        if args.command == "run":
            repo = args.repo.resolve()
            run_root = (
                args.run_dir
                if args.run_dir is not None
                else repo.parent / f"{repo.name}-orchestrator-runs"
            )
            state = initialize_run(
                RunOptions(
                    mode=args.mode,
                    spec=args.spec,
                    plan=args.plan,
                    repo=repo,
                    base=args.base,
                    run_root=run_root,
                    max_fix_rounds=args.max_fix_rounds,
                    implementer=args.implementer,
                    models={
                        agent: model
                        for agent, model in (
                            ("claude", args.claude_model),
                            ("codex", args.codex_model),
                        )
                        if model is not None
                    },
                )
            )
            print(state.run_dir)
            if state.phase != "COMPLETE":
                drive_run(Path(state.run_dir))
            return 0
        if args.command == "resume":
            drive_run(args.run_dir)
            return 0
        print_report(args.run_dir)
        return 0
    except (OSError, RuntimeError, ValueError) as error:
        print(f"orchestrate: {error}", file=sys.stderr)
        return 1


def drive_run(
    run_dir: Path,
    invoker: Invoker | None = None,
    executor: Executor | None = None,
) -> RunState:
    state_path = run_dir.resolve() / "state.json"
    state = load_state(state_path)
    active_invoker = invoker or SubprocessInvoker(
        Path(state.run_dir), models=state.models
    )
    active_executor = executor or SubprocessExecutor()
    while state.phase != "COMPLETE":
        phase = state.phase
        try:
            if phase == "IMPLEMENT":
                run_implementation_phase(state, active_invoker)
            elif phase == "IMPLEMENT_PROPERTIES":
                run_asymmetric_implementation_phase(
                    state,
                    active_invoker,
                    active_executor,
                )
            elif phase.startswith("REVIEW_ROUND_"):
                run_review_phase(state, active_invoker, active_executor)
            elif phase.startswith("FIX_ROUND_"):
                if state.mode == "symmetric":
                    run_fix_phase(state, active_invoker, active_executor)
                else:
                    run_asymmetric_fix_phase(state, active_invoker)
            elif phase == "RUN" or phase.startswith("RUN_ROUND_"):
                run_asymmetric_test_phase(
                    state,
                    active_executor,
                    active_invoker,
                )
            elif phase == "SCORE":
                run_score_phase(state, active_executor)
            elif phase == "LAND":
                run_land_phase(state)
            else:
                raise ValueError(f"unknown state phase {phase!r}")
        except Exception as error:
            state.history.append(
                PhaseHistory(
                    phase=phase,
                    started="interrupted",
                    ended="interrupted",
                    ok=False,
                    detail=str(error),
                )
            )
            save_state(state_path, state)
            raise
    return state


def print_report(run_dir: Path) -> None:
    run_dir = run_dir.resolve()
    report = run_dir / "report.md"
    if report.is_file():
        print(report.read_text(encoding="utf-8"), end="")
        return
    state = load_state(run_dir / "state.json")
    print(render_report(state, []), end="")


def _nonnegative(value: str) -> int:
    parsed = int(value)
    if parsed < 0:
        raise argparse.ArgumentTypeError("must be zero or greater")
    return parsed


if __name__ == "__main__":
    raise SystemExit(main())
