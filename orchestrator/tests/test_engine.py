from __future__ import annotations

import json
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

from orchestrator.commands import CommandResult
from orchestrator.engine import (
    RunOptions,
    initialize_run,
    run_asymmetric_implementation_phase,
    run_asymmetric_fix_phase,
    run_asymmetric_test_phase,
    run_implementation_phase,
    run_land_phase,
    run_fix_phase,
    run_review_phase,
    run_score_phase,
    verify_review_candidate,
)
from orchestrator.models import Finding, Invocation
from orchestrator.review import ReviewCandidate
from orchestrator.storage import load_state


def git(cwd: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=cwd,
        check=True,
        text=True,
        capture_output=True,
    )
    return result.stdout.strip()


def target_repo(root: Path) -> Path:
    repo = root / "target"
    repo.mkdir()
    git(repo, "init", "-b", "main")
    git(repo, "config", "user.name", "Test")
    git(repo, "config", "user.email", "test@example.invalid")
    (repo / "Cargo.toml").write_text(
        '[package]\nname = "fixture"\nversion = "0.1.0"\nedition = "2024"\n'
    )
    (repo / "src").mkdir()
    (repo / "src/lib.rs").write_text("pub fn add(a: i32, b: i32) -> i32 { a + b }\n")
    (repo / "Makefile").write_text("check:\n\t@true\ngate:\n\t@true\n")
    git(repo, "add", "Cargo.toml", "src/lib.rs", "Makefile")
    git(repo, "-c", "commit.gpgsign=false", "commit", "-m", "base")
    return repo


class InitializeRunTests(unittest.TestCase):
    def test_symmetric_setup_freezes_input_and_creates_independent_worktrees(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            plan = root / "plan.md"
            spec.write_bytes(b"# Exact spec\n")
            plan.write_bytes(b"# Exact plan\n")

            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=plan,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=2,
                    implementer="claude",
                ),
                run_id="fixed-symmetric",
            )

            run_dir = Path(state.run_dir)
            self.assertEqual(b"# Exact spec\n", (run_dir / "spec.md").read_bytes())
            self.assertEqual(b"# Exact plan\n", (run_dir / "plan.md").read_bytes())
            self.assertEqual("IMPLEMENT", state.phase)
            self.assertEqual({"claude", "codex"}, set(state.agents))
            self.assertNotEqual(
                state.agents["claude"].worktree, state.agents["codex"].worktree
            )
            for name in ("claude", "codex"):
                worktree = Path(state.agents[name].worktree)
                self.assertTrue(worktree.is_dir())
                self.assertEqual(state.base_sha, git(worktree, "rev-parse", "HEAD"))
                self.assertEqual(
                    f"agent/{name}/fixed-symmetric",
                    state.agents[name].branch,
                )
            self.assertEqual(state, load_state(run_dir / "state.json"))
            self.assertTrue(state.prompt_hashes)

    def test_asymmetric_setup_uses_a_blank_stub_repo_for_the_property_author(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text(
                """\
# Add
## API
```rust
pub fn add(a: i32, b: i32) -> i32;
```
"""
            )

            state = initialize_run(
                RunOptions(
                    mode="asymmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=2,
                    implementer="claude",
                ),
                run_id="fixed-asymmetric",
            )

            property_worktree = Path(state.agents["codex"].worktree)
            self.assertFalse((property_worktree / "Makefile").exists())
            self.assertEqual(
                "pub fn add(a: i32, b: i32) -> i32;\n",
                (property_worktree / "src/lib.rs").read_text(),
            )
            self.assertIn("proptest", (property_worktree / "Cargo.toml").read_text())
            self.assertEqual("IMPLEMENT_PROPERTIES", state.phase)

    def test_asymmetric_setup_records_a_missing_api_contract_as_a_spec_bug(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Add\nNo API contract.\n")

            state = initialize_run(
                RunOptions(
                    mode="asymmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=2,
                    implementer="codex",
                ),
                run_id="spec-bug",
            )

            self.assertEqual("COMPLETE", state.phase)
            self.assertIn("## API", state.spec_bug or "")
            self.assertTrue((Path(state.run_dir) / "report.md").is_file())


class FakeInvoker:
    def __init__(self, run_dir: Path) -> None:
        self.run_dir = run_dir
        self.calls: list[str] = []
        self.claude_attempts = 0
        self.lock = threading.Lock()

    def invoke(
        self, agent: str, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        del prompt, timeout
        with self.lock:
            self.calls.append(agent)
            sequence = len(self.calls)
            if agent == "claude":
                self.claude_attempts += 1
                claude_attempt = self.claude_attempts
            else:
                claude_attempt = 0
        if agent == "claude":
            if claude_attempt == 1:
                (cwd / "Makefile").write_text("gate:\n\t@true\n")
            else:
                (cwd / "src/claude.rs").write_text("pub fn implementation() {}\n")
        else:
            (cwd / "src/codex.rs").write_text("pub fn implementation() {}\n")
        git(cwd, "add", "-N", ".")
        patch = self.run_dir / "patches" / f"{sequence:03d}.patch"
        patch.write_text(git(cwd, "diff", "--binary", "HEAD") + "\n")
        transcript = (
            self.run_dir / "transcripts" / f"{sequence:03d}-{agent}.txt"
        )
        transcript.write_text("done\n")
        return Invocation(
            exit_code=0,
            transcript_path=str(transcript),
            patch_path=str(patch),
            final_message="done",
            duration_seconds=0.1,
        )


class BarrierInvoker:
    def __init__(self, run_dir: Path) -> None:
        self.run_dir = run_dir
        self.barrier = threading.Barrier(2)
        self.calls: list[str] = []
        self.lock = threading.Lock()

    def invoke(
        self, agent: str, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        del prompt, timeout
        with self.lock:
            self.calls.append(agent)
        self.barrier.wait(timeout=1)
        (cwd / f"src/{agent}.rs").write_text("pub fn implementation() {}\n")
        git(cwd, "add", "-N", ".")
        patch = self.run_dir / "patches" / f"parallel-{agent}.patch"
        patch.write_text(git(cwd, "diff", "--binary", "HEAD") + "\n")
        transcript = self.run_dir / "transcripts" / f"parallel-{agent}.txt"
        transcript.write_text("done\n")
        return Invocation(
            exit_code=0,
            transcript_path=str(transcript),
            patch_path=str(patch),
            final_message="done",
            duration_seconds=0.1,
        )


class InterruptingInvoker:
    def __init__(self) -> None:
        self.barrier = threading.Barrier(2)
        self.cancelled = threading.Event()

    def invoke(
        self, agent: str, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        del prompt, cwd, timeout
        self.barrier.wait(timeout=1)
        if agent == "claude":
            raise KeyboardInterrupt
        if not self.cancelled.wait(timeout=1):
            raise RuntimeError("the peer invocation was not cancelled")
        raise RuntimeError("cancelled")

    def cancel_all(self) -> None:
        self.cancelled.set()


class InterruptedInvoker(FakeInvoker):
    def invoke(
        self, agent: str, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        if agent == "codex":
            self.calls.append(agent)
            transcript = self.run_dir / "transcripts" / "interrupted-codex.txt"
            transcript.write_text("interrupted\n")
            patch = self.run_dir / "patches" / "interrupted-codex.patch"
            patch.write_text("")
            return Invocation(
                exit_code=1,
                transcript_path=str(transcript),
                patch_path=str(patch),
                final_message="interrupted",
                duration_seconds=0.1,
            )
        return super().invoke(agent, prompt, cwd, timeout)


class ResumeCodexInvoker:
    def __init__(self, run_dir: Path) -> None:
        self.run_dir = run_dir
        self.calls: list[str] = []

    def invoke(
        self, agent: str, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        del prompt, timeout
        if agent != "codex":
            raise AssertionError("completed Claude step must not be rerun")
        self.calls.append(agent)
        (cwd / "src/codex.rs").write_text("pub fn resumed() {}\n")
        git(cwd, "add", "-N", ".")
        patch = self.run_dir / "patches" / "resumed-codex.patch"
        patch.write_text(git(cwd, "diff", "--binary", "HEAD") + "\n")
        transcript = self.run_dir / "transcripts" / "resumed-codex.txt"
        transcript.write_text("done\n")
        return Invocation(
            exit_code=0,
            transcript_path=str(transcript),
            patch_path=str(patch),
            final_message="done",
            duration_seconds=0.1,
        )


class FakeExecutor:
    def __init__(self) -> None:
        self.calls: list[tuple[str, ...]] = []

    def run(
        self, args: list[str], cwd: Path, timeout: float | None = None
    ) -> CommandResult:
        del cwd, timeout
        self.calls.append(tuple(args))
        if "--no-run" in args:
            return CommandResult(0, "compiled", "", 0.1)
        if "--filter-expr" in args:
            return CommandResult(101, "", "assertion failed: lost update", 0.1)
        return CommandResult(0, "baseline passed", "", 0.1)


class FakeReviewInvoker:
    def __init__(self, run_dir: Path) -> None:
        self.run_dir = run_dir
        self.calls = 0
        self.lock = threading.Lock()

    def invoke(
        self, agent: str, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        del prompt, timeout
        with self.lock:
            self.calls += 1
            sequence = self.calls
        output = cwd / ".orchestrator-review"
        output.mkdir()
        patch = output / "repro.patch"
        test_name = f"{agent}_repro"
        patch.write_text(
            "diff --git a/tests/repro.rs b/tests/repro.rs\n"
            "new file mode 100644\n"
            "--- /dev/null\n"
            "+++ b/tests/repro.rs\n"
            "@@ -0,0 +1,2 @@\n"
            "+#[test]\n"
            f'+fn {test_name}() {{ panic!("red"); }}\n'
        )
        (output / "findings.json").write_text(
            "[{"
            f'"summary":"{agent} found a lost update",'
            f'"test_name":"{test_name}",'
            '"test_patch":"repro.patch",'
            '"real_user_path":"Open two supported browser tabs and submit both.",'
            '"impact":"One successful update disappears from the saved state."'
            "}]\n"
        )
        transcript = (
            self.run_dir / "transcripts" / f"review-{sequence}-{agent}.txt"
        )
        transcript.write_text("review complete\n")
        snapshot = self.run_dir / "patches" / f"review-{sequence}-{agent}.patch"
        snapshot.write_text("")
        return Invocation(
            exit_code=0,
            transcript_path=str(transcript),
            patch_path=str(snapshot),
            final_message="review complete",
            duration_seconds=0.1,
        )


class BarrierReviewInvoker:
    def __init__(self, run_dir: Path) -> None:
        self.run_dir = run_dir
        self.barrier = threading.Barrier(2)

    def invoke(
        self, agent: str, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        del prompt, timeout
        self.barrier.wait(timeout=1)
        output = cwd / ".orchestrator-review"
        output.mkdir()
        (output / "findings.json").write_text("[]\n")
        transcript = self.run_dir / "transcripts" / f"parallel-review-{agent}.txt"
        transcript.write_text("done\n")
        patch = self.run_dir / "patches" / f"parallel-review-{agent}.patch"
        patch.write_text("")
        return Invocation(
            exit_code=0,
            transcript_path=str(transcript),
            patch_path=str(patch),
            final_message="done",
            duration_seconds=0.1,
        )


class InterruptedReviewInvoker:
    def __init__(self, run_dir: Path) -> None:
        self.run_dir = run_dir

    def invoke(
        self, agent: str, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        del prompt, cwd, timeout
        transcript = self.run_dir / "transcripts" / f"interrupted-{agent}.txt"
        transcript.write_text("interrupted\n")
        patch = self.run_dir / "patches" / f"interrupted-{agent}.patch"
        patch.write_text("")
        return Invocation(
            exit_code=1,
            transcript_path=str(transcript),
            patch_path=str(patch),
            final_message="interrupted",
            duration_seconds=0.1,
        )


class FakeFixInvoker:
    def __init__(self, run_dir: Path) -> None:
        self.run_dir = run_dir
        self.calls = 0
        self.prompts: list[str] = []
        self.lock = threading.Lock()

    def invoke(
        self, agent: str, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        del timeout
        with self.lock:
            self.calls += 1
            sequence = self.calls
            self.prompts.append(prompt)
        self.asserted_test = (cwd / "tests/repro.rs").read_text()
        (cwd / f"src/{agent}_fix.rs").write_text("pub fn fixed() {}\n")
        git(cwd, "add", "-N", ".")
        patch = self.run_dir / "patches" / f"fix-{sequence}-{agent}.patch"
        patch.write_text(git(cwd, "diff", "--binary", "HEAD") + "\n")
        transcript = (
            self.run_dir / "transcripts" / f"fix-{sequence}-{agent}.txt"
        )
        transcript.write_text("fixed\n")
        return Invocation(
            exit_code=0,
            transcript_path=str(transcript),
            patch_path=str(patch),
            final_message="fixed",
            duration_seconds=0.1,
        )


class GreenExecutor:
    def run(
        self, args: list[str], cwd: Path, timeout: float | None = None
    ) -> CommandResult:
        del args, cwd, timeout
        return CommandResult(0, "passed", "", 0.1)


class FailingPropertyExecutor:
    def run(
        self, args: list[str], cwd: Path, timeout: float | None = None
    ) -> CommandResult:
        del cwd, timeout
        if args == ["cargo", "nextest", "run"]:
            return CommandResult(101, "", "property failed", 0.1)
        return CommandResult(0, "compiled", "", 0.1)


class FakeScoreExecutor:
    def run(
        self, args: list[str], cwd: Path, timeout: float | None = None
    ) -> CommandResult:
        del timeout
        if args == ["make", "mutants"]:
            return CommandResult(0, "mutants: 0 missed", "", 0.1)
        if args[:2] == ["cargo", "clippy"]:
            warning = "warning: pedantic example\n" if cwd.name == "codex" else ""
            return CommandResult(0, "", warning, 0.1)
        return CommandResult(0, "passed", "", 0.1)


class SerializedGateExecutor(FakeScoreExecutor):
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.active_gates = 0
        self.max_active_gates = 0
        self.check_calls = 0

    def run(
        self, args: list[str], cwd: Path, timeout: float | None = None
    ) -> CommandResult:
        if args == ["make", "check"]:
            self.check_calls += 1
        if args != ["make", "mutants"]:
            return super().run(args, cwd, timeout)
        with self.lock:
            self.active_gates += 1
            self.max_active_gates = max(
                self.max_active_gates,
                self.active_gates,
            )
        try:
            return super().run(args, cwd, timeout)
        finally:
            with self.lock:
                self.active_gates -= 1


class FakeAsymmetricInvoker:
    def __init__(self, run_dir: Path, implementer: str) -> None:
        self.run_dir = run_dir
        self.implementer = implementer
        self.prompts: dict[str, str] = {}

    def invoke(
        self, agent: str, prompt: str, cwd: Path, timeout: float
    ) -> Invocation:
        del timeout
        self.prompts[agent] = prompt
        if agent == self.implementer:
            (cwd / "src/implementation.rs").write_text("pub fn fixed() {}\n")
        else:
            (cwd / "tests").mkdir(exist_ok=True)
            (cwd / "tests/property.rs").write_text(
                "#[test]\nfn addition_is_commutative() {}\n"
            )
        git(cwd, "add", "-N", ".")
        patch = self.run_dir / "patches" / f"asym-{agent}.patch"
        patch.write_text(git(cwd, "diff", "--binary", "HEAD") + "\n")
        transcript = self.run_dir / "transcripts" / f"asym-{agent}.txt"
        transcript.write_text("done\n")
        return Invocation(
            exit_code=0,
            transcript_path=str(transcript),
            patch_path=str(patch),
            final_message="done",
            duration_seconds=0.1,
        )


class PhaseExecutionTests(unittest.TestCase):
    def test_interrupting_a_parallel_phase_cancels_the_peer_invocation(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Implement\n")
            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=0,
                    implementer="claude",
                ),
                run_id="interrupt-parallel",
            )
            invoker = InterruptingInvoker()

            with self.assertRaises(KeyboardInterrupt):
                run_implementation_phase(state, invoker)

            self.assertTrue(invoker.cancelled.is_set())

    def test_symmetric_implementers_run_concurrently(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Implement\n")
            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=0,
                    implementer="claude",
                ),
                run_id="parallel-implementation",
            )
            invoker = BarrierInvoker(Path(state.run_dir))

            run_implementation_phase(state, invoker)

            self.assertCountEqual(["claude", "codex"], invoker.calls)
            self.assertEqual("SCORE", state.phase)

    def test_implementation_rejects_once_then_reprompts_and_commits_uniformly(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Implement\n")
            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=2,
                    implementer="claude",
                ),
                run_id="phase",
            )
            invoker = FakeInvoker(Path(state.run_dir))

            run_implementation_phase(state, invoker)

            self.assertEqual(2, invoker.calls.count("claude"))
            self.assertEqual(1, invoker.calls.count("codex"))
            self.assertEqual("REVIEW_ROUND_1", state.phase)
            for name in ("claude", "codex"):
                worktree = Path(state.agents[name].worktree)
                self.assertEqual(
                    f"[orchestrator] implement {name}",
                    git(worktree, "log", "-1", "--format=%s"),
                )
                self.assertEqual(
                    git(worktree, "rev-parse", "HEAD"),
                    state.agents[name].last_sha,
                )
            self.assertEqual(
                state,
                load_state(Path(state.run_dir) / "state.json"),
            )

    def test_resume_skips_an_agent_whose_step_was_committed_before_interruption(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Implement\n")
            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=2,
                    implementer="claude",
                ),
                run_id="resume",
            )

            with self.assertRaisesRegex(RuntimeError, "codex invocation failed"):
                run_implementation_phase(
                    state,
                    InterruptedInvoker(Path(state.run_dir)),
                )
            claude_sha = state.agents["claude"].last_sha
            persisted = load_state(Path(state.run_dir) / "state.json")
            resumed = ResumeCodexInvoker(Path(state.run_dir))

            run_implementation_phase(persisted, resumed)

            self.assertEqual(["codex"], resumed.calls)
            self.assertEqual(claude_sha, persisted.agents["claude"].last_sha)
            self.assertEqual("REVIEW_ROUND_1", persisted.phase)
            self.assertEqual([], persisted.completed_steps)

    def test_review_verification_requires_compile_red_and_green_baseline(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            patch = root / "F1.patch"
            patch.write_text(
                """\
diff --git a/tests/repro.rs b/tests/repro.rs
new file mode 100644
--- /dev/null
+++ b/tests/repro.rs
@@ -0,0 +1,2 @@
+#[test]
+fn two_tabs_keep_both_updates() { panic!("lost update"); }
"""
            )
            candidate = ReviewCandidate(
                summary="lost update",
                test_name="two_tabs_keep_both_updates",
                patch=patch,
                real_user_path="Open the supported client in two browser tabs.",
                impact="One successful selection disappears from recent history.",
            )
            executor = FakeExecutor()

            result = verify_review_candidate(
                candidate,
                repo,
                git(repo, "rev-parse", "HEAD"),
                root / "scratch",
                executor,
            )

            self.assertTrue(result.verified)
            self.assertIn("assertion failed", result.observed)
            self.assertEqual(
                [
                    ("cargo", "nextest", "run", "--no-run"),
                    (
                        "cargo",
                        "nextest",
                        "run",
                        "--filter-expr",
                        "test(=two_tabs_keep_both_updates)",
                    ),
                    ("cargo", "nextest", "run"),
                ],
                executor.calls,
            )

    def test_review_authors_run_concurrently_before_serial_verification(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Implement\n")
            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=1,
                    implementer="claude",
                ),
                run_id="parallel-review",
            )
            run_implementation_phase(state, FakeInvoker(Path(state.run_dir)))

            run_review_phase(
                state,
                BarrierReviewInvoker(Path(state.run_dir)),
                FakeExecutor(),
            )

            self.assertEqual("SCORE", state.phase)

    def test_review_verification_rejects_rewriting_an_existing_test(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            (repo / "tests").mkdir()
            (repo / "tests/existing.rs").write_text(
                "#[test]\nfn existing_behavior() {}\n"
            )
            git(repo, "add", "tests/existing.rs")
            git(
                repo,
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "existing test",
            )
            patch = root / "rewrite.patch"
            patch.write_text(
                """\
diff --git a/tests/existing.rs b/tests/existing.rs
index e2d68f0..1bbef69 100644
--- a/tests/existing.rs
+++ b/tests/existing.rs
@@ -1,2 +1,2 @@
 #[test]
-fn existing_behavior() {}
+fn existing_behavior() { panic!("manufactured red"); }
"""
            )
            candidate = ReviewCandidate(
                summary="manufactured failure",
                test_name="existing_behavior",
                patch=patch,
                real_user_path="Run the supported command.",
                impact="The command fails.",
            )

            result = verify_review_candidate(
                candidate,
                repo,
                git(repo, "rev-parse", "HEAD"),
                root / "scratch",
                FakeExecutor(),
            )

            self.assertFalse(result.verified)
            self.assertIn("new test files", result.reason)

    def test_review_round_records_only_mechanically_verified_user_defects(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Implement\n")
            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=2,
                    implementer="claude",
                ),
                run_id="review",
            )
            state.phase = "REVIEW_ROUND_1"
            invoker = FakeReviewInvoker(Path(state.run_dir))

            run_review_phase(state, invoker, FakeExecutor())

            self.assertEqual("FIX_ROUND_1", state.phase)
            self.assertEqual(2, len(state.findings))
            for finding in state.findings:
                self.assertEqual("defect", finding.kind)
                self.assertTrue(finding.verified)
                self.assertIn("two supported browser tabs", finding.real_user_path)
                self.assertIn("disappears", finding.impact)
                self.assertIn("assertion failed", finding.observed)
                self.assertTrue(
                    (Path(state.run_dir) / finding.test_patch).is_file()
                )
                self.assertTrue(finding.patch_sha256)

    def test_resume_replaces_an_interrupted_neutral_review_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Implement\n")
            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=1,
                    implementer="claude",
                ),
                run_id="review-resume",
            )
            state.phase = "REVIEW_ROUND_1"

            with self.assertRaisesRegex(RuntimeError, "review failed"):
                run_review_phase(
                    state,
                    InterruptedReviewInvoker(Path(state.run_dir)),
                    FakeExecutor(),
                )

            run_review_phase(
                state,
                FakeReviewInvoker(Path(state.run_dir)),
                FakeExecutor(),
            )

            self.assertEqual("FIX_ROUND_1", state.phase)
            self.assertEqual(2, len(state.findings))

    def test_fix_round_applies_verified_tests_verbatim_and_marks_them_resolved(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Implement\n")
            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=2,
                    implementer="claude",
                ),
                run_id="fix",
            )
            state.phase = "REVIEW_ROUND_1"
            run_review_phase(
                state,
                FakeReviewInvoker(Path(state.run_dir)),
                FakeExecutor(),
            )
            invoker = FakeFixInvoker(Path(state.run_dir))

            run_fix_phase(state, invoker, GreenExecutor())

            self.assertEqual("REVIEW_ROUND_2", state.phase)
            self.assertTrue(all(finding.resolved for finding in state.findings))
            self.assertTrue(
                all("two supported browser tabs" not in prompt for prompt in invoker.prompts)
            )
            self.assertTrue(
                all("Observed red failure" in prompt for prompt in invoker.prompts)
            )
            for name in ("claude", "codex"):
                worktree = Path(state.agents[name].worktree)
                self.assertIn(
                    f"fn {('codex' if name == 'claude' else 'claude')}_repro",
                    (worktree / "tests/repro.rs").read_text(),
                )
                self.assertEqual(
                    f"[orchestrator] fix round 1 {name}",
                    git(worktree, "log", "-1", "--format=%s"),
                )

    def test_asymmetric_property_author_never_sees_the_implementation_worktree(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text(
                """\
# Add
## API
```rust
pub fn add(a: i32, b: i32) -> i32 { a + b }
```
"""
            )
            state = initialize_run(
                RunOptions(
                    mode="asymmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=2,
                    implementer="claude",
                ),
                run_id="asymmetric-phase",
            )
            invoker = FakeAsymmetricInvoker(Path(state.run_dir), "claude")

            run_asymmetric_implementation_phase(state, invoker, GreenExecutor())

            self.assertEqual("RUN", state.phase)
            self.assertNotIn(
                state.agents["claude"].worktree,
                invoker.prompts["codex"],
            )
            self.assertIn(
                state.agents["codex"].worktree,
                invoker.prompts["codex"],
            )

            run_asymmetric_test_phase(state, GreenExecutor())

            self.assertEqual("SCORE", state.phase)
            implementation = Path(state.agents["claude"].worktree)
            self.assertEqual(
                "#[test]\nfn addition_is_commutative() {}\n",
                (implementation / "tests/property.rs").read_text(),
            )
            self.assertTrue(state.property_suite_passed)

    def test_asymmetric_fix_cannot_change_the_independent_property_suite(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text(
                """\
# Add
## API
```rust
pub fn add(a: i32, b: i32) -> i32 { a + b }
```
"""
            )
            state = initialize_run(
                RunOptions(
                    mode="asymmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=1,
                    implementer="claude",
                ),
                run_id="asymmetric-fix",
            )
            invoker = FakeAsymmetricInvoker(Path(state.run_dir), "claude")
            run_asymmetric_implementation_phase(state, invoker, GreenExecutor())
            run_asymmetric_test_phase(state, FailingPropertyExecutor())
            before = (
                Path(state.agents["claude"].worktree) / "tests/property.rs"
            ).read_bytes()

            run_asymmetric_fix_phase(state, invoker)
            run_asymmetric_test_phase(state, GreenExecutor())

            self.assertEqual("SCORE", state.phase)
            self.assertEqual(
                before,
                (
                    Path(state.agents["claude"].worktree)
                    / "tests/property.rs"
                ).read_bytes(),
            )

    def test_score_serializes_machine_metrics_and_recommends_the_sound_branch(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Implement\n")
            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=0,
                    implementer="claude",
                ),
                run_id="score",
            )
            run_implementation_phase(state, FakeInvoker(Path(state.run_dir)))
            state.phase = "SCORE"
            state.findings.append(
                Finding(
                    id="F1",
                    author="claude",
                    against="codex",
                    kind="defect",
                    test_patch="findings/F1.patch",
                    verified=True,
                    resolved=True,
                    summary="codex loses a user update",
                    test_name="update_is_retained",
                    real_user_path="Submit two supported updates.",
                    impact="One update disappears.",
                    observed="red",
                    patch_sha256="sha",
                )
            )

            executor = SerializedGateExecutor()
            run_score_phase(state, executor)

            scores = json.loads((Path(state.run_dir) / "scores.json").read_text())
            self.assertEqual("LAND", state.phase)
            self.assertEqual(2, len(scores))
            self.assertEqual(0, scores[0]["mutants_missed"])
            self.assertEqual(0, scores[0]["pedantic_warnings"])
            self.assertEqual(0, scores[0]["pedantic_warnings_added"])
            self.assertEqual(1, scores[1]["pedantic_warnings_added"])
            self.assertEqual(1, executor.max_active_gates)
            self.assertEqual(2, executor.check_calls)
            self.assertIn("Merge `claude`.", (Path(state.run_dir) / "report.md").read_text())

    def test_land_commits_union_tests_before_the_winning_implementation(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = target_repo(root)
            spec = root / "input.md"
            spec.write_text("# Implement\n")
            state = initialize_run(
                RunOptions(
                    mode="symmetric",
                    spec=spec,
                    plan=None,
                    repo=repo,
                    base="main",
                    run_root=root / "runs",
                    max_fix_rounds=0,
                    implementer="claude",
                ),
                run_id="land",
            )
            run_implementation_phase(state, FakeInvoker(Path(state.run_dir)))
            claude = Path(state.agents["claude"].worktree)
            (claude / "tests").mkdir()
            (claude / "tests/claude.rs").write_text("#[test]\nfn claude_case() {}\n")
            git(claude, "add", "tests/claude.rs")
            git(
                claude,
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "claude test",
            )
            state.agents["claude"].last_sha = git(claude, "rev-parse", "HEAD")
            state.phase = "SCORE"
            run_score_phase(state, FakeScoreExecutor())

            run_land_phase(state)

            self.assertEqual("COMPLETE", state.phase)
            self.assertTrue((repo / "tests/claude.rs").is_file())
            self.assertTrue((repo / "src/claude.rs").is_file())
            self.assertFalse((repo / "src/codex.rs").exists())
            self.assertEqual(
                [
                    "[orchestrator] land implementation claude",
                    "[orchestrator] land tests",
                ],
                git(repo, "log", "-2", "--format=%s").splitlines(),
            )
            landed_sha = git(repo, "rev-parse", "HEAD")

            # Simulate a crash after the fast-forward and before the atomic
            # state write. Resume must recognize its own landing commit.
            state.history.pop()
            state.phase = "LAND"
            run_land_phase(state)

            self.assertEqual("COMPLETE", state.phase)
            self.assertEqual(landed_sha, git(repo, "rev-parse", "HEAD"))
            self.assertIn(
                "recognized already-landed",
                state.history[-1].detail or "",
            )
