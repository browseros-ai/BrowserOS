#!/usr/bin/env python3
"""Tests for Sparkle/WinSparkle signing artifact selection."""

import tempfile
import unittest
from pathlib import Path

from .sparkle import find_signable_artifacts


class FindSignableArtifactsTest(unittest.TestCase):
    def test_picks_dmgs_and_installer_exe_but_not_zip(self):
        with tempfile.TemporaryDirectory() as tmp:
            dist = Path(tmp)
            (dist / "BrowserOS_v0.31.0_arm64.dmg").write_bytes(b"dmg")
            (dist / "BrowserOS_v0.31.0_x64_installer.exe").write_bytes(b"exe")
            (dist / "BrowserOS_v0.31.0_x64_installer.zip").write_bytes(b"zip")
            (dist / "notes.txt").write_text("not an artifact")

            names = [p.name for p in find_signable_artifacts(dist)]

            self.assertEqual(
                names,
                [
                    "BrowserOS_v0.31.0_arm64.dmg",
                    "BrowserOS_v0.31.0_x64_installer.exe",
                ],
            )

    def test_empty_dist(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(find_signable_artifacts(Path(tmp)), [])

    def test_filters_by_product_filename_prefix(self):
        with tempfile.TemporaryDirectory() as tmp:
            dist = Path(tmp)
            (dist / "BrowserClaw_v0.31.0_arm64.dmg").write_bytes(b"claw")
            (dist / "BrowserOS_v0.31.0_arm64.dmg").write_bytes(b"os")
            (dist / "BrowserClaw_v0.31.0_x64_installer.exe").write_bytes(b"exe")

            names = [
                p.name for p in find_signable_artifacts(dist, "BrowserClaw_v0.31.0_")
            ]

            self.assertEqual(
                names,
                [
                    "BrowserClaw_v0.31.0_arm64.dmg",
                    "BrowserClaw_v0.31.0_x64_installer.exe",
                ],
            )


if __name__ == "__main__":
    unittest.main()
