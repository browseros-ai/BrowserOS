#!/usr/bin/env python3
"""Tests for OTA bundle-zip creation."""

import stat
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

from .common import create_server_bundle_zip, find_server_resources_dir


def _write_exec(path: Path, content: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    path.chmod(path.stat().st_mode | 0o755)


class CreateServerBundleZipTest(unittest.TestCase):
    def test_bundles_full_resources_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            staging = Path(tmp) / "darwin-arm64"
            resources = staging / "resources"
            _write_exec(resources / "bin" / "browseros_server", b"server")
            _write_exec(resources / "bin" / "third_party" / "bun", b"bun")
            _write_exec(resources / "bin" / "third_party" / "rg", b"rg")
            _write_exec(resources / "bin" / "third_party" / "podman" / "podman", b"pd")
            _write_exec(
                resources / "bin" / "third_party" / "podman" / "gvproxy", b"gv"
            )

            zip_path = Path(tmp) / "bundle.zip"
            self.assertTrue(create_server_bundle_zip(resources, zip_path))

            with zipfile.ZipFile(zip_path) as zf:
                names = set(zf.namelist())

            self.assertEqual(
                names,
                {
                    "resources/bin/browseros_server",
                    "resources/bin/third_party/bun",
                    "resources/bin/third_party/rg",
                    "resources/bin/third_party/podman/podman",
                    "resources/bin/third_party/podman/gvproxy",
                },
            )

    @unittest.skipIf(sys.platform == "win32", "file mode check is meaningless on Windows")
    def test_preserves_executable_bits(self):
        with tempfile.TemporaryDirectory() as tmp:
            resources = Path(tmp) / "darwin-arm64" / "resources"
            _write_exec(resources / "bin" / "browseros_server", b"server")

            zip_path = Path(tmp) / "bundle.zip"
            self.assertTrue(create_server_bundle_zip(resources, zip_path))

            with zipfile.ZipFile(zip_path) as zf:
                info = zf.getinfo("resources/bin/browseros_server")

            mode = (info.external_attr >> 16) & 0o777
            self.assertTrue(mode & stat.S_IXUSR)

    def test_missing_resources_dir_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            missing = Path(tmp) / "does-not-exist"
            zip_path = Path(tmp) / "bundle.zip"
            self.assertFalse(create_server_bundle_zip(missing, zip_path))


class FindServerResourcesDirTest(unittest.TestCase):
    def test_returns_resources_dir_when_present(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "darwin-arm64" / "resources" / "bin").mkdir(parents=True)
            found = find_server_resources_dir(
                root, {"name": "darwin_arm64", "target": "darwin-arm64"}
            )
            self.assertEqual(found, root / "darwin-arm64" / "resources")

    def test_returns_none_when_absent(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.assertIsNone(
                find_server_resources_dir(
                    root, {"name": "darwin_arm64", "target": "darwin-arm64"}
                )
            )


if __name__ == "__main__":
    unittest.main()
