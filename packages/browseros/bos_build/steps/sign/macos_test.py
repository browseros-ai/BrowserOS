#!/usr/bin/env python3
"""Tests for macOS app signing discovery."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import yaml

from ...core.context import Context
from . import macos as macos_module
from .macos import (
    SERVER_RESOURCES_SOURCE_REL,
    MacOSSignModule,
    find_components_to_sign,
    sign_component,
    verify_server_resources_bundle,
    verify_signature,
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
            claw_bin = (
                app_path
                / "Contents"
                / "Resources"
                / "BrowserClawServer"
                / "default"
                / "resources"
                / "bin"
            )
            _write_exec(claw_bin / "browseros-claw-server")
            _write_exec(claw_bin / "not-registered")

            executables = set(find_components_to_sign(app_path)["executables"])

            self.assertIn(server_bin / "browseros_server", executables)
            self.assertIn(server_bin / "third_party" / "rg", executables)
            self.assertIn(claw_bin / "browseros-claw-server", executables)
            self.assertNotIn(server_bin / "third_party" / "codex", executables)
            self.assertNotIn(server_bin / "third_party" / "claude", executables)
            self.assertNotIn(
                server_bin / "third_party" / "lima" / "bin" / "limactl",
                executables,
            )
            self.assertNotIn(claw_bin / "not-registered", executables)


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
            _write_exec(source_root / "bin" / "third_party" / "rg")
            _write_exec(bundle_root / "bin" / "browseros_server")

            problems = verify_server_resources_bundle(app_path, chromium_src)

            self.assertEqual(len(problems), 1)
            self.assertIn("bin/third_party/rg", problems[0])

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
            _write_exec(source_root / "bin" / "third_party" / "rg")
            _write_file(source_root / "db" / "migrations" / "0000_init.sql")
            _write_exec(bundle_root / "bin" / "browseros_server")
            _write_exec(bundle_root / "bin" / "third_party" / "rg")
            _write_file(bundle_root / "db" / "migrations" / "0000_init.sql")

            self.assertEqual(
                verify_server_resources_bundle(app_path, chromium_src), []
            )

    def test_skips_claw_resource_verification_until_bundle_root_exists(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src = Path(tmp) / "src"
            app_path = Path(tmp) / "out" / "BrowserOS.app"
            source_root = (
                chromium_src
                / "chrome"
                / "browser"
                / "browseros"
                / "claw_server"
                / "resources"
            )
            _write_exec(source_root / "bin" / "browseros-claw-server")

            problems = verify_server_resources_bundle(app_path, chromium_src)

            self.assertEqual(problems, [])

    def test_reports_claw_bundle_root_for_missing_resource_once_packaged(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src = Path(tmp) / "src"
            app_path = Path(tmp) / "out" / "BrowserOS.app"
            source_root = (
                chromium_src
                / "chrome"
                / "browser"
                / "browseros"
                / "claw_server"
                / "resources"
            )
            bundle_root = (
                app_path
                / "Contents"
                / "Resources"
                / "BrowserClawServer"
                / "default"
                / "resources"
            )
            _write_exec(source_root / "bin" / "browseros-claw-server")
            bundle_root.mkdir(parents=True)

            problems = verify_server_resources_bundle(app_path, chromium_src)

            self.assertEqual(len(problems), 1)
            self.assertIn(
                "Contents/Resources/BrowserClawServer/default/resources",
                problems[0],
            )
            self.assertIn("bin/browseros-claw-server", problems[0])

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

        claw_destinations = {
            op["destination"]
            for op in config["copy_operations"]
            if op["name"].startswith("BrowserOS Claw Rust Server Resources")
        }
        self.assertEqual(
            claw_destinations,
            {"chrome/browser/browseros/claw_server/resources"},
        )


class SignModuleGuardWiringTest(unittest.TestCase):
    def test_module_guard_raises_on_stale_bundle(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src = Path(tmp) / "src"
            app_path = Path(tmp) / "out" / "BrowserOS.app"
            source_root = (
                chromium_src / "chrome" / "browser" / "browseros" / "server" / "resources"
            )
            _write_exec(source_root / "bin" / "third_party" / "rg")

            ctx = Context(
                chromium_src=chromium_src,
                architecture="arm64",
                build_type="release",
            )

            with self.assertRaises(RuntimeError) as raised:
                MacOSSignModule()._verify_server_resources(app_path, ctx)

            self.assertIn("bin/third_party/rg", str(raised.exception))

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
            _write_exec(source_root / "bin" / "third_party" / "rg")
            _write_exec(bundle_root / "bin" / "third_party" / "rg")

            ctx = Context(
                chromium_src=chromium_src,
                architecture="arm64",
                build_type="release",
            )

            MacOSSignModule()._verify_server_resources(app_path, ctx)


def _completed(cmd, returncode=0, stdout=""):
    return subprocess.CompletedProcess(cmd, returncode, stdout=stdout, stderr="")


def _fake_probe(archs, plist_archs, macho=True):
    """Stub for macos._run_probe: lipo -archs and otool -l answers."""

    def probe(cmd):
        if cmd[:2] == ["lipo", "-archs"]:
            if not macho:
                return _completed(cmd, returncode=1)
            return _completed(cmd, stdout=" ".join(archs) + "\n")
        if cmd[0] == "otool":
            arch = cmd[2]
            section = "__info_plist" if arch in plist_archs else "__text"
            return _completed(cmd, stdout=f"Section\n  sectname {section}\n")
        raise AssertionError(f"unexpected probe command: {cmd}")

    return probe


def _fake_run_command(calls, fail_predicate=None):
    """Stub for macos.run_command: records calls, materializes lipo outputs."""

    def run(cmd, cwd=None, check=True):
        calls.append(cmd)
        if fail_predicate and fail_predicate(cmd):
            raise subprocess.CalledProcessError(1, cmd)
        if cmd[0] == "lipo" and "-output" in cmd:
            payload = b"signed-fat" if "-create" in cmd else b"thin"
            Path(cmd[cmd.index("-output") + 1]).write_bytes(payload)
        return _completed(cmd)

    return run


class SignComponentPerSliceTest(unittest.TestCase):
    """Fat binaries whose slices disagree on an embedded Info.plist must be
    signed slice-by-slice: codesign on the fat file binds the file-level
    Info.plist into every slice's CodeDirectory, which the plist-less slice
    can never satisfy (Apple notarization rejects it)."""

    def _make_component(self, tmp):
        component = Path(tmp) / "tool"
        component.write_bytes(b"original-fat")
        component.chmod(0o755)
        return component

    def test_asymmetric_fat_signs_each_slice_and_reassembles(self):
        with tempfile.TemporaryDirectory() as tmp:
            component = self._make_component(tmp)
            calls = []
            with (
                mock.patch.object(
                    macos_module,
                    "_run_probe",
                    _fake_probe(["x86_64", "arm64"], {"arm64"}),
                ),
                mock.patch.object(
                    macos_module, "run_command", _fake_run_command(calls)
                ),
            ):
                ok = sign_component(
                    component, "Cert", "com.browseros.tool", "runtime"
                )

            self.assertTrue(ok)
            codesign_calls = [c for c in calls if c[0] == "codesign"]
            self.assertEqual(len(codesign_calls), 2)
            for cmd in codesign_calls:
                self.assertNotEqual(cmd[-1], str(component))
                self.assertIn("--force", cmd)
                self.assertIn("--timestamp", cmd)
                self.assertIn("--identifier", cmd)
                self.assertIn("com.browseros.tool", cmd)
                self.assertIn("--options", cmd)
                self.assertIn("runtime", cmd)
            thin_calls = [c for c in calls if c[0] == "lipo" and "-thin" in c]
            self.assertEqual(
                {c[c.index("-thin") + 1] for c in thin_calls}, {"x86_64", "arm64"}
            )
            create_calls = [c for c in calls if c[0] == "lipo" and "-create" in c]
            self.assertEqual(len(create_calls), 1)
            self.assertEqual(component.read_bytes(), b"signed-fat")
            self.assertTrue(os.access(component, os.X_OK))
            self.assertEqual(
                sorted(p.name for p in Path(tmp).iterdir()), ["tool"]
            )

    def test_symmetric_fat_uses_single_codesign(self):
        for plist_archs in ({"x86_64", "arm64"}, set()):
            with self.subTest(plist_archs=plist_archs):
                with tempfile.TemporaryDirectory() as tmp:
                    component = self._make_component(tmp)
                    calls = []
                    with (
                        mock.patch.object(
                            macos_module,
                            "_run_probe",
                            _fake_probe(["x86_64", "arm64"], plist_archs),
                        ),
                        mock.patch.object(
                            macos_module, "run_command", _fake_run_command(calls)
                        ),
                    ):
                        ok = sign_component(component, "Cert")

                    self.assertTrue(ok)
                    self.assertEqual(len(calls), 1)
                    self.assertEqual(calls[0][0], "codesign")
                    self.assertEqual(calls[0][-1], str(component))
                    self.assertEqual(component.read_bytes(), b"original-fat")

    def test_non_macho_executable_uses_single_codesign(self):
        with tempfile.TemporaryDirectory() as tmp:
            component = self._make_component(tmp)
            calls = []
            with (
                mock.patch.object(
                    macos_module, "_run_probe", _fake_probe([], set(), macho=False)
                ),
                mock.patch.object(
                    macos_module, "run_command", _fake_run_command(calls)
                ),
            ):
                ok = sign_component(component, "Cert")

            self.assertTrue(ok)
            self.assertEqual(len(calls), 1)
            self.assertEqual(calls[0][0], "codesign")
            self.assertEqual(calls[0][-1], str(component))

    def test_thin_single_arch_uses_single_codesign(self):
        with tempfile.TemporaryDirectory() as tmp:
            component = self._make_component(tmp)
            calls = []
            with (
                mock.patch.object(
                    macos_module, "_run_probe", _fake_probe(["arm64"], {"arm64"})
                ),
                mock.patch.object(
                    macos_module, "run_command", _fake_run_command(calls)
                ),
            ):
                ok = sign_component(component, "Cert")

            self.assertTrue(ok)
            self.assertEqual(len(calls), 1)
            self.assertEqual(calls[0][0], "codesign")
            self.assertEqual(calls[0][-1], str(component))

    def test_failing_slice_codesign_keeps_original_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            component = self._make_component(tmp)
            calls = []
            with (
                mock.patch.object(
                    macos_module,
                    "_run_probe",
                    _fake_probe(["x86_64", "arm64"], {"arm64"}),
                ),
                mock.patch.object(
                    macos_module,
                    "run_command",
                    _fake_run_command(
                        calls, fail_predicate=lambda cmd: cmd[0] == "codesign"
                    ),
                ),
            ):
                ok = sign_component(component, "Cert")

            self.assertFalse(ok)
            self.assertEqual(component.read_bytes(), b"original-fat")
            self.assertTrue(os.access(component, os.X_OK))
            self.assertEqual(
                sorted(p.name for p in Path(tmp).iterdir()), ["tool"]
            )


class VerifySignatureComponentTest(unittest.TestCase):
    """The app-level --deep verify seals Resources executables as plain files
    without validating their own signatures; verify_signature must check each
    file-type component directly so a bad slice fails locally, not at Apple."""

    def _build_app(self, tmp):
        app_path = Path(tmp) / "BrowserOS.app"
        rg = (
            app_path
            / "Contents"
            / "Resources"
            / "BrowserOSServer"
            / "default"
            / "resources"
            / "bin"
            / "third_party"
            / "rg"
        )
        _write_exec(rg)
        return app_path, rg

    def test_fails_when_component_signature_invalid(self):
        with tempfile.TemporaryDirectory() as tmp:
            app_path, rg = self._build_app(tmp)
            calls = []

            def run(cmd, cwd=None, check=True):
                calls.append(cmd)
                returncode = 1 if cmd[-1] == str(rg) else 0
                return _completed(cmd, returncode=returncode)

            with mock.patch.object(macos_module, "run_command", run):
                self.assertFalse(verify_signature(app_path))

            self.assertTrue(
                any(c[0] == "codesign" and c[-1] == str(rg) for c in calls)
            )

    def test_passes_and_verifies_each_component(self):
        with tempfile.TemporaryDirectory() as tmp:
            app_path, rg = self._build_app(tmp)
            calls = []

            with mock.patch.object(
                macos_module, "run_command", _fake_run_command(calls)
            ):
                self.assertTrue(verify_signature(app_path))

            self.assertTrue(
                any(
                    c[0] == "codesign" and "--verify" in c and c[-1] == str(rg)
                    for c in calls
                )
            )


if __name__ == "__main__":
    unittest.main()
