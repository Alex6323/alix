import contextlib
import hashlib
import http.server
import io
import os
import pathlib
import shutil
import stat
import subprocess
import tarfile
import tempfile
import threading
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent
INSTALLER = ROOT / "site" / "install.sh"
TAG = "v9.8.7"


class ReleaseHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.server.requests.append(self.path)
        if self.path == "/releases/latest":
            self.send_response(302)
            self.send_header("Location", f"{self.server.base_url}/releases/tag/{TAG}")
            self.end_headers()
            return
        if self.path == f"/releases/tag/{TAG}":
            self.send_response(200)
            self.end_headers()
            return

        body = self.server.assets.get(self.path)
        if body is None:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format, *_args):
        pass


@contextlib.contextmanager
def release_server(assets):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ReleaseHandler)
    server.assets = assets
    server.requests = []
    server.base_url = f"http://127.0.0.1:{server.server_port}"
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def archive_with_binary(body=b"#!/bin/sh\necho alix fixture\n"):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as archive:
        info = tarfile.TarInfo("alix")
        info.mode = 0o755
        info.size = len(body)
        archive.addfile(info, io.BytesIO(body))
    return output.getvalue(), body


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)

    def release_assets(self, target, archive_bytes, checksum=None):
        asset = f"alix-{target}.tar.gz"
        checksum_asset = f"alix-{target}.sha256"
        prefix = f"/releases/download/{TAG}"
        if checksum is None:
            digest = hashlib.sha256(archive_bytes).hexdigest()
            checksum = f"{digest}  {asset}\n".encode()
        return {
            f"{prefix}/{asset}": archive_bytes,
            f"{prefix}/{checksum_asset}": checksum,
        }

    def run_installer(self, assets, path=None, extra_env=None):
        bindir = self.root / "bin"
        with release_server(assets) as server:
            env = os.environ.copy()
            env.update(
                {
                    "ALIX_BIN_DIR": str(bindir),
                    "ALIX_RELEASE_BASE_URL": server.base_url,
                    "HOME": str(self.root),
                    "TMPDIR": str(self.root),
                }
            )
            if path is not None:
                env["PATH"] = str(path)
            if extra_env:
                env.update(extra_env)
            result = subprocess.run(
                ["/bin/sh", str(INSTALLER)],
                cwd=ROOT,
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )
            return result, bindir / "alix", server.requests

    def minimal_path(self, os_name, arch, include_shasum=False):
        directory = self.root / f"path-{os_name}-{arch}"
        directory.mkdir()
        for name in (
            "awk",
            "curl",
            "find",
            "grep",
            "gzip",
            "head",
            "install",
            "mkdir",
            "mktemp",
            "rm",
            "sed",
            "tar",
            "tr",
        ):
            source = pathlib.Path(shutil.which(name))
            (directory / name).symlink_to(source)

        uname = directory / "uname"
        uname.write_text(
            f"#!/bin/sh\n[ \"$1\" = -s ] && echo {os_name} || echo {arch}\n",
            encoding="utf-8",
        )
        uname.chmod(uname.stat().st_mode | stat.S_IXUSR)

        if include_shasum:
            shasum = directory / "shasum"
            shasum.write_text(
                "#!/bin/sh\n"
                "[ \"$1\" = -a ] && [ \"$2\" = 256 ] || exit 64\n"
                "shift 2\n"
                f"{shutil.which('sha256sum')} \"$@\"\n",
                encoding="utf-8",
            )
            shasum.chmod(shasum.stat().st_mode | stat.S_IXUSR)
        return directory

    def test_valid_linux_archive_is_verified_from_one_immutable_release(self):
        archive_bytes, binary = archive_with_binary()
        target = "x86_64-unknown-linux-gnu"

        result, installed, requests = self.run_installer(
            self.release_assets(target, archive_bytes)
        )

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        self.assertEqual(binary, installed.read_bytes())
        self.assertEqual(
            [
                "/releases/latest",
                f"/releases/tag/{TAG}",
                f"/releases/download/{TAG}/alix-{target}.tar.gz",
                f"/releases/download/{TAG}/alix-{target}.sha256",
            ],
            requests,
        )

    def test_corrupt_archive_is_not_installed(self):
        good_archive, _ = archive_with_binary()
        corrupt_archive, _ = archive_with_binary(b"corrupt but extractable\n")
        target = "x86_64-unknown-linux-gnu"
        digest = hashlib.sha256(good_archive).hexdigest()
        checksum = f"{digest}  alix-{target}.tar.gz\n".encode()

        result, installed, _ = self.run_installer(
            self.release_assets(target, corrupt_archive, checksum)
        )

        self.assertNotEqual(0, result.returncode)
        self.assertFalse(installed.exists())
        self.assertIn("checksum mismatch", result.stderr)

    def test_missing_checksum_is_not_installed(self):
        archive_bytes, _ = archive_with_binary()
        target = "x86_64-unknown-linux-gnu"
        assets = self.release_assets(target, archive_bytes)
        del assets[f"/releases/download/{TAG}/alix-{target}.sha256"]

        result, installed, _ = self.run_installer(assets)

        self.assertNotEqual(0, result.returncode)
        self.assertFalse(installed.exists())

    def test_checksum_for_a_different_asset_is_not_installed(self):
        archive_bytes, _ = archive_with_binary()
        target = "x86_64-unknown-linux-gnu"
        digest = hashlib.sha256(archive_bytes).hexdigest()
        checksum = f"{digest}  another-archive.tar.gz\n".encode()

        result, installed, _ = self.run_installer(
            self.release_assets(target, archive_bytes, checksum)
        )

        self.assertNotEqual(0, result.returncode)
        self.assertFalse(installed.exists())
        self.assertIn("does not name", result.stderr)

    def test_macos_falls_back_to_shasum_a_256(self):
        archive_bytes, binary = archive_with_binary()
        target = "aarch64-apple-darwin"
        path = self.minimal_path("Darwin", "arm64", include_shasum=True)

        result, installed, _ = self.run_installer(
            self.release_assets(target, archive_bytes), path=path
        )

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        self.assertEqual(binary, installed.read_bytes())

    def test_no_checksum_tool_fails_unless_explicitly_overridden(self):
        archive_bytes, binary = archive_with_binary()
        target = "x86_64-unknown-linux-gnu"
        path = self.minimal_path("Linux", "x86_64")
        assets = self.release_assets(target, archive_bytes)

        failed, installed, _ = self.run_installer(assets, path=path)
        self.assertNotEqual(0, failed.returncode)
        self.assertFalse(installed.exists())
        self.assertIn("ALIX_INSTALL_UNVERIFIED=1", failed.stderr)

        overridden, installed, _ = self.run_installer(
            assets,
            path=path,
            extra_env={"ALIX_INSTALL_UNVERIFIED": "1"},
        )
        self.assertEqual(
            0,
            overridden.returncode,
            overridden.stdout + overridden.stderr,
        )
        self.assertEqual(binary, installed.read_bytes())
        self.assertIn("UNVERIFIED", overridden.stderr)


if __name__ == "__main__":
    unittest.main()
