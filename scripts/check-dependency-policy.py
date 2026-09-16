#!/usr/bin/env python3

import argparse
import base64
import binascii
import json
import pathlib
import re
import subprocess
import sys
import tomllib
import urllib.parse


MANIFESTS = {"Cargo.toml", "package.json", "pubspec.yaml", "pyproject.toml"}
ROOT_CARGO_CONFIGS = (
    pathlib.PurePosixPath(".cargo/config.toml"),
    pathlib.PurePosixPath(".cargo/config"),
)
MAX_CARGO_CONFIG_INCLUDE_DEPTH = 32
DEPENDENCY_TABLES = {
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "dev_dependencies",
    "build_dependencies",
}
NPM_DEPENDENCY_TABLES = (
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
)
NPM_LOCK_VERSIONS = frozenset({2, 3})
HEX_40 = re.compile(r"[0-9a-f]{40}")
HEX_64 = re.compile(r"[0-9a-f]{64}")
INTEGRITY = re.compile(r"sha(256|384|512)-([A-Za-z0-9+/]+={0,2})")
DIGEST_BYTES = {"256": 32, "384": 48, "512": 64}


class PolicyError(Exception):
    pass


def load_toml(path):
    with path.open("rb") as source:
        return tomllib.load(source)


def display(path, root):
    return path.relative_to(root).as_posix()


def require_inside(root, path):
    try:
        path.resolve().relative_to(root.resolve())
    except ValueError:
        return False
    return True


def tracked_files(root):
    completed = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=root,
        check=False,
        capture_output=True,
    )
    if completed.returncode != 0:
        raise PolicyError("git ls-files failed")
    return {
        pathlib.PurePosixPath(raw.decode("utf-8"))
        for raw in completed.stdout.split(b"\0")
        if raw
    }


def dependency_tables(value):
    if not isinstance(value, dict):
        return
    for key, child in value.items():
        if key in DEPENDENCY_TABLES and isinstance(child, dict):
            yield child
        elif isinstance(child, dict):
            yield from dependency_tables(child)


def cargo_requirement_tables(data):
    yield from dependency_tables(data)
    patch = data.get("patch")
    if isinstance(patch, dict):
        for table in patch.values():
            if isinstance(table, dict):
                yield table
    if isinstance(data.get("replace"), dict):
        yield data["replace"]


def check_cargo_requirements(root, path, base, data):
    for table in cargo_requirement_tables(data):
        for name, requirement in sorted(table.items()):
            if not isinstance(requirement, dict):
                continue
            if "git" in requirement and not HEX_40.fullmatch(str(requirement.get("rev", ""))):
                raise PolicyError(
                    f"{display(path, root)}: package {name}: "
                    "git dependency requires a 40-hex rev"
                )
            if "path" in requirement:
                target = base / str(requirement["path"])
                if not require_inside(root, target):
                    raise PolicyError(
                        f"{display(path, root)}: package {name}: "
                        "path dependency leaves repository"
                    )


def check_cargo_manifest(root, manifest):
    check_cargo_requirements(root, manifest, manifest.parent, load_toml(manifest))


def cargo_config_includes(root, config, data):
    includes = data.get("include", [])
    if isinstance(includes, (str, dict)):
        includes = [includes]
    if not isinstance(includes, list):
        raise PolicyError(
            f"{display(config, root)}: included Cargo config path is invalid"
        )
    for include in includes:
        if isinstance(include, str):
            yield include
        elif isinstance(include, dict) and isinstance(include.get("path"), str):
            yield include["path"]
        else:
            raise PolicyError(
                f"{display(config, root)}: included Cargo config path is invalid"
            )


def check_cargo_config(
    root,
    config,
    declared_sources,
    policy_path,
    ancestors=(),
    depth=0,
):
    resolved = config.resolve()
    if resolved in ancestors:
        raise PolicyError(f"{display(config, root)}: included Cargo config cycle")
    if depth > MAX_CARGO_CONFIG_INCLUDE_DEPTH:
        raise PolicyError(
            f"{display(config, root)}: included Cargo config depth exceeds "
            f"{MAX_CARGO_CONFIG_INCLUDE_DEPTH}"
        )
    data = load_toml(config)
    next_ancestors = (*ancestors, resolved)
    for raw_include in cargo_config_includes(root, config, data):
        included = config.parent / raw_include
        if not require_inside(root, included):
            raise PolicyError(
                f"{display(config, root)}: included Cargo config leaves repository"
            )
        if not included.is_file():
            raise PolicyError(
                f"{display(config, root)}: included Cargo config is missing"
            )
        check_cargo_config(
            root,
            included,
            declared_sources,
            policy_path,
            next_ancestors,
            depth + 1,
        )

    base = config.parent.parent
    check_cargo_requirements(root, config, base, data)
    paths = data.get("paths", [])
    if not isinstance(paths, list) or any(not isinstance(path, str) for path in paths):
        raise PolicyError(f"{display(config, root)}: path override is invalid")
    if any(not require_inside(root, base / path) for path in paths):
        raise PolicyError(f"{display(config, root)}: path override leaves repository")
    sources = data.get("source")
    if not isinstance(sources, dict):
        return
    if any(name not in declared_sources for name in sources):
        raise PolicyError(
            f"{display(config, root)}: source replacement is not declared in "
            f"{display(policy_path, root)}"
        )


def check_cargo_lock(root, lock, registry):
    data = load_toml(lock)
    packages = data.get("package")
    if not packages:
        raise PolicyError(
            f"{display(lock, root)}: lockfile records no packages, "
            "which a declared dependency root cannot produce"
        )
    for package in packages:
        source = package.get("source")
        if source is None:
            continue
        name = package.get("name", "<unnamed>")
        version = package.get("version", "<unknown>")
        subject = f"{display(lock, root)}: package {name} {version}"
        if source.startswith("registry+"):
            if source != f"registry+{registry}":
                raise PolicyError(f"{subject}: registry must be {registry}")
            if not HEX_64.fullmatch(str(package.get("checksum", ""))):
                raise PolicyError(f"{subject}: registry package has no checksum")
        elif source.startswith("git+"):
            revision = source.rpartition("#")[2]
            if not HEX_40.fullmatch(revision):
                raise PolicyError(f"{subject}: git source has no exact revision")
        else:
            raise PolicyError(f"{subject}: source is not allowed")


def integrity_digest_matches(value):
    match = INTEGRITY.fullmatch(str(value))
    if match is None:
        return False
    try:
        digest = base64.b64decode(match.group(2), validate=True)
    except binascii.Error:
        return False
    return len(digest) == DIGEST_BYTES[match.group(1)]


def npm_package_name(path):
    return path.rsplit("node_modules/", 1)[-1]


def check_dependency_free_npm(root, manifest):
    data = json.loads(manifest.read_text(encoding="utf-8"))
    for table in NPM_DEPENDENCY_TABLES:
        if data.get(table):
            raise PolicyError(
                f"{display(manifest, root)}: declared dependency-free "
                f"but contains {table}"
            )


def check_npm_lock(root, lock, registry):
    data = json.loads(lock.read_text(encoding="utf-8"))
    version = data.get("lockfileVersion")
    if version not in NPM_LOCK_VERSIONS:
        raise PolicyError(
            f"{display(lock, root)}: lockfile version {version} "
            "is not a layout this check reads"
        )
    packages = data.get("packages")
    if not packages:
        raise PolicyError(
            f"{display(lock, root)}: no package entries were read; "
            "the lockfile layout is not the one this check understands"
        )
    for path, package in sorted(packages.items()):
        if not path:
            continue
        name = npm_package_name(path)
        subject = f"{display(lock, root)}: package {name}"
        if package.get("link"):
            target = lock.parent / str(package.get("resolved", ""))
            if not require_inside(root, target):
                raise PolicyError(f"{subject}: link dependency leaves repository")
            continue
        resolved = str(package.get("resolved", ""))
        parsed = urllib.parse.urlsplit(resolved)
        origin = f"{parsed.scheme}://{parsed.netloc}"
        if origin != registry:
            raise PolicyError(f"{subject}: registry must be {registry}")
        if not integrity_digest_matches(package.get("integrity", "")):
            raise PolicyError(f"{subject}: registry package has no integrity")


def yaml_scalar(value):
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
        return value[1:-1]
    return value


def pub_packages(lock):
    packages = {}
    current = None
    in_packages = False
    for line in lock.read_text(encoding="utf-8").splitlines():
        if line == "packages:":
            in_packages = True
            continue
        if not in_packages:
            continue
        if line and not line.startswith(" "):
            break
        match = re.fullmatch(r"  ([^ :][^:]*):", line)
        if match:
            current = {}
            packages[match.group(1)] = current
            continue
        if current is None:
            continue
        match = re.fullmatch(r"    (source|version): (.+)", line)
        if match:
            current[match.group(1)] = yaml_scalar(match.group(2))
            continue
        match = re.fullmatch(r"      (sha256|url|path|relative): (.+)", line)
        if match:
            current[match.group(1)] = yaml_scalar(match.group(2))
    return packages


def check_pub_lock(root, lock, registry):
    packages = pub_packages(lock)
    if not packages:
        raise PolicyError(
            f"{display(lock, root)}: no package entries were read; "
            "the lockfile layout is not the one this check understands"
        )
    for name, package in packages.items():
        subject = f"{display(lock, root)}: package {name}"
        source = package.get("source")
        if source == "hosted":
            if package.get("url") != registry:
                raise PolicyError(f"{subject}: registry must be {registry}")
            if not HEX_64.fullmatch(str(package.get("sha256", ""))):
                raise PolicyError(f"{subject}: hosted package has no sha256")
        elif source == "path":
            target = lock.parent / str(package.get("path", ""))
            if not require_inside(root, target):
                raise PolicyError(f"{subject}: path dependency leaves repository")
        elif source != "sdk":
            raise PolicyError(f"{subject}: source is not allowed")


def uv_artifacts(package):
    sdist = package.get("sdist")
    if isinstance(sdist, dict):
        yield sdist
    for wheel in package.get("wheels", []):
        if isinstance(wheel, dict):
            yield wheel


def check_uv_lock(root, lock, registry):
    data = load_toml(lock)
    packages = data.get("package")
    if not packages:
        raise PolicyError(
            f"{display(lock, root)}: lockfile records no packages, "
            "which a declared dependency root cannot produce"
        )
    for package in packages:
        name = package.get("name", "<unnamed>")
        version = package.get("version", "<unknown>")
        subject = f"{display(lock, root)}: package {name} {version}"
        source = package.get("source", {})
        if "registry" in source:
            if source["registry"] != registry:
                raise PolicyError(f"{subject}: registry must be {registry}")
            artifacts = list(uv_artifacts(package))
            if not artifacts:
                raise PolicyError(f"{subject}: registry package has no artifacts")
            for artifact in artifacts:
                digest = str(artifact.get("hash", ""))
                if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
                    raise PolicyError(f"{subject}: registry artifact has no sha256 hash")
        elif "editable" in source:
            target = lock.parent / str(source["editable"])
            if not require_inside(root, target):
                raise PolicyError(f"{subject}: editable dependency leaves repository")
        else:
            raise PolicyError(f"{subject}: source is not allowed")


def check_policy(root, policy_path):
    policy = load_toml(policy_path)
    if policy.get("version") != 1:
        raise PolicyError(f"{display(policy_path, root)}: version must be 1")
    projects = policy.get("projects", [])
    declared = {}
    for project in projects:
        manifest = pathlib.PurePosixPath(project["manifest"])
        if manifest in declared:
            raise PolicyError(f"{manifest}: manifest is declared more than once")
        declared[manifest] = project

    tracked = tracked_files(root)
    manifests = {
        path
        for path in tracked
        if path.name in MANIFESTS
    }
    for manifest in sorted(manifests - declared.keys()):
        raise PolicyError(
            f"{manifest}: manifest is not declared in {display(policy_path, root)}"
        )
    for manifest in sorted(declared.keys() - manifests):
        raise PolicyError(f"{manifest}: declared manifest is not tracked")

    declared_cargo_sources = set(policy.get("cargo", {}).get("sources", []))
    for config_path in ROOT_CARGO_CONFIGS:
        config = root / config_path
        if config.is_file():
            if config_path not in tracked:
                raise PolicyError(
                    f"{display(config, root)}: untracked Cargo config is refused; "
                    "track it or remove it"
                )
            check_cargo_config(root, config, declared_cargo_sources, policy_path)

    registries = policy.get("registries", {})
    locks = set()
    for manifest_path, project in declared.items():
        ecosystem = project["ecosystem"]
        manifest = root / manifest_path
        if project.get("dependency_free"):
            if ecosystem != "npm":
                raise PolicyError(f"{manifest_path}: dependency-free mode requires npm")
            check_dependency_free_npm(root, manifest)
            continue
        lock_path = pathlib.PurePosixPath(project["lock"])
        if lock_path not in tracked:
            raise PolicyError(f"{lock_path}: declared lock is not tracked")
        locks.add((ecosystem, lock_path))
        if ecosystem == "cargo":
            check_cargo_manifest(root, manifest)

    for ecosystem, lock_path in sorted(locks):
        lock = root / lock_path
        registry = registries[ecosystem]
        if ecosystem == "cargo":
            check_cargo_lock(root, lock, registry)
        elif ecosystem == "npm":
            check_npm_lock(root, lock, registry)
        elif ecosystem == "pub":
            check_pub_lock(root, lock, registry)
        elif ecosystem == "uv":
            check_uv_lock(root, lock, registry)
        else:
            raise PolicyError(f"{lock_path}: unknown ecosystem {ecosystem}")
    return len(manifests), len(locks)


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--policy", type=pathlib.Path)
    args = parser.parse_args(argv)
    root = args.root.resolve()
    policy_path = args.policy or root / "scripts" / "dependency-policy.toml"
    try:
        manifests, locks = check_policy(root, policy_path)
    except (KeyError, OSError, PolicyError, tomllib.TOMLDecodeError, json.JSONDecodeError) as error:
        print(f"dependency-policy: {error}", file=sys.stderr)
        return 1
    print(f"dependency-policy: {manifests} manifests and {locks} lock roots match policy")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
