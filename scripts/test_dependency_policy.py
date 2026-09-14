import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent
CHECK = ROOT / "scripts" / "check-dependency-policy.py"
FIXTURE = ROOT / "scripts" / "fixtures" / "dependency-policy" / "valid"


class DependencyPolicyTests(unittest.TestCase):
    def run_fixture(self, change=None, after_track=None):
        with tempfile.TemporaryDirectory() as raw:
            directory = pathlib.Path(raw) / "repo"
            shutil.copytree(FIXTURE, directory)
            for source in directory.rglob("*.fixture"):
                source.rename(source.with_suffix(""))
            if change is not None:
                change(directory)
            subprocess.run(
                ["git", "init", "--quiet"],
                cwd=directory,
                check=True,
                capture_output=True,
                text=True,
            )
            subprocess.run(
                ["git", "add", "."],
                cwd=directory,
                check=True,
                capture_output=True,
                text=True,
            )
            if after_track is not None:
                after_track(directory)
            return subprocess.run(
                [sys.executable, str(CHECK), "--root", str(directory)],
                cwd=directory,
                check=False,
                capture_output=True,
                text=True,
            )

    def assert_denied(self, change, line):
        result = self.run_fixture(change)
        self.assertEqual(1, result.returncode, result.stdout + result.stderr)
        self.assertEqual(f"dependency-policy: {line}\n", result.stderr)

    def test_the_complete_fixture_is_allowed(self):
        result = self.run_fixture()

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        self.assertEqual(
            "dependency-policy: 5 manifests and 4 lock roots match policy\n",
            result.stdout,
        )

    def test_an_ungoverned_manifest_is_denied(self):
        def change(directory):
            path = directory / "nested" / "package.json"
            path.parent.mkdir()
            path.write_text('{"type":"module"}\n', encoding="utf-8")

        self.assert_denied(
            change,
            "nested/package.json: manifest is not declared in "
            "scripts/dependency-policy.toml",
        )

    def test_a_cargo_git_dependency_without_an_exact_rev_is_denied(self):
        def change(directory):
            path = directory / "Cargo.toml"
            path.write_text(
                path.read_text(encoding="utf-8")
                + '\nremote_dep = { git = "https://example.com/remote", branch = "main" }\n',
                encoding="utf-8",
            )

        self.assert_denied(
            change,
            "Cargo.toml: package remote_dep: git dependency requires a 40-hex rev",
        )

    def test_a_cargo_git_patch_without_an_exact_rev_is_denied(self):
        def change(directory):
            path = directory / "Cargo.toml"
            path.write_text(
                path.read_text(encoding="utf-8")
                + '\n[patch.crates-io]\n'
                + 'remote_dep = { git = "https://example.com/remote", branch = "main" }\n',
                encoding="utf-8",
            )

        self.assert_denied(
            change,
            "Cargo.toml: package remote_dep: git dependency requires a 40-hex rev",
        )

    def test_a_cargo_git_replace_without_an_exact_rev_is_denied(self):
        def change(directory):
            path = directory / "Cargo.toml"
            path.write_text(
                path.read_text(encoding="utf-8")
                + '\n[replace]\n'
                + '"registry_dep:1.0.0" = { git = "https://example.com/remote", branch = "main" }\n',
                encoding="utf-8",
            )

        self.assert_denied(
            change,
            "Cargo.toml: package registry_dep:1.0.0: git dependency requires a 40-hex rev",
        )

    def test_an_untracked_root_cargo_config_cannot_supply_a_git_patch(self):
        def after_track(directory):
            path = directory / ".cargo" / "config.toml"
            path.parent.mkdir()
            path.write_text(
                '[patch.crates-io]\n'
                + 'remote_dep = { git = "https://example.com/remote", branch = "main" }\n',
                encoding="utf-8",
            )

        result = self.run_fixture(after_track=after_track)
        self.assertEqual(1, result.returncode, result.stdout + result.stderr)
        self.assertEqual(
            "dependency-policy: .cargo/config.toml: package remote_dep: "
            "git dependency requires a 40-hex rev\n",
            result.stderr,
        )

    def test_a_root_cargo_source_replacement_must_be_declared(self):
        def after_track(directory):
            path = directory / ".cargo" / "config.toml"
            path.parent.mkdir()
            path.write_text(
                '[source.crates-io]\nreplace-with = "mirror"\n',
                encoding="utf-8",
            )

        result = self.run_fixture(after_track=after_track)
        self.assertEqual(1, result.returncode, result.stdout + result.stderr)
        self.assertEqual(
            "dependency-policy: .cargo/config.toml: source replacement is not declared "
            "in scripts/dependency-policy.toml\n",
            result.stderr,
        )

    def test_declared_root_cargo_sources_are_allowed(self):
        def change(directory):
            policy = directory / "scripts" / "dependency-policy.toml"
            policy.write_text(
                policy.read_text(encoding="utf-8")
                + '\n[cargo]\nsources = ["crates-io", "mirror"]\n',
                encoding="utf-8",
            )

        def after_track(directory):
            path = directory / ".cargo" / "config.toml"
            path.parent.mkdir()
            path.write_text(
                '[source.crates-io]\nreplace-with = "mirror"\n'
                + '[source.mirror]\nregistry = "https://example.com/index"\n',
                encoding="utf-8",
            )

        result = self.run_fixture(change=change, after_track=after_track)
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        self.assertEqual(
            "dependency-policy: 5 manifests and 4 lock roots match policy\n",
            result.stdout,
        )

    def test_a_root_cargo_config_with_only_build_settings_is_allowed(self):
        def after_track(directory):
            path = directory / ".cargo" / "config.toml"
            path.parent.mkdir()
            path.write_text("[build]\njobs = 1\n", encoding="utf-8")

        result = self.run_fixture(after_track=after_track)
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        self.assertEqual(
            "dependency-policy: 5 manifests and 4 lock roots match policy\n",
            result.stdout,
        )

    def test_a_root_cargo_config_patch_path_is_resolved_from_the_checkout_root(self):
        def after_track(directory):
            path = directory / ".cargo" / "config.toml"
            path.parent.mkdir()
            path.write_text(
                '[patch.crates-io]\n'
                + 'remote_dep = { path = "../outside" }\n',
                encoding="utf-8",
            )

        result = self.run_fixture(after_track=after_track)
        self.assertEqual(1, result.returncode, result.stdout + result.stderr)
        self.assertEqual(
            "dependency-policy: .cargo/config.toml: package remote_dep: "
            "path dependency leaves repository\n",
            result.stderr,
        )

    def test_both_root_cargo_config_spellings_are_read(self):
        def after_track(directory):
            cargo = directory / ".cargo"
            cargo.mkdir()
            (cargo / "config.toml").write_text(
                "[build]\njobs = 1\n", encoding="utf-8"
            )
            (cargo / "config").write_text(
                '[patch.crates-io]\n'
                + 'remote_dep = { path = "../../../outside" }\n',
                encoding="utf-8",
            )

        result = self.run_fixture(after_track=after_track)
        self.assertEqual(1, result.returncode, result.stdout + result.stderr)
        self.assertEqual(
            "dependency-policy: .cargo/config: package remote_dep: "
            "path dependency leaves repository\n",
            result.stderr,
        )

    def test_a_cargo_path_dependency_outside_the_repository_is_denied(self):
        def change(directory):
            path = directory / "Cargo.toml"
            path.write_text(
                path.read_text(encoding="utf-8")
                + '\nlocal_dep = { path = "../outside" }\n',
                encoding="utf-8",
            )

        self.assert_denied(
            change,
            "Cargo.toml: package local_dep: path dependency leaves repository",
        )

    def test_a_cargo_registry_package_without_a_checksum_is_denied(self):
        def change(directory):
            path = directory / "Cargo.lock"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    'checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"\n',
                    "",
                ),
                encoding="utf-8",
            )

        self.assert_denied(
            change,
            "Cargo.lock: package registry_dep 1.0.0: registry package has no checksum",
        )

    def test_a_cargo_package_from_another_registry_is_denied(self):
        def change(directory):
            path = directory / "Cargo.lock"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    "https://github.com/rust-lang/crates.io-index",
                    "https://example.com/index",
                ),
                encoding="utf-8",
            )

        self.assert_denied(
            change,
            "Cargo.lock: package registry_dep 1.0.0: registry must be "
            "https://github.com/rust-lang/crates.io-index",
        )

    def test_an_npm_registry_package_without_integrity_is_denied(self):
        def change(directory):
            path = directory / "package-lock.json"
            data = json.loads(path.read_text(encoding="utf-8"))
            del data["packages"]["node_modules/dep"]["integrity"]
            path.write_text(json.dumps(data) + "\n", encoding="utf-8")

        self.assert_denied(
            change,
            "package-lock.json: package dep: registry package has no integrity",
        )

    def test_an_npm_package_from_another_registry_is_denied(self):
        def change(directory):
            path = directory / "package-lock.json"
            data = json.loads(path.read_text(encoding="utf-8"))
            data["packages"]["node_modules/dep"]["resolved"] = (
                "https://example.com/dep-1.0.0.tgz"
            )
            path.write_text(json.dumps(data) + "\n", encoding="utf-8")

        self.assert_denied(
            change,
            "package-lock.json: package dep: registry must be "
            "https://registry.npmjs.org",
        )

    def test_an_npm_link_outside_the_repository_is_denied(self):
        def change(directory):
            path = directory / "package-lock.json"
            data = json.loads(path.read_text(encoding="utf-8"))
            data["packages"]["node_modules/dep"] = {
                "resolved": "../outside",
                "link": True,
            }
            path.write_text(json.dumps(data) + "\n", encoding="utf-8")

        self.assert_denied(
            change,
            "package-lock.json: package dep: link dependency leaves repository",
        )

    def test_a_declared_dependency_free_manifest_with_dependencies_is_denied(self):
        def change(directory):
            path = directory / "web" / "package.json"
            data = json.loads(path.read_text(encoding="utf-8"))
            data["devDependencies"] = {"dep": "1.0.0"}
            path.write_text(json.dumps(data) + "\n", encoding="utf-8")

        self.assert_denied(
            change,
            "web/package.json: declared dependency-free but contains devDependencies",
        )

    def test_a_pub_hosted_package_without_a_checksum_is_denied(self):
        def change(directory):
            path = directory / "pubspec.lock"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    "      sha256: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
                    "",
                ),
                encoding="utf-8",
            )

        self.assert_denied(
            change,
            "pubspec.lock: package dep: hosted package has no sha256",
        )

    def test_a_pub_package_from_another_registry_is_denied(self):
        def change(directory):
            path = directory / "pubspec.lock"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    "https://pub.dev", "https://example.com"
                ),
                encoding="utf-8",
            )

        self.assert_denied(
            change,
            "pubspec.lock: package dep: registry must be https://pub.dev",
        )

    def test_a_uv_registry_artifact_without_a_hash_is_denied(self):
        def change(directory):
            path = directory / "uv.lock"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    "sha512:cccc",
                ),
                encoding="utf-8",
            )

        self.assert_denied(
            change,
            "uv.lock: package dep 1.0.0: registry artifact has no sha256 hash",
        )

    def test_a_uv_package_from_another_registry_is_denied(self):
        def change(directory):
            path = directory / "uv.lock"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    "https://pypi.org/simple", "https://example.com/simple"
                ),
                encoding="utf-8",
            )

        self.assert_denied(
            change,
            "uv.lock: package dep 1.0.0: registry must be https://pypi.org/simple",
        )


if __name__ == "__main__":
    unittest.main()
