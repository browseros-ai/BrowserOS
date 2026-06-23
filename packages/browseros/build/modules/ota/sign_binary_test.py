#!/usr/bin/env python3
"""Tests for OTA binary signing."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from ...common.env import EnvConfig
from . import sign_binary


def _write_exe(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"exe")


class SignServerBundleWindowsTest(unittest.TestCase):
    def test_signs_shipped_windows_binaries_without_claude(self):
        with tempfile.TemporaryDirectory() as tmp:
            resources = Path(tmp) / "resources"
            _write_exe(resources / "bin" / "browseros_server.exe")
            _write_exe(resources / "bin" / "third_party" / "codex.exe")

            signed = []

            def fake_sign(path, env):
                signed.append(path.relative_to(resources / "bin").as_posix())
                return True

            with mock.patch.object(
                sign_binary, "sign_windows_binary", side_effect=fake_sign
            ):
                self.assertTrue(
                    sign_binary.sign_server_bundle_windows(resources, EnvConfig())
                )

            self.assertEqual(signed, ["browseros_server.exe", "third_party/codex.exe"])


if __name__ == "__main__":
    unittest.main()
