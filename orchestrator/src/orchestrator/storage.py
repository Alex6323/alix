from __future__ import annotations

import json
import os
from pathlib import Path
import sys
import threading
from datetime import UTC, datetime
from typing import cast

from orchestrator.models import RunState


_PROGRESS_LOCK = threading.Lock()


def save_state(path: Path, state: RunState) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.tmp")
    with temporary.open("w", encoding="utf-8") as handle:
        json.dump(state.to_dict(), handle, indent=2, sort_keys=True)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)


def load_state(path: Path) -> RunState:
    with path.open(encoding="utf-8") as handle:
        payload = cast(object, json.load(handle))
    return RunState.from_dict(payload)


def append_progress(run_dir: Path, message: str) -> None:
    timestamp = datetime.now(UTC).isoformat().replace("+00:00", "Z")
    line = f"{timestamp} {' '.join(message.splitlines())}"
    with _PROGRESS_LOCK:
        with (run_dir / "progress.log").open("a", encoding="utf-8") as handle:
            handle.write(line + "\n")
            handle.flush()
            os.fsync(handle.fileno())
        print(line, file=sys.stderr, flush=True)
