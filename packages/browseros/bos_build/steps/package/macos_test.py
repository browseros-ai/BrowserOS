#!/usr/bin/env python3
"""Tests for macOS DMG packaging."""

import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import macos as macos_module
from ...lib.notarization import NOTARYTOOL_WAIT_TIMEOUT
from .macos import notarize_dmg, sign_dmg


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


if __name__ == "__main__":
    unittest.main()
