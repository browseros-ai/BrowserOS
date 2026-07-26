#!/usr/bin/env python3
"""Tests for OTA binary signing."""

import tempfile
import subprocess
import unittest
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock

from ...lib.env import EnvConfig
from ...products.browserclaw.product import BROWSERCLAW_SERVER_BUNDLE
from ...products.browseros.product import BROWSEROS_SERVER_BUNDLE
from . import sign_binary


FAKE_PASSWORD = "FAKE_OTA_SIGNING_PASSWORD_FOR_REDACTION_TEST"
FAKE_TOTP = "FAKE_OTA_SIGNING_TOTP_FOR_REDACTION_TEST"


def _write_exe(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"exe")


class SignServerBundleWindowsTest(unittest.TestCase):
    def _assert_signs_bundle_binary(self, bundle, binary_name):
        with tempfile.TemporaryDirectory() as tmp:
            resources = Path(tmp) / "resources"
            _write_exe(resources / "bin" / binary_name)

            signed = []

            def fake_sign(path, env):
                signed.append(path.relative_to(resources / "bin").as_posix())
                return True

            with mock.patch.object(
                sign_binary, "sign_windows_binary", side_effect=fake_sign
            ):
                self.assertTrue(
                    sign_binary.sign_server_bundle_windows(
                        resources, EnvConfig(), bundle
                    )
                )

            self.assertEqual(signed, [binary_name])

    def test_signs_browseros_descriptor_binary(self):
        self._assert_signs_bundle_binary(
            BROWSEROS_SERVER_BUNDLE, "browseros_server.exe"
        )

    def test_signs_browserclaw_descriptor_binary(self):
        self._assert_signs_bundle_binary(
            BROWSERCLAW_SERVER_BUNDLE, "browseros-claw-server.exe"
        )

    def test_fails_when_browserclaw_descriptor_binary_is_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            resources = Path(tmp) / "resources"
            _write_exe(resources / "bin" / "browseros_server.exe")

            with mock.patch.object(sign_binary, "sign_windows_binary") as signer:
                self.assertFalse(
                    sign_binary.sign_server_bundle_windows(
                        resources, EnvConfig(), BROWSERCLAW_SERVER_BUNDLE
                    )
                )

            signer.assert_not_called()

    def test_preflights_all_descriptor_binaries_before_signing(self):
        bundle = replace(
            BROWSERCLAW_SERVER_BUNDLE,
            windows_binaries=("browseros-claw-server.exe", "missing.exe"),
        )
        with tempfile.TemporaryDirectory() as tmp:
            resources = Path(tmp) / "resources"
            _write_exe(resources / "bin" / "browseros-claw-server.exe")

            with mock.patch.object(
                sign_binary, "sign_windows_binary", return_value=True
            ) as signer:
                self.assertFalse(
                    sign_binary.sign_server_bundle_windows(
                        resources, EnvConfig(), bundle
                    )
                )

            signer.assert_not_called()


class SignWindowsBinaryLoggingTest(unittest.TestCase):
    def test_redacts_credentials_echoed_in_codesigntool_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            tool = root / "CodeSignTool.bat"
            binary = root / "browseros_server.exe"
            _write_exe(tool)
            _write_exe(binary)
            env = cast(
                EnvConfig,
                SimpleNamespace(
                    code_sign_tool_exe=str(tool),
                    code_sign_tool_path=None,
                    esigner_username="build@example.test",
                    esigner_password=FAKE_PASSWORD,
                    esigner_totp_secret=FAKE_TOTP,
                    esigner_credential_id="fake-credential-id",
                ),
            )
            result = subprocess.CompletedProcess(
                "fake codesign command",
                1,
                stdout=f"Error: {FAKE_PASSWORD} {FAKE_TOTP}",
                stderr="",
            )

            with (
                mock.patch.object(sign_binary.subprocess, "run", return_value=result),
                mock.patch.object(sign_binary, "log_error") as log_error,
            ):
                self.assertFalse(sign_binary.sign_windows_binary(binary, env))

        logged = "\n".join(str(call.args[0]) for call in log_error.call_args_list)
        self.assertNotIn(FAKE_PASSWORD, logged)
        self.assertNotIn(FAKE_TOTP, logged)
        self.assertIn("Signing failed: Error: *** ***", logged)


if __name__ == "__main__":
    unittest.main()
