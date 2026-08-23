#!/usr/bin/env python3
"""Pin tools/gfm-harness/Cargo.lock to the root lock's versions.

The harness is a standalone workspace on purpose, so its lockfile can
drift from the production resolution whenever the root lock is
refreshed. The gate contract (test_gfm_gate.py) fails on any lean-tree
difference; this script is the remedy. It never resolves versions
itself: every change goes through `cargo update --precise`, so cargo
still enforces semver compatibility.
"""

import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HARNESS_MANIFEST = ROOT / "tools" / "gfm-harness" / "Cargo.toml"


def packages(lock_path):
    with open(lock_path, "rb") as handle:
        return tomllib.load(handle)["package"]


def sync_pass(root_versions):
    failures = []
    changes = 0
    for package in packages(ROOT / "tools" / "gfm-harness" / "Cargo.lock"):
        name, version = package["name"], package["version"]
        if name == "alix-gfm-corpus-harness" or "source" not in package:
            continue
        candidates = root_versions.get(name)
        if candidates is None or version in candidates:
            continue
        # Multi-major crates (getrandom) can have several root versions;
        # try the same compat bucket first or a wide requirement range can
        # merge the entry into the wrong major.
        def shared_prefix(candidate):
            shared = 0
            for a, b in zip(version.split("."), candidate.split(".")):
                if a != b:
                    break
                shared += 1
            return shared

        for candidate in sorted(candidates, key=lambda c: (shared_prefix(c), c), reverse=True):
            completed = subprocess.run(
                [
                    "cargo",
                    "update",
                    "--manifest-path",
                    str(HARNESS_MANIFEST),
                    "-p",
                    f"{name}@{version}",
                    "--precise",
                    candidate,
                ],
                capture_output=True,
                text=True,
            )
            if completed.returncode == 0:
                print(f"{name}: {version} -> {candidate}")
                changes += 1
                break
        else:
            failures.append(f"{name} {version} (root has {sorted(candidates)})")
    return changes, failures


def main():
    root_versions = {}
    for package in packages(ROOT / "Cargo.lock"):
        root_versions.setdefault(package["name"], set()).add(package["version"])

    # Exact-pinned sibling families (wasm-bindgen, futures, ratex) only
    # become movable once their parent moves, so iterate to a fixpoint.
    total = 0
    while True:
        changes, failures = sync_pass(root_versions)
        total += changes
        if changes == 0:
            break

    if failures:
        print("could not pin to any root version (incompatible requirement):")
        for failure in failures:
            print(f"  {failure}")
        return 1
    print(f"{total} package(s) repinned")
    return 0


if __name__ == "__main__":
    sys.exit(main())
