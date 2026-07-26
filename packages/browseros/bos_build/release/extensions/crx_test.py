#!/usr/bin/env python3
"""Tests for CRX packaging helpers (command assembly only — no real chrome)."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from .crx import find_chrome_binary, pack_crx, pack_extension_command


class FindChromeBinaryTest(unittest.TestCase):
    def test_explicit_argument_wins(self):
        found = find_chrome_binary(
            "/custom/chrome", is_valid=lambda p: p == "/custom/chrome"
        )
        self.assertEqual(found, "/custom/chrome")

    def test_env_var_used_when_no_argument(self):
        with patch.dict("os.environ", {"CHROME_BINARY": "/env/chrome"}):
            found = find_chrome_binary(None, is_valid=lambda p: p == "/env/chrome")
        self.assertEqual(found, "/env/chrome")

    def test_first_valid_platform_candidate(self):
        import os

        with patch.dict("os.environ", {}, clear=False):
            os.environ.pop("CHROME_BINARY", None)
            found = find_chrome_binary(
                None,
                is_valid=lambda p: p == "google-chrome",
                platform_name="Linux",
            )
        self.assertEqual(found, "google-chrome")

    def test_none_found_raises_actionable(self):
        import os

        with patch.dict("os.environ", {}, clear=False):
            os.environ.pop("CHROME_BINARY", None)
            with self.assertRaisesRegex(RuntimeError, "CHROME_BINARY"):
                find_chrome_binary(
                    None, is_valid=lambda p: False, platform_name="Darwin"
                )

    def test_windows_candidate_supported(self):
        import os

        with patch.dict("os.environ", {}, clear=False):
            os.environ.pop("CHROME_BINARY", None)
            found = find_chrome_binary(
                None,
                is_valid=lambda p: p.endswith(r"Google\Chrome\Application\chrome.exe"),
                platform_name="Windows",
            )
        self.assertEqual(
            found, r"C:\Program Files\Google\Chrome\Application\chrome.exe"
        )

    def test_invalid_explicit_binary_raises(self):
        with self.assertRaisesRegex(RuntimeError, "/broken/path"):
            find_chrome_binary(
                "/broken/path",
                is_valid=lambda p: p == "chromium",
                platform_name="Linux",
            )


class PackExtensionCommandTest(unittest.TestCase):
    def test_command_shape(self):
        cmd = pack_extension_command(
            "google-chrome", Path("/work/dist"), Path("/tmp/key.pem")
        )
        self.assertEqual(
            cmd,
            [
                "google-chrome",
                "--pack-extension=/work/dist",
                "--pack-extension-key=/tmp/key.pem",
            ],
        )


class PackCrxTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.dist = self.root / "dist" / "chrome-mv3"
        self.dist.mkdir(parents=True)
        (self.dist / "manifest.json").write_text("{}")
        self.out = self.root / "out" / "agent-1.0.0.crx"

    def _fake_run(self, create_crx=True, returncode=0, stderr=""):
        recorded = {}

        def run(cmd):
            recorded["cmd"] = cmd
            key_path = cmd[2].split("=", 1)[1]
            recorded["key_existed"] = Path(key_path).exists()
            recorded["key_content"] = Path(key_path).read_text()
            if create_crx:
                Path(f"{self.dist}.crx").write_bytes(b"crx-bytes")
            return SimpleNamespace(returncode=returncode, stderr=stderr)

        return run, recorded

    def test_packs_moves_and_cleans_up_key(self):
        run, recorded = self._fake_run()
        result = pack_crx(self.dist, "PEM-CONTENT", "google-chrome", self.out, run=run)

        self.assertEqual(result, self.out)
        self.assertEqual(self.out.read_bytes(), b"crx-bytes")
        self.assertFalse(Path(f"{self.dist}.crx").exists())
        self.assertEqual(recorded["cmd"][0], "google-chrome")
        self.assertTrue(recorded["key_existed"])
        self.assertEqual(recorded["key_content"], "PEM-CONTENT")
        key_path = recorded["cmd"][2].split("=", 1)[1]
        self.assertFalse(Path(key_path).exists())

    def test_chrome_failure_raises_with_stderr(self):
        run, recorded = self._fake_run(create_crx=False, returncode=1, stderr="boom")
        with self.assertRaisesRegex(RuntimeError, "boom"):
            pack_crx(self.dist, "KEY", "chrome", self.out, run=run)
        key_path = recorded["cmd"][2].split("=", 1)[1]
        self.assertFalse(Path(key_path).exists())

    def test_missing_crx_output_raises(self):
        run, _ = self._fake_run(create_crx=False)
        with self.assertRaisesRegex(RuntimeError, "crx"):
            pack_crx(self.dist, "KEY", "chrome", self.out, run=run)

    def test_missing_dist_dir_raises_before_chrome(self):
        calls = []

        def run(cmd):
            calls.append(cmd)
            return SimpleNamespace(returncode=0, stderr="")

        with self.assertRaises(FileNotFoundError):
            pack_crx(self.root / "nope", "KEY", "chrome", self.out, run=run)
        self.assertEqual(calls, [])

    def test_missing_manifest_raises_before_chrome(self):
        (self.dist / "manifest.json").unlink()
        calls = []

        def run(cmd):
            calls.append(cmd)
            return SimpleNamespace(returncode=0, stderr="")

        with self.assertRaisesRegex(FileNotFoundError, "manifest.json"):
            pack_crx(self.dist, "KEY", "chrome", self.out, run=run)
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
