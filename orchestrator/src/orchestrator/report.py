from __future__ import annotations

from orchestrator.models import RunState
from orchestrator.scoring import BranchScore, recommend


def render_report(
    state: RunState,
    scores: list[BranchScore],
    divergence_notes: list[str] | None = None,
) -> str:
    winner = recommend(scores)
    lines = [
        f"# Orchestrator run {state.run_id}",
        "",
        f"- Mode: `{state.mode}`",
        f"- Base: `{state.base}` at `{state.base_sha}`",
        f"- Spec SHA-256: `{state.spec_hash}`",
        f"- Implementer: `{state.implementer}`",
        "",
        "## Phase durations",
        "",
        "| Phase | Seconds | Result | Detail |",
        "| --- | ---: | --- | --- |",
    ]
    for item in state.history:
        detail = (item.detail or "").replace("|", "\\|").replace("\n", "<br>")
        lines.append(
            f"| {item.phase} | {item.duration_seconds:.2f} | "
            f"{'ok' if item.ok else 'failed'} | {detail} |"
        )
    lines.extend(
        [
            "",
            "## Agent usage",
            "",
            "| Agent | Tokens | Cost |",
            "| --- | ---: | ---: |",
        ]
    )
    for agent in ("claude", "codex"):
        tokens = state.token_usage.get(agent)
        cost = state.costs_usd.get(agent)
        token_text = str(tokens) if tokens is not None else "not exposed"
        cost_text = f"${cost:.4f}" if cost is not None else "not exposed"
        lines.append(f"| {agent} | {token_text} | {cost_text} |")
    lines.extend(
        [
            "",
            "## Findings",
            "",
            "| ID | Author | Against | Kind | Verified | Resolved | Summary |",
            "| --- | --- | --- | --- | --- | --- | --- |",
        ]
    )
    if state.findings:
        for finding in state.findings:
            lines.append(
                f"| {finding.id} | {finding.author} | {finding.against} | "
                f"{finding.kind} | {finding.verified} | {finding.resolved} | "
                f"{finding.summary} |"
            )
    else:
        lines.append("| - | - | - | - | - | - | No findings |")
    for finding in state.findings:
        lines.extend(
            [
                "",
                f"### {finding.id}: {finding.summary}",
                "",
                f"- Review target: `{finding.target_sha or 'not supplied'}`",
                f"- Test: `{finding.test_name or 'not supplied'}`",
                f"- Patch SHA-256: `{finding.patch_sha256 or 'not supplied'}`",
                f"- Apply: `git apply {finding.test_patch or '<missing patch>'}`",
                "- Repro: "
                f"`cargo nextest run --filter-expr 'test(={finding.test_name})'`",
                f"- Real-user path: {finding.real_user_path or 'not supplied'}",
                f"- Impact: {finding.impact or 'not supplied'}",
                f"- Observed: {finding.observed or 'not verified'}",
            ]
        )
    lines.extend(
        [
            "",
            "## Scores",
            "",
            "| Agent | Cross tests | Mutants missed | Unresolved | Pedantic raw/added | LOC | Check | Eligibility | Penalty |",
            "| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | ---: |",
        ]
    )
    for score in scores:
        eligibility = (
            "eligible"
            if score.eligible
            else "ineligible: " + ", ".join(score.ineligible_reasons)
        )
        mutants = str(score.mutants_missed) if score.mutants_run else "skipped"
        lines.append(
            f"| {score.agent} | {score.cross_tests_passed}/{score.cross_tests_total} | "
            f"{mutants} | {score.unresolved_defects} | "
            f"{score.pedantic_warnings}/{score.pedantic_warnings_added} | "
            f"{score.diff_loc} | {'pass' if score.check_ok else 'fail'} | "
            f"{eligibility} | {score.penalty:.3f} |"
        )
    if winner is not None:
        recommendation = f"Merge `{winner}`."
    elif len(scores) > 1 and any(not score.eligible for score in scores):
        recommendation = "Incomplete eligible comparison: human decision required."
    elif any(score.eligible for score in scores):
        recommendation = "Eligible candidates tied: human decision required."
    else:
        recommendation = "No eligible candidate: human decision required."
    lines.extend(["", "## Merge recommendation", "", recommendation])
    if state.spec_bug is not None:
        lines.extend(["", "## Spec bug", "", state.spec_bug])
    lines.extend(["", "## Divergence notes", ""])
    notes = divergence_notes or ["No divergence notes recorded."]
    lines.extend(f"- {note}" for note in notes)
    return "\n".join(lines) + "\n"
