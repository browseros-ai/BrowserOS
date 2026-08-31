#!/usr/bin/env python3
"""Tests for macOS app signing discovery."""

import os
import plistlib
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock

import yaml

from ...core.context import Context
from ...core.products import get_product_descriptor
from ...lib.notarization import NOTARYTOOL_WAIT_TIMEOUT
from . import macos as macos_module
from .macos import (
    SERVER_RESOURCES_SOURCE_REL,
    MacOSSignModule,
    check_environment,
    find_components_to_sign,
    notarize_app,
    sign_component,
    unlock_keychain,
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


def _env(**values):
    env = type("Env", (), {})()
    for name, value in values.items():
        setattr(env, name, value)
    return env


PASSKEY_TEAM_ID = "8YMKWU47S5"
PASSKEY_BUNDLE_ID = "com.browseros.BrowserOS"
PASSKEY_BUNDLE_IDS = {
    "browseros": PASSKEY_BUNDLE_ID,
    "browserclaw": "com.browseros.BrowserClaw",
}


def _passkey_profile(
    *,
    team_id: str = PASSKEY_TEAM_ID,
    bundle_id: str = PASSKEY_BUNDLE_ID,
    browser_capability: bool = True,
) -> dict:
    """Return the allowlist shape Apple embeds in a provisioning profile."""
    return {
        "Entitlements": {
            "com.apple.application-identifier": f"{team_id}.{bundle_id}",
            "com.apple.developer.team-identifier": team_id,
            "keychain-access-groups": [f"{team_id}.{bundle_id}.*"],
            macos_module.BROWSER_PASSKEY_ENTITLEMENT: browser_capability,
        }
    }


def _write_passkey_template(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    prefix = "${CHROMIUM_TEAM_ID}.${CHROMIUM_BUNDLE_ID}"
    payload = {
        "com.apple.application-identifier": prefix,
        "keychain-access-groups": [
            f"{prefix}.{suffix}"
            for suffix in macos_module.BROWSER_KEYCHAIN_GROUP_SUFFIXES
        ],
        macos_module.BROWSER_PASSKEY_ENTITLEMENT: True,
    }
    path.write_bytes(plistlib.dumps(payload))


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
            if op["name"].startswith("BrowserOS Claw Server Resources")
            or op["name"].startswith("BrowserOS Claw Rust Server Resources")
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


class MacOSKeychainSelectionTest(unittest.TestCase):
    def test_unlock_keychain_uses_configured_keychain(self):
        with tempfile.TemporaryDirectory() as tmp:
            keychain = Path(tmp) / "ci.keychain-db"
            keychain.write_text("keychain")
            calls = []
            env = _env(
                macos_keychain_path=str(keychain),
                macos_keychain_password="password",
            )

            with mock.patch.object(
                macos_module, "run_command", _fake_run_command(calls)
            ):
                unlock_keychain(env)

            self.assertEqual(
                calls[0],
                ["security", "unlock-keychain", "-p", "password", str(keychain)],
            )
            self.assertEqual(calls[1][-1], str(keychain))

    def test_unlock_keychain_requires_existing_configured_keychain(self):
        env = _env(
            macos_keychain_path="/tmp/missing-browseros-ci.keychain-db",
            macos_keychain_password="password",
        )

        with self.assertRaisesRegex(RuntimeError, "Configured keychain not found"):
            unlock_keychain(env)

    def test_check_environment_exposes_configured_keychain(self):
        env = _env(
            macos_certificate_name="Developer ID Application",
            macos_notarization_apple_id="dev@example.com",
            macos_notarization_team_id="TEAMID1234",
            macos_notarization_password="notary-password",
            macos_keychain_path="/tmp/browseros-ci.keychain-db",
        )

        ok, values = check_environment(env)

        self.assertTrue(ok)
        self.assertEqual(values["keychain_path"], "/tmp/browseros-ci.keychain-db")
        self.assertEqual(values["keychain_profile"], "notarytool-profile")

    def test_sign_component_passes_fingerprint_and_keychain_to_codesign(self):
        with tempfile.TemporaryDirectory() as tmp:
            component = Path(tmp) / "tool"
            component.write_bytes(b"not-macho")
            keychain = Path(tmp) / "ci.keychain-db"
            fingerprint = "0123456789abcdef0123456789abcdef01234567"
            calls = []

            with (
                mock.patch.object(
                    macos_module, "_run_probe", _fake_probe([], set(), macho=False)
                ),
                mock.patch.object(
                    macos_module, "run_command", _fake_run_command(calls)
                ),
            ):
                ok = sign_component(component, fingerprint, keychain_path=keychain)

            self.assertTrue(ok)
            self.assertEqual(calls[0][calls[0].index("--sign") + 1], fingerprint)
            self.assertIn("--keychain", calls[0])
            self.assertEqual(calls[0][calls[0].index("--keychain") + 1], str(keychain))

    def test_notarize_app_uses_configured_keychain_for_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            app_path = Path(tmp) / "BrowserOS.app"
            app_path.mkdir()
            keychain = Path(tmp) / "ci.keychain-db"
            calls = []

            def run(cmd, cwd=None, check=True):
                calls.append(cmd)
                if cmd[0] == "ditto":
                    Path(cmd[-1]).write_text("zip")
                if cmd[:3] == ["xcrun", "notarytool", "submit"]:
                    return _completed(cmd, stdout="status: Accepted\n")
                return _completed(cmd)

            env_vars = {
                "apple_id": "dev@example.com",
                "team_id": "TEAMID1234",
                "notarization_pwd": "notary-password",
                "keychain_profile": "notarytool-profile",
            }

            with mock.patch.object(macos_module, "run_command", run):
                self.assertTrue(
                    notarize_app(app_path, Path(tmp), env_vars, keychain_path=keychain)
                )

            store = next(c for c in calls if c[:3] == ["xcrun", "notarytool", "store-credentials"])
            submit = next(c for c in calls if c[:3] == ["xcrun", "notarytool", "submit"])
            for cmd in (store, submit):
                self.assertIn("--keychain", cmd)
                self.assertEqual(cmd[cmd.index("--keychain") + 1], str(keychain))
            self.assertEqual(
                submit[submit.index("--timeout") + 1],
                NOTARYTOOL_WAIT_TIMEOUT,
            )

    def test_notarize_app_bounds_direct_credentials_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            app_path = Path(tmp) / "BrowserOS.app"
            app_path.mkdir()
            calls = []

            def run(cmd, cwd=None, check=True):
                calls.append(cmd)
                if cmd[0] == "ditto":
                    Path(cmd[-1]).write_text("zip")
                    return _completed(cmd)
                if cmd[:3] == ["xcrun", "notarytool", "store-credentials"]:
                    return _completed(cmd, returncode=1)
                if cmd[:3] == ["xcrun", "notarytool", "submit"]:
                    return _completed(cmd, stdout="status: Accepted\n")
                return _completed(cmd)

            env_vars = {
                "apple_id": "dev@example.com",
                "team_id": "TEAMID1234",
                "notarization_pwd": "notary-password",
                "keychain_profile": "notarytool-profile",
            }

            with mock.patch.object(macos_module, "run_command", run):
                self.assertTrue(notarize_app(app_path, Path(tmp), env_vars))

            submit = next(
                c for c in calls if c[:3] == ["xcrun", "notarytool", "submit"]
            )
            self.assertIn("--apple-id", submit)
            self.assertEqual(
                submit[submit.index("--timeout") + 1],
                NOTARYTOOL_WAIT_TIMEOUT,
            )

    def test_notarize_app_requires_profile_storage_for_configured_keychain(self):
        with tempfile.TemporaryDirectory() as tmp:
            app_path = Path(tmp) / "BrowserOS.app"
            app_path.mkdir()
            keychain = Path(tmp) / "ci.keychain-db"
            calls = []

            def run(cmd, cwd=None, check=True):
                calls.append(cmd)
                if cmd[0] == "ditto":
                    Path(cmd[-1]).write_text("zip")
                    return _completed(cmd)
                if cmd[:3] == ["xcrun", "notarytool", "store-credentials"]:
                    return _completed(cmd, returncode=1)
                raise AssertionError(f"unexpected command: {cmd}")

            env_vars = {
                "apple_id": "dev@example.com",
                "team_id": "TEAMID1234",
                "notarization_pwd": "notary-password",
                "keychain_profile": "notarytool-profile",
            }

            with mock.patch.object(macos_module, "run_command", run):
                self.assertFalse(
                    notarize_app(app_path, Path(tmp), env_vars, keychain_path=keychain)
                )

            self.assertFalse(
                any(c[:3] == ["xcrun", "notarytool", "submit"] for c in calls)
            )


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


class BrowserPasskeySigningTest(unittest.TestCase):
    def _context(
        self,
        root: Path,
        *,
        product_id: str = "browseros",
        build_type: str = "release",
    ) -> Context:
        return cast(
            Context,
            SimpleNamespace(
                build_type=build_type,
                chromium_src=root / "chromium",
                env=_env(
                    macos_browseros_passkey_profile_path=None,
                    macos_browserclaw_passkey_profile_path=None,
                ),
                product=get_product_descriptor(product_id),
                root_dir=root / "browseros",
            ),
        )

    def test_profile_accepts_app_specific_wildcard_groups(self):
        macos_module.validate_browser_passkey_profile(
            _passkey_profile(), PASSKEY_TEAM_ID, PASSKEY_BUNDLE_ID
        )

    def test_profile_rejects_wrong_app_and_missing_managed_capability(self):
        cases = (
            (
                _passkey_profile(bundle_id="com.example.Other"),
                "application identifier",
            ),
            (_passkey_profile(browser_capability=False), "does not authorize"),
        )
        for profile, message in cases:
            with self.subTest(message=message):
                with self.assertRaisesRegex(RuntimeError, message):
                    macos_module.validate_browser_passkey_profile(
                        profile, PASSKEY_TEAM_ID, PASSKEY_BUNDLE_ID
                    )

    def test_release_preflight_rejects_team_drift(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            profile_path = root / "BrowserOS.provisionprofile"
            profile_path.write_bytes(b"profile")
            ctx = self._context(root)
            ctx.env = _env(
                macos_browseros_passkey_profile_path=str(profile_path),
                macos_notarization_team_id="WRONGTEAM1",
            )
            branding = (
                ctx.root_dir
                / "chromium_files"
                / "products"
                / "browseros"
                / "chrome"
                / "app"
                / "theme"
                / "chromium"
                / "BRANDING.release"
            )
            branding.parent.mkdir(parents=True)
            branding.write_text(f"MAC_TEAM_ID={PASSKEY_TEAM_ID}\n")

            with (
                mock.patch.object(
                    macos_module,
                    "decode_provisioning_profile",
                    return_value=_passkey_profile(),
                ),
                self.assertRaisesRegex(
                    macos_module.ValidationError, "does not match.*release branding"
                ),
            ):
                MacOSSignModule().preflight(ctx)

    def test_each_product_can_build_without_a_passkey_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for product_id in macos_module.BROWSER_PASSKEY_PRODUCTS:
                with self.subTest(product=product_id, build_type="debug"):
                    MacOSSignModule().preflight(
                        self._context(root, product_id=product_id, build_type="debug")
                    )

                with self.subTest(product=product_id, build_type="release"):
                    MacOSSignModule().preflight(
                        self._context(root, product_id=product_id)
                    )

    def test_configured_missing_profile_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for product_id, config in macos_module.BROWSER_PASSKEY_PRODUCTS.items():
                with self.subTest(product=product_id):
                    ctx = self._context(root, product_id=product_id)
                    setattr(ctx.env, config.env_attr, str(root / "missing.profile"))
                    with self.assertRaisesRegex(
                        macos_module.ValidationError,
                        "profile not found",
                    ):
                        MacOSSignModule().preflight(ctx)

    def test_release_preflight_accepts_each_products_own_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for product_id, bundle_id in PASSKEY_BUNDLE_IDS.items():
                with self.subTest(product=product_id):
                    ctx = self._context(root, product_id=product_id)
                    config = macos_module.BROWSER_PASSKEY_PRODUCTS[product_id]
                    profile_path = root / f"{product_id}.provisionprofile"
                    profile_path.write_bytes(b"profile")
                    setattr(ctx.env, config.env_attr, str(profile_path))
                    ctx.env.macos_notarization_team_id = PASSKEY_TEAM_ID
                    branding = (
                        ctx.root_dir
                        / "chromium_files"
                        / "products"
                        / product_id
                        / "chrome"
                        / "app"
                        / "theme"
                        / "chromium"
                        / "BRANDING.release"
                    )
                    branding.parent.mkdir(parents=True)
                    branding.write_text(f"MAC_TEAM_ID={PASSKEY_TEAM_ID}\n")

                    with mock.patch.object(
                        macos_module,
                        "decode_provisioning_profile",
                        return_value=_passkey_profile(bundle_id=bundle_id),
                    ):
                        MacOSSignModule().preflight(ctx)

    def test_missing_profile_uses_standard_entitlements_and_removes_stale_copy(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = self._context(root)
            ctx.get_entitlements_dir = lambda: root / "entitlements"
            app_path = root / "BrowserOS.app"
            stale_profile = app_path / "Contents" / "embedded.provisionprofile"
            stale_profile.parent.mkdir(parents=True)
            stale_profile.write_bytes(b"stale profile")
            generic_entitlements = (
                ctx.chromium_src / "chrome" / "app" / "app-entitlements.plist"
            )
            generic_entitlements.parent.mkdir(parents=True)
            generic_entitlements.write_text("standard entitlements")

            with macos_module.resolved_app_entitlements(
                app_path, root, ctx, None
            ) as resolved:
                self.assertEqual(resolved, generic_entitlements)
                self.assertFalse(stale_profile.exists())

    def test_resolved_entitlements_embed_profile_and_resolve_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            app_path = root / "BrowserOS.app"
            (app_path / "Contents").mkdir(parents=True)
            profile_path = root / "BrowserOS.provisionprofile"
            profile_path.write_bytes(b"profile payload")
            template_path = root / "app-entitlements-browseros.plist"
            _write_passkey_template(template_path)
            inputs = macos_module.BrowserPasskeySigningInputs(
                team_id=PASSKEY_TEAM_ID,
                bundle_id=PASSKEY_BUNDLE_ID,
                profile_path=profile_path,
                entitlements_template=template_path,
            )

            with macos_module.resolved_app_entitlements(
                app_path, root, None, inputs
            ) as rendered_path:
                self.assertIsNotNone(rendered_path)
                assert rendered_path is not None
                rendered = plistlib.loads(rendered_path.read_bytes())
                macos_module.validate_browser_passkey_entitlements(
                    rendered,
                    PASSKEY_TEAM_ID,
                    PASSKEY_BUNDLE_ID,
                    "test entitlements",
                )
                temporary_path = rendered_path

            self.assertFalse(temporary_path.exists())
            self.assertEqual(
                (app_path / "Contents" / "embedded.provisionprofile").read_bytes(),
                b"profile payload",
            )

    def test_each_compiled_framework_must_contain_its_signing_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for product_id, bundle_id in PASSKEY_BUNDLE_IDS.items():
                with self.subTest(product=product_id):
                    ctx = self._context(root, product_id=product_id)
                    app_path = root / f"{product_id}.app"
                    framework_name = ctx.product.mac_framework_name(ctx.build_type)
                    framework = (
                        app_path
                        / "Contents"
                        / "Frameworks"
                        / framework_name
                        / "Versions"
                        / "Current"
                        / framework_name.removesuffix(".framework")
                    )
                    framework.parent.mkdir(parents=True)
                    framework.write_bytes(
                        f"{PASSKEY_TEAM_ID}.{bundle_id}.webauthn".encode()
                    )

                    macos_module.verify_compiled_browser_passkey_identity(
                        app_path, ctx, PASSKEY_TEAM_ID
                    )
                    with self.assertRaisesRegex(RuntimeError, "WRONGTEAM1"):
                        macos_module.verify_compiled_browser_passkey_identity(
                            app_path, ctx, "WRONGTEAM1"
                        )

    def test_product_profile_path_selection_does_not_cross_app_ids(self):
        env = _env(
            macos_browseros_passkey_profile_path="/profiles/browseros.provisionprofile",
            macos_browserclaw_passkey_profile_path="/profiles/browserclaw.provisionprofile",
        )
        self.assertEqual(
            macos_module.get_browser_passkey_profile_path(env, "browseros"),
            Path("/profiles/browseros.provisionprofile"),
        )
        self.assertEqual(
            macos_module.get_browser_passkey_profile_path(env, "browserclaw"),
            Path("/profiles/browserclaw.provisionprofile"),
        )

    def test_post_sign_verification_checks_team_claims_and_embedded_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = self._context(root)
            app_path = root / "BrowserOS.app"
            profile_path = app_path / "Contents" / "embedded.provisionprofile"
            profile_path.parent.mkdir(parents=True)
            profile_path.write_bytes(b"profile")
            ctx.env.macos_browseros_passkey_profile_path = str(profile_path)
            claims = dict(_passkey_profile()["Entitlements"])
            claims["keychain-access-groups"] = list(
                macos_module.browser_passkey_groups(PASSKEY_TEAM_ID, PASSKEY_BUNDLE_ID)
            )

            def run(cmd, cwd=None, check=True):
                if "--verbose=4" in cmd:
                    return _completed(cmd, stdout=f"TeamIdentifier={PASSKEY_TEAM_ID}")
                if "--entitlements" in cmd:
                    return _completed(
                        cmd, stdout=plistlib.dumps(claims).decode("utf-8")
                    )
                self.fail(f"unexpected command: {cmd}")

            with (
                mock.patch.object(macos_module, "run_command", run),
                mock.patch.object(
                    macos_module,
                    "decode_provisioning_profile",
                    return_value=_passkey_profile(),
                ),
            ):
                macos_module.verify_browser_passkey_signature(
                    app_path, ctx, PASSKEY_TEAM_ID
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
