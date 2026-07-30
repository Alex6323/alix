from __future__ import annotations

import json
import os
from pathlib import Path
from typing import cast

from orchestrator.models import RunState


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
