#!/usr/bin/env python3
"""Tests for the WinSparkle setup module."""

import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

from . import git
from ...core.step import ValidationError
from ...lib.testing import MockBrowserOSRoot, MockChromium, make_context


def _make_release_zip(path: Path, version: str = "0.9.3") -> None:
    """Build a zip shaped like the official WinSparkle release archive."""
    top = f"WinSparkle-{version}"
    with zipfile.ZipFile(path, "w") as zf:
        zf.writestr(f"{top}/include/winsparkle.h", "// header\n")
        zf.writestr(f"{top}/include/winsparkle-version.h", "// version\n")
        zf.writestr(f"{top}/x64/Release/WinSparkle.dll", b"dll-bytes")
        zf.writestr(f"{top}/x64/Release/WinSparkle.lib", b"lib-bytes")
        zf.writestr(f"{top}/ARM64/Release/WinSparkle.dll", b"dll-bytes-arm")
        zf.writestr(f"{top}/COPYING", "MIT\n")


class ExtractWinSparkleZipTest(unittest.TestCase):
    def test_strips_top_level_version_dir(self):
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp) / "winsparkle.zip"
            dest = Path(tmp) / "out"
            dest.mkdir()
            _make_release_zip(archive)

            git.extract_winsparkle_zip(archive, dest)

            self.assertTrue((dest / "include" / "winsparkle.h").exists())
            self.assertTrue(
                (dest / "x64" / "Release" / "WinSparkle.dll").exists()
            )
            self.assertTrue(
                (dest / "ARM64" / "Release" / "WinSparkle.dll").exists()
            )
            self.assertEqual((dest / "COPYING").read_text(), "MIT\n")
            self.assertFalse((dest / "WinSparkle-0.9.3").exists())

    def test_rejects_path_traversal(self):
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp) / "evil.zip"
            dest = Path(tmp) / "out"
            dest.mkdir()
            with zipfile.ZipFile(archive, "w") as zf:
                zf.writestr("WinSparkle-0.9.3/../../evil.txt", "boom")

            with self.assertRaises(RuntimeError):
                git.extract_winsparkle_zip(archive, dest)
            self.assertFalse((Path(tmp) / "evil.txt").exists())


class WinSparkleSetupModuleTest(unittest.TestCase):
    def test_validate_requires_windows(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            ctx = make_context(
                MockChromium(Path(chromium_tmp)),
                MockBrowserOSRoot(Path(root_tmp)),
                architecture="x64",
            )
            with mock.patch.object(git, "IS_WINDOWS", return_value=False):
                with self.assertRaises(ValidationError):
                    git.WinSparkleSetupModule().validate(ctx)
            with mock.patch.object(git, "IS_WINDOWS", return_value=True):
                git.WinSparkleSetupModule().validate(ctx)

    def test_execute_downloads_extracts_and_replaces_existing(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            chromium = MockChromium(Path(chromium_tmp))
            ctx = make_context(
                chromium, MockBrowserOSRoot(Path(root_tmp)), architecture="x64"
            )
            stale = chromium.with_winsparkle() / "stale.txt"
            stale.write_text("old contents")

            def fake_urlretrieve(url, dest):
                self.assertEqual(url, ctx.get_winsparkle_url())
                _make_release_zip(Path(dest))

            with mock.patch.object(
                git.urllib.request, "urlretrieve", side_effect=fake_urlretrieve
            ):
                git.WinSparkleSetupModule().execute(ctx)

            winsparkle_dir = ctx.get_winsparkle_dir()
            self.assertTrue(
                (winsparkle_dir / "include" / "winsparkle.h").exists()
            )
            self.assertFalse(stale.exists())
            self.assertFalse((winsparkle_dir / "winsparkle.zip").exists())


class WinSparkleContextTest(unittest.TestCase):
    def test_url_and_dir_helpers(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            chromium = MockChromium(Path(chromium_tmp))
            ctx = make_context(
                chromium, MockBrowserOSRoot(Path(root_tmp)), architecture="x64"
            )
            self.assertEqual(
                ctx.get_winsparkle_dir(),
                chromium.src / "third_party" / "winsparkle",
            )
            version = ctx.WINSPARKLE_VERSION
            self.assertEqual(
                ctx.get_winsparkle_url(),
                "https://github.com/vslavik/winsparkle/releases/download/"
                f"v{version}/WinSparkle-{version}.zip",
            )


if __name__ == "__main__":
    unittest.main()
