#!/usr/bin/env python3
"""Tests for macOS app signing discovery."""

import tempfile
import unittest
from pathlib import Path

import yaml

from ...common.context import Context
from .macos import (
    SERVER_RESOURCES_SOURCE_REL,
    MacOSSignModule,
    find_components_to_sign,
    verify_server_resources_bundle,
)


def _write_exec(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\n")
    path.chmod(path.stat().st_mode | 0o755)


def _write_file(path: Path, content: str = "data\n") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


class MacOSSignDiscoveryTest(unittest.TestCase):
    def test_discovers_registered_server_binaries_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            app_path = Path(tmp) / "BrowserOS.app"
            server_bin = (
                app_path
                / "Contents"
                / "Resources"
                / "BrowserOSServer"
                / "default"
                / "resources"
                / "bin"
            )
            _write_exec(server_bin / "browseros_server")
            _write_exec(server_bin / "third_party" / "rg")
            _write_exec(server_bin / "third_party" / "codex")
            _write_exec(server_bin / "third_party" / "claude")
            _write_exec(server_bin / "third_party" / "lima" / "bin" / "limactl")

            executables = set(find_components_to_sign(app_path)["executables"])

            self.assertIn(server_bin / "browseros_server", executables)
            self.assertIn(server_bin / "third_party" / "rg", executables)
            self.assertIn(server_bin / "third_party" / "codex", executables)
            self.assertIn(server_bin / "third_party" / "claude", executables)
            self.assertNotIn(
                server_bin / "third_party" / "lima" / "bin" / "limactl",
                executables,
            )


class VerifyServerResourcesBundleTest(unittest.TestCase):
    def _setup(self, tmp: str) -> tuple[Path, Path, Path, Path]:
        chromium_src = Path(tmp) / "src"
        app_path = Path(tmp) / "out" / "BrowserOS.app"
        source_root = chromium_src / "chrome" / "browser" / "browseros" / "server" / "resources"
        bundle_root = (
            app_path
            / "Contents"
            / "Resources"
            / "BrowserOSServer"
            / "default"
            / "resources"
        )
        return chromium_src, app_path, source_root, bundle_root

    def test_reports_files_missing_from_bundle(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src, app_path, source_root, bundle_root = self._setup(tmp)
            _write_exec(source_root / "bin" / "browseros_server")
            _write_exec(source_root / "bin" / "third_party" / "codex")
            _write_exec(bundle_root / "bin" / "browseros_server")

            problems = verify_server_resources_bundle(app_path, chromium_src)

            self.assertEqual(len(problems), 1)
            self.assertIn("bin/third_party/codex", problems[0])

    def test_reports_lost_executable_bit(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src, app_path, source_root, bundle_root = self._setup(tmp)
            _write_exec(source_root / "bin" / "third_party" / "claude")
            _write_file(bundle_root / "bin" / "third_party" / "claude", "#!/bin/sh\n")

            problems = verify_server_resources_bundle(app_path, chromium_src)

            self.assertEqual(len(problems), 1)
            self.assertIn("bin/third_party/claude", problems[0])
            self.assertIn("executable", problems[0])

    def test_passes_when_bundle_matches_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src, app_path, source_root, bundle_root = self._setup(tmp)
            _write_exec(source_root / "bin" / "browseros_server")
            _write_exec(source_root / "bin" / "third_party" / "codex")
            _write_file(source_root / "db" / "migrations" / "0000_init.sql")
            _write_exec(bundle_root / "bin" / "browseros_server")
            _write_exec(bundle_root / "bin" / "third_party" / "codex")
            _write_file(bundle_root / "db" / "migrations" / "0000_init.sql")

            self.assertEqual(
                verify_server_resources_bundle(app_path, chromium_src), []
            )

    def test_skips_when_source_dir_absent(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src, app_path, _, bundle_root = self._setup(tmp)
            _write_exec(bundle_root / "bin" / "browseros_server")

            self.assertEqual(
                verify_server_resources_bundle(app_path, chromium_src), []
            )

    def test_bundle_only_extras_are_not_failures(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src, app_path, source_root, bundle_root = self._setup(tmp)
            _write_exec(source_root / "bin" / "browseros_server")
            _write_exec(bundle_root / "bin" / "browseros_server")
            _write_exec(bundle_root / "bin" / "third_party" / "lima" / "limactl")

            self.assertEqual(
                verify_server_resources_bundle(app_path, chromium_src), []
            )

    def test_junk_files_in_source_are_ignored(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src, app_path, source_root, bundle_root = self._setup(tmp)
            _write_exec(source_root / "bin" / "browseros_server")
            _write_file(source_root / "bin" / ".DS_Store", "junk")
            _write_exec(bundle_root / "bin" / "browseros_server")

            self.assertEqual(
                verify_server_resources_bundle(app_path, chromium_src), []
            )

    def test_source_rel_matches_copy_resources_destination(self):
        # The guard reads the staging dir that copy_resources.yaml writes; if
        # that destination moves, the guard must not silently degrade to the
        # skip branch.
        config_path = (
            Path(__file__).resolve().parents[2] / "config" / "copy_resources.yaml"
        )
        config = yaml.safe_load(config_path.read_text())
        destinations = {
            op["destination"]
            for op in config["copy_operations"]
            if op["name"].startswith("BrowserOS Server Resources")
        }

        self.assertEqual(destinations, {SERVER_RESOURCES_SOURCE_REL.as_posix()})


class SignModuleGuardWiringTest(unittest.TestCase):
    def test_module_guard_raises_on_stale_bundle(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src = Path(tmp) / "src"
            app_path = Path(tmp) / "out" / "BrowserOS.app"
            source_root = (
                chromium_src / "chrome" / "browser" / "browseros" / "server" / "resources"
            )
            _write_exec(source_root / "bin" / "third_party" / "codex")

            ctx = Context(
                chromium_src=chromium_src,
                architecture="arm64",
                build_type="release",
            )

            with self.assertRaises(RuntimeError) as raised:
                MacOSSignModule()._verify_server_resources(app_path, ctx)

            self.assertIn("bin/third_party/codex", str(raised.exception))

    def test_module_guard_accepts_matching_bundle(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src = Path(tmp) / "src"
            app_path = Path(tmp) / "out" / "BrowserOS.app"
            source_root = (
                chromium_src / "chrome" / "browser" / "browseros" / "server" / "resources"
            )
            bundle_root = (
                app_path
                / "Contents"
                / "Resources"
                / "BrowserOSServer"
                / "default"
                / "resources"
            )
            _write_exec(source_root / "bin" / "third_party" / "codex")
            _write_exec(bundle_root / "bin" / "third_party" / "codex")

            ctx = Context(
                chromium_src=chromium_src,
                architecture="arm64",
                build_type="release",
            )

            MacOSSignModule()._verify_server_resources(app_path, ctx)


if __name__ == "__main__":
    unittest.main()
