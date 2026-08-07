from __future__ import annotations

import argparse
import subprocess
import tempfile
from pathlib import Path

from orchestrator.cli import drive_run
from orchestrator.engine import RunOptions, initialize_run


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--live",
        action="store_true",
        help="spend real Claude and Codex calls on the smoke experiment",
    )
    args = parser.parse_args()
    if not args.live:
        print("live smoke skipped; pass --live to run real agents")
        return 0
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        repo = root / "target"
        repo.mkdir()
        _run(repo, "git", "init", "-b", "main")
        (repo / "src").mkdir()
        (repo / "src/lib.rs").write_text(
            "pub fn add(_a: i32, _b: i32) -> i32 { 0 }\n",
            encoding="utf-8",
        )
        (repo / "Cargo.toml").write_text(
            '[package]\nname = "add_smoke"\nversion = "0.1.0"\nedition = "2024"\n',
            encoding="utf-8",
        )
        (repo / "Makefile").write_text(
            "check:\n"
            "\tcargo fmt --check\n"
            "\tcargo clippy --all-targets -- -D warnings\n"
            "\tcargo nextest run\n\n"
            "gate: check\n"
            "\tcargo mutants --in-diff main --test-tool=nextest\n",
            encoding="utf-8",
        )
        _run(repo, "git", "add", "Cargo.toml", "Makefile", "src/lib.rs")
        _run(
            repo,
            "git",
            "-c",
            "user.name=orchestrator-smoke",
            "-c",
            "user.email=orchestrator-smoke@invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "smoke base",
        )
        spec = root / "spec.md"
        spec.write_text(
            """\
# Add two integers

Implement `pub fn add(a: i32, b: i32) -> i32` and exhaustive focused tests.

## API

```rust
pub fn add(a: i32, b: i32) -> i32 { a + b }
```
""",
            encoding="utf-8",
        )
        state = initialize_run(
            RunOptions(
                mode="symmetric",
                spec=spec,
                plan=None,
                repo=repo,
                base="main",
                run_root=root / "runs",
                max_fix_rounds=1,
                implementer="a",
                backends={"a": "claude", "b": "codex"},
            )
        )
        completed = drive_run(Path(state.run_dir))
        if completed.phase != "COMPLETE":
            raise RuntimeError(f"smoke stopped in {completed.phase}")
        _run(repo, "cargo", "test")
    print("live smoke passed")
    return 0


def _run(cwd: Path, *args: str) -> None:
    subprocess.run(args, cwd=cwd, check=True)


if __name__ == "__main__":
    raise SystemExit(main())
