#!/usr/bin/env python3
"""Tests for OTA binary signing."""

import json
import os
import tempfile
import subprocess
import unittest
from dataclasses import replace
from pathlib import Path
from unittest import mock

from ...lib.env import EnvConfig
from ...products.browserclaw.product import BROWSERCLAW_SERVER_BUNDLE
from ...products.browseros.product import BROWSEROS_SERVER_BUNDLE
from ...lib import windows_signing
from ...lib.windows_signing import sslcom as ssl_signing
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


class SignWindowsBinaryTest(unittest.TestCase):
    """Exercise OTA through the real signer with fake subprocesses and captured logs."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.tool_dir = root / "signing tool"
        self.java = self.tool_dir / "jdk-11.0.2" / "bin" / "java.exe"
        self.jar = self.tool_dir / "jar" / "code_sign_tool-1.3.2.jar"
        self.resources = root / "staged resources"
        self.binary = self.resources / "bin" / "browseros_server.exe"
        for path in (
            self.java, self.jar, self.tool_dir / "CodeSignTool.bat", self.binary
        ):
            _write_exe(path)
        self.password = FAKE_PASSWORD + ' ^%!"'
        environment = mock.patch.dict(
            os.environ,
            {
                "CODE_SIGN_TOOL_PATH": str(self.tool_dir),
                "ESIGNER_USERNAME": "build@example.test",
                "ESIGNER_PASSWORD": self.password,
                "ESIGNER_TOTP_SECRET": FAKE_TOTP,
                "ESIGNER_CREDENTIAL_ID": "fake-credential-id",
                "WINDOWS_SIGNING_PROVIDER": "sslcom",
                "WINDOWS_SIGNING_REPORT_DIR": str(root / "reports"),
            },
            clear=True,
        )
        environment.start()
        self.addCleanup(environment.stop)
        self.env = EnvConfig()
        # OTA now crosses the shared facade before the SSL adapter. Keep the
        # existing subprocess fixtures at that boundary, with Windows policy.
        for module in (windows_signing, ssl_signing):
            platform = mock.patch.object(module, "IS_WINDOWS", return_value=True)
            platform.start()
            self.addCleanup(platform.stop)
        self.logs = []
        for module, names in (
            (sign_binary, ("log_info", "log_error", "log_success")),
            (ssl_signing, ("log_info", "log_error", "log_success")),
            (windows_signing, ("log_info", "log_error")),
        ):
            for name in names:
                patcher = mock.patch.object(module, name, side_effect=self.logs.append)
                patcher.start()
                self.addCleanup(patcher.stop)

    def test_bundle_signing_preserves_arguments_and_installs_verified_output(self):
        def fake_process(command, **kwargs):
            if command[0] == str(self.java):
                output = Path(command[command.index("-output_dir_path") + 1])
                (output / self.binary.name).write_bytes(b"signed executable")
                return subprocess.CompletedProcess(command, 0, "Signed successfully", "")
            if "-NonInteractive" in command:
                evidence = {
                    "status": "Valid",
                    "publisher": "Felafax, Inc.",
                    "issuer": "SSL.com EV Code Signing Intermediate CA RSA R3",
                    "timestamp_subject": "SSL.com Timestamping",
                }
                return subprocess.CompletedProcess(command, 0, json.dumps(evidence), "")
            return subprocess.CompletedProcess(command, 0, "Valid\n", "")

        with mock.patch.object(
            subprocess,
            "run",
            side_effect=fake_process,
        ) as run:
            self.assertTrue(
                sign_binary.sign_server_bundle_windows(
                    self.resources, self.env, BROWSEROS_SERVER_BUNDLE
                )
            )

        command = run.call_args_list[0].args[0]
        self.assertIsInstance(command, list)
        self.assertEqual(command[:4], [str(self.java), "-jar", str(self.jar), "sign"])
        self.assertFalse(run.call_args_list[0].kwargs["shell"])
        self.assertEqual(command[command.index("-password") + 1], self.password)
        staged = Path(command[command.index("-input_file_path") + 1])
        self.assertEqual(staged.name, self.binary.name)
        self.assertNotEqual(staged, self.binary)
        self.assertEqual(run.call_count, 3)
        self.assertEqual(self.binary.read_bytes(), b"signed executable")
        self.assertFalse(staged.parent.exists())

    def test_signer_errors_are_reported_and_redacted_before_verification(self):
        for returncode, stdout, stderr in (
            (3, "", f"Provider rejected {self.password} {FAKE_TOTP}"),
            (0, f"Error: Provider rejected {self.password} {FAKE_TOTP}", ""),
        ):
            with self.subTest(returncode=returncode):
                self.logs.clear()
                with mock.patch.object(
                    subprocess,
                    "run",
                    return_value=subprocess.CompletedProcess(
                        [], returncode, stdout, stderr
                    ),
                ) as run:
                    self.assertFalse(
                        sign_binary.sign_windows_binary(self.binary, self.env)
                    )

                run.assert_called_once()
                self.assertEqual(self.binary.read_bytes(), b"exe")
                logged = "\n".join(self.logs)
                self.assertNotIn(self.password, logged)
                self.assertNotIn(FAKE_TOTP, logged)
                self.assertIn("Provider rejected *** ***", logged)

    def test_windows_verification_exception_prevents_release(self):
        with (
            mock.patch.object(windows_signing, "IS_WINDOWS", return_value=True),
            mock.patch.object(
                subprocess,
                "run",
                side_effect=[
                    subprocess.CompletedProcess([], 0, "Signed successfully", ""),
                    FileNotFoundError("powershell"),
                ],
            ),
        ):
            self.assertFalse(sign_binary.sign_windows_binary(self.binary, self.env))

    def test_redacts_whitespace_bearing_credentials_before_trimming_output(self):
        password = FAKE_PASSWORD + " "
        for stream in ("stdout", "stderr"):
            with self.subTest(stream=stream):
                self.logs.clear()
                output = {"stdout": "", "stderr": ""}
                output[stream] = f"Error: Provider rejected {password}\n"
                with (
                    mock.patch.dict(os.environ, {"ESIGNER_PASSWORD": password}),
                    mock.patch.object(
                        subprocess,
                        "run",
                        return_value=subprocess.CompletedProcess([], 3, **output),
                    ),
                ):
                    self.assertFalse(
                        sign_binary.sign_windows_binary(self.binary, self.env)
                    )

                logged = "\n".join(self.logs)
                self.assertNotIn(FAKE_PASSWORD, logged)
                self.assertIn("Error: Provider rejected ***", logged)


if __name__ == "__main__":
    unittest.main()
