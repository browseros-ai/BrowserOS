#!/usr/bin/env python3
"""Tests for command and logging redaction."""

import io
import json
import os
import shlex
import subprocess
import unittest
from contextlib import redirect_stdout
from unittest import mock

from . import logger, utils


FAKE_PASSWORD = "FAKE_ESIGNER_PASSWORD_FOR_REDACTION_TEST"
FAKE_TOTP = "FAKE_ESIGNER_TOTP_FOR_REDACTION_TEST"
FAKE_KEYCHAIN_PASSWORD = "FAKE_KEYCHAIN_PASSWORD_FOR_REDACTION_TEST"
FAKE_PEM_PRIVATE_KEY = """-----BEGIN FAKE TEST PRIVATE KEY-----
FAKE_PRIVATE_KEY_PAYLOAD_LINE_ONE_FOR_REDACTION_TEST
FAKE_PRIVATE_KEY_PAYLOAD_LINE_TWO_FOR_REDACTION_TEST
-----END FAKE TEST PRIVATE KEY-----"""


class _FakeProcess:
    def __init__(self, output: str, returncode: int = 0):
        self.stdout = io.StringIO(output)
        self.returncode = returncode

    def wait(self) -> None:
        return None


class CommandRedactionTest(unittest.TestCase):
    def test_redacts_separate_and_inline_secret_flags_without_mutating_command(self):
        command = [
            "CodeSignTool.bat",
            "sign",
            "-username",
            "build@example.test",
            "-password",
            f'"{FAKE_PASSWORD}"',
            "-totp_secret",
            FAKE_TOTP,
            "--password=FAKE_INLINE_PASSWORD_FOR_REDACTION_TEST",
            "artifact.exe",
        ]
        original = command.copy()

        displayed = utils.redact_command(command)

        self.assertEqual(command, original)
        self.assertIn("-username build@example.test", displayed)
        self.assertIn("-password ***", displayed)
        self.assertIn("-totp_secret ***", displayed)
        self.assertIn("--password=***", displayed)
        for secret in (
            FAKE_PASSWORD,
            FAKE_TOTP,
            "FAKE_INLINE_PASSWORD_FOR_REDACTION_TEST",
        ):
            self.assertNotIn(secret, displayed)

    def test_redacts_keychain_password_flag(self):
        displayed = utils.redact_command(
            [
                "security",
                "unlock-keychain",
                "-p",
                FAKE_KEYCHAIN_PASSWORD,
                "login.keychain-db",
            ]
        )

        self.assertEqual(
            displayed,
            "security unlock-keychain -p *** login.keychain-db",
        )

    def test_redacts_configured_secret_values_from_arbitrary_text(self):
        with mock.patch.dict(
            os.environ,
            {"ESIGNER_PASSWORD": FAKE_PASSWORD},
            clear=False,
        ):
            displayed = utils.redact_sensitive_text(
                f"signer echoed password={FAKE_PASSWORD}"
            )

        self.assertEqual(displayed, "signer echoed password=***")

    def test_redacts_credentials_configured_outside_envconfig_properties(self):
        credentials = {
            "BROWSEROS_AGENT_V2_KEY": "FAKE_AGENT_PRIVATE_KEY_FOR_REDACTION_TEST",
            "BROWSEROS_CONTROLLER_KEY": "FAKE_CONTROLLER_KEY_FOR_REDACTION_TEST",
            "BUGREPORTER_KEY": "FAKE_BUGREPORTER_KEY_FOR_REDACTION_TEST",
            "BROWSERCLAW_KEY": "FAKE_BROWSERCLAW_KEY_FOR_REDACTION_TEST",
            "CLOUDFLARE_API_TOKEN": "FAKE_CLOUDFLARE_TOKEN_FOR_REDACTION_TEST",
            "GH_TOKEN": "FAKE_GITHUB_TOKEN_FOR_REDACTION_TEST",
            "POSTHOG_API_KEY": "FAKE_POSTHOG_KEY_FOR_REDACTION_TEST",
            "SENTRY_AUTH_TOKEN": "FAKE_SENTRY_TOKEN_FOR_REDACTION_TEST",
        }
        message = "credentials: " + " ".join(credentials.values())

        with mock.patch.dict(os.environ, credentials, clear=False):
            displayed = utils.redact_sensitive_text(message)

        for credential in credentials.values():
            self.assertNotIn(credential, displayed)
        self.assertEqual(displayed, "credentials: " + " ".join(["***"] * 8))

    def test_redacts_common_escaped_and_quoted_secret_representations(self):
        secret = "FAKE_SECRET_WITH_\"DOUBLE\"_AND_'SINGLE'_FOR_REDACTION_TEST"
        representations = [
            json.dumps(secret)[1:-1],
            repr(secret),
            shlex.quote(secret),
            subprocess.list2cmdline([secret]),
        ]

        with mock.patch.dict(
            os.environ,
            {"ESIGNER_PASSWORD": secret},
            clear=False,
        ):
            displayed = utils.redact_sensitive_text(" | ".join(representations))

        self.assertEqual(displayed, "*** | *** | *** | ***")


class LoggingSinkRedactionTest(unittest.TestCase):
    def test_console_and_file_sinks_redact_configured_values(self):
        file_output = io.StringIO()
        with (
            mock.patch.dict(
                os.environ,
                {"ESIGNER_PASSWORD": FAKE_PASSWORD},
                clear=False,
            ),
            mock.patch.object(logger, "_ensure_log_file", return_value=file_output),
            mock.patch.object(logger.typer, "echo") as echo,
        ):
            logger.log_info(f"tool output: {FAKE_PASSWORD}")

        console_message = echo.call_args.args[0]
        self.assertEqual(console_message, "tool output: ***")
        self.assertNotIn(FAKE_PASSWORD, file_output.getvalue())
        self.assertIn("INFO: tool output: ***", file_output.getvalue())


class RunCommandRedactionTest(unittest.TestCase):
    def test_streams_redacted_output_but_returns_raw_output(self):
        command = [
            "fake-signer",
            "-password",
            f'"{FAKE_PASSWORD}"',
            "-totp_secret",
            FAKE_TOTP,
        ]
        process = _FakeProcess(
            f"echoed {FAKE_PASSWORD} and {FAKE_TOTP}\n",
            returncode=0,
        )
        file_messages = []
        info_messages = []
        console_output = io.StringIO()

        with (
            mock.patch.object(
                utils.subprocess, "Popen", return_value=process
            ) as popen,
            mock.patch.object(utils, "_log_to_file", side_effect=file_messages.append),
            mock.patch.object(utils, "log_info", side_effect=info_messages.append),
            redirect_stdout(console_output),
        ):
            result = utils.run_command(command)

        logged = "\n".join(file_messages + info_messages) + console_output.getvalue()
        self.assertNotIn(FAKE_PASSWORD, logged)
        self.assertNotIn(FAKE_TOTP, logged)
        self.assertIn("-password ***", logged)
        self.assertIn("-totp_secret ***", logged)
        self.assertIn("echoed *** and ***", logged)
        self.assertEqual(
            result.stdout,
            f"echoed {FAKE_PASSWORD} and {FAKE_TOTP}",
        )
        self.assertEqual(result.args, command)
        self.assertEqual(popen.call_args.args[0], command)

    def test_failure_logs_never_repeat_the_raw_command(self):
        command = ["fake-signer", "-password", FAKE_PASSWORD]
        process = _FakeProcess(f"tool echoed {FAKE_PASSWORD}\n", returncode=7)
        file_messages = []
        error_messages = []

        with (
            mock.patch.object(utils.subprocess, "Popen", return_value=process),
            mock.patch.object(utils, "_log_to_file", side_effect=file_messages.append),
            mock.patch.object(utils, "log_info"),
            mock.patch.object(utils, "log_error", side_effect=error_messages.append),
            self.assertRaises(subprocess.CalledProcessError) as raised,
        ):
            utils.run_command(command)

        logged = "\n".join(file_messages + error_messages)
        self.assertNotIn(FAKE_PASSWORD, logged)
        self.assertIn("Command failed: fake-signer -password ***", logged)
        self.assertNotIn(FAKE_PASSWORD, str(raised.exception))
        self.assertNotIn(FAKE_PASSWORD, repr(raised.exception))
        self.assertEqual(raised.exception.cmd, command)
        self.assertEqual(raised.exception.output, f"tool echoed {FAKE_PASSWORD}")

    def test_empty_env_redacts_values_from_the_inherited_process_environment(self):
        command = ["fake-signer"]
        process = _FakeProcess(f"echoed {FAKE_PASSWORD}\n")
        file_messages = []
        console_output = io.StringIO()

        with (
            mock.patch.dict(
                os.environ,
                {"ESIGNER_PASSWORD": FAKE_PASSWORD},
                clear=False,
            ),
            mock.patch.object(utils.subprocess, "Popen", return_value=process),
            mock.patch.object(utils, "_log_to_file", side_effect=file_messages.append),
            mock.patch.object(utils, "log_info"),
            redirect_stdout(console_output),
        ):
            result = utils.run_command(command, env={})

        logged = "\n".join(file_messages) + console_output.getvalue()
        self.assertNotIn(FAKE_PASSWORD, logged)
        self.assertIn("echoed ***", logged)
        self.assertEqual(result.stdout, f"echoed {FAKE_PASSWORD}")

    def test_streams_multiline_private_key_output_without_leaking_lines(self):
        command = ["fake-extension-builder"]
        process = _FakeProcess(f"{FAKE_PEM_PRIVATE_KEY}\n")
        file_messages = []
        console_output = io.StringIO()

        with (
            mock.patch.object(utils.subprocess, "Popen", return_value=process),
            mock.patch.object(utils, "_log_to_file", side_effect=file_messages.append),
            mock.patch.object(utils, "log_info"),
            redirect_stdout(console_output),
        ):
            result = utils.run_command(
                command,
                env={"BROWSEROS_AGENT_V2_KEY": FAKE_PEM_PRIVATE_KEY},
            )

        logged = "\n".join(file_messages) + console_output.getvalue()
        for line in FAKE_PEM_PRIVATE_KEY.splitlines():
            self.assertNotIn(line, logged)
        self.assertEqual(logged.count("***"), 8)
        self.assertEqual(result.stdout, FAKE_PEM_PRIVATE_KEY)


if __name__ == "__main__":
    unittest.main()
