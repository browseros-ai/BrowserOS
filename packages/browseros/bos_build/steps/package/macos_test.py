#!/usr/bin/env python3
"""Tests for macOS DMG packaging."""

import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

from . import macos as macos_module
from ...lib.notarization import NOTARYTOOL_WAIT_TIMEOUT
from .macos import DmgAppearance, notarize_dmg, resolve_dmg_appearance, sign_dmg


def _completed(cmd, returncode=0, stdout=""):
    return subprocess.CompletedProcess(cmd, returncode, stdout=stdout, stderr="")


class MacOSPackageKeychainTest(unittest.TestCase):
    def test_sign_dmg_passes_fingerprint_and_keychain_to_codesign(self):
        with tempfile.TemporaryDirectory() as tmp:
            dmg_path = Path(tmp) / "BrowserOS.dmg"
            dmg_path.write_text("dmg")
            keychain = Path(tmp) / "ci.keychain-db"
            fingerprint = "0123456789abcdef0123456789abcdef01234567"
            calls = []

            def run(cmd, cwd=None, check=True):
                calls.append(cmd)
                return _completed(cmd)

            with mock.patch.object(macos_module, "run_command", run):
                self.assertTrue(sign_dmg(dmg_path, fingerprint, keychain))

            sign_cmd = calls[0]
            self.assertEqual(sign_cmd[0], "codesign")
            self.assertEqual(sign_cmd[sign_cmd.index("--sign") + 1], fingerprint)
            self.assertIn("--keychain", sign_cmd)
            self.assertEqual(
                sign_cmd[sign_cmd.index("--keychain") + 1],
                str(keychain),
            )

    def test_notarize_dmg_passes_keychain_and_bounded_wait_to_notarytool(self):
        with tempfile.TemporaryDirectory() as tmp:
            dmg_path = Path(tmp) / "BrowserOS.dmg"
            dmg_path.write_text("dmg")
            keychain = Path(tmp) / "ci.keychain-db"
            calls = []

            def run(cmd, cwd=None, check=True):
                calls.append(cmd)
                if cmd[:3] == ["xcrun", "notarytool", "submit"]:
                    return _completed(cmd, stdout="status: Accepted\n")
                return _completed(cmd)

            with mock.patch.object(macos_module, "run_command", run):
                self.assertTrue(
                    notarize_dmg(
                        dmg_path,
                        "notarytool-profile",
                        keychain,
                    )
                )

            submit = calls[0]
            self.assertEqual(submit[:3], ["xcrun", "notarytool", "submit"])
            self.assertIn("--keychain", submit)
            self.assertEqual(
                submit[submit.index("--keychain") + 1],
                str(keychain),
            )
            self.assertEqual(
                submit[submit.index("--timeout") + 1],
                NOTARYTOOL_WAIT_TIMEOUT,
            )

    def test_notarize_dmg_bounds_direct_credentials_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            dmg_path = Path(tmp) / "BrowserOS.dmg"
            dmg_path.write_text("dmg")
            calls = []
            submit_count = 0

            def run(cmd, cwd=None, check=True):
                nonlocal submit_count
                calls.append(cmd)
                if cmd[:3] == ["xcrun", "notarytool", "submit"]:
                    submit_count += 1
                    if submit_count == 1:
                        return _completed(cmd, returncode=1)
                    return _completed(cmd, stdout="status: Accepted\n")
                return _completed(cmd)

            with mock.patch.object(macos_module, "run_command", run):
                self.assertTrue(
                    notarize_dmg(
                        dmg_path,
                        notarization_env={
                            "apple_id": "release@example.com",
                            "team_id": "TEAMID",
                            "notarization_pwd": "app-password",
                        },
                    )
                )

            submit_calls = [
                call for call in calls if call[:3] == ["xcrun", "notarytool", "submit"]
            ]
            self.assertEqual(len(submit_calls), 2)
            for submit in submit_calls:
                self.assertEqual(
                    submit[submit.index("--timeout") + 1],
                    NOTARYTOOL_WAIT_TIMEOUT,
                )


def _fake_dmgbuild(capture, on_build=None):
    """A stand-in dmgbuild module that records what build_dmg was handed."""
    module = types.ModuleType("dmgbuild")

    def build_dmg(filename, volume_name, settings_file, lookForHiDPI):
        capture["filename"] = filename
        capture["volume_name"] = volume_name
        capture["settings"] = Path(settings_file).read_text()
        capture["hidpi"] = lookForHiDPI
        if on_build is not None:
            on_build()
        Path(filename).write_text("dmg")

    module.build_dmg = build_dmg
    return module


class MacOSDmgLayoutTest(unittest.TestCase):
    def _app_and_background(self, tmp):
        app = Path(tmp) / "BrowserOS neo.app"
        app.mkdir()
        background = Path(tmp) / "background.png"
        background.write_bytes(b"png")
        return app, background

    def test_create_dmg_lays_out_window_when_appearance_present(self):
        with tempfile.TemporaryDirectory() as tmp:
            app, background = self._app_and_background(tmp)
            dmg = Path(tmp) / "out.dmg"
            appearance = DmgAppearance(background=background)
            capture = {}

            with mock.patch.dict(sys.modules, {"dmgbuild": _fake_dmgbuild(capture)}):
                ok = macos_module.create_dmg(
                    app, dmg, "BrowserOS neo", appearance=appearance
                )

            self.assertTrue(ok)
            self.assertEqual(capture["volume_name"], "BrowserOS neo")
            self.assertEqual(capture["filename"], str(dmg))
            self.assertTrue(capture["hidpi"])
            settings = capture["settings"]
            self.assertIn('format = "UDBZ"', settings)
            self.assertIn('"Applications": "/Applications"', settings)
            self.assertIn("'BrowserOS neo.app'", settings)
            self.assertIn("(165, 190)", settings)
            self.assertIn("(475, 190)", settings)

    def test_create_dmg_without_appearance_uses_pkg_dmg(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp) / "App.app"
            app.mkdir()
            dmg = Path(tmp) / "out.dmg"
            pkg_dmg = Path(tmp) / "pkg-dmg"
            pkg_dmg.write_text("#!/bin/sh\n")
            calls = []

            def run(cmd, cwd=None, check=True):
                calls.append(cmd)
                dmg.write_text("dmg")
                return _completed(cmd)

            with mock.patch.object(macos_module, "run_command", run):
                ok = macos_module.create_dmg(
                    app, dmg, "BrowserOS", pkg_dmg_path=pkg_dmg, appearance=None
                )

            self.assertTrue(ok)
            self.assertEqual(calls[0][0], str(pkg_dmg))
            self.assertIn("--symlink", calls[0])
            self.assertIn("/Applications:/Applications", calls[0])
            self.assertEqual(calls[0][calls[0].index("--format") + 1], "UDBZ")

    def test_create_dmg_fails_hard_when_layout_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            app, background = self._app_and_background(tmp)
            dmg = Path(tmp) / "out.dmg"
            pkg_dmg = Path(tmp) / "pkg-dmg"
            pkg_dmg.write_text("#!/bin/sh\n")
            appearance = DmgAppearance(background=background)
            capture = {}
            run_calls = []

            def boom():
                raise RuntimeError("layout failed")

            with (
                mock.patch.dict(
                    sys.modules, {"dmgbuild": _fake_dmgbuild(capture, on_build=boom)}
                ),
                mock.patch.object(
                    macos_module, "run_command", lambda *a, **k: run_calls.append(a)
                ),
            ):
                ok = macos_module.create_dmg(
                    app,
                    dmg,
                    "BrowserOS neo",
                    pkg_dmg_path=pkg_dmg,
                    appearance=appearance,
                )

            self.assertFalse(ok)
            # No silent fall back to an unstyled pkg-dmg image.
            self.assertEqual(run_calls, [])

    def test_resolve_dmg_appearance_opt_in(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            assets = root / "resources" / "browserclaw" / "dmg-assets"
            ctx = types.SimpleNamespace(
                root_dir=root,
                product=types.SimpleNamespace(id="browserclaw"),
                get_dmg_assets_dir=lambda: assets,
            )

            self.assertIsNone(resolve_dmg_appearance(ctx))

            assets.mkdir(parents=True)
            (assets / "background.png").write_bytes(b"png")
            appearance = resolve_dmg_appearance(ctx)
            self.assertIsNotNone(appearance)
            self.assertEqual(appearance.background, assets / "background.png")
            self.assertIsNone(appearance.volume_icon)

            icns = root / "resources" / "browserclaw" / "icons" / "mac" / "app.icns"
            icns.parent.mkdir(parents=True)
            icns.write_bytes(b"icns")
            self.assertEqual(resolve_dmg_appearance(ctx).volume_icon, icns)


if __name__ == "__main__":
    unittest.main()
