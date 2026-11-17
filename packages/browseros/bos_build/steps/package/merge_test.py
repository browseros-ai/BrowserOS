#!/usr/bin/env python3
"""Tests for universal merge output handling."""

import tempfile
import unittest
from pathlib import Path

from .merge import merge_architectures


def _make_app(root: Path, name: str) -> Path:
    app = root / name
    (app / "Contents" / "MacOS").mkdir(parents=True)
    (app / "Contents" / "MacOS" / "BrowserOS").write_bytes(b"\x00")
    return app


class MergeArchitecturesTest(unittest.TestCase):
    def test_failure_preserves_existing_output(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp = Path(tmp_dir)
            arch1 = _make_app(tmp, "BrowserOS-arm64.app")
            arch2 = _make_app(tmp, "BrowserOS-x64.app")
            out = tmp / "BrowserOS.app"
            out.mkdir()
            marker = out / "previous-universal-build.marker"
            marker.write_text("last good universal app")

            stub = tmp / "stub_universalizer.py"
            stub.write_text("import sys\nsys.exit(3)\n")

            ok = merge_architectures(arch1, arch2, out, universalizer_script=stub)

            self.assertFalse(ok)
            self.assertTrue(marker.exists())
            self.assertFalse((tmp / "BrowserOS.app.tmp").exists())

    def test_success_replaces_existing_output(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp = Path(tmp_dir)
            arch1 = _make_app(tmp, "BrowserOS-arm64.app")
            arch2 = _make_app(tmp, "BrowserOS-x64.app")
            out = tmp / "BrowserOS.app"
            out.mkdir()
            (out / "stale.marker").write_text("stale")

            stub = tmp / "stub_universalizer.py"
            stub.write_text(
                "import shutil, sys\n"
                "shutil.copytree(sys.argv[1], sys.argv[3])\n"
            )

            ok = merge_architectures(arch1, arch2, out, universalizer_script=stub)

            self.assertTrue(ok)
            self.assertTrue(
                (out / "Contents" / "MacOS" / "BrowserOS").exists()
            )
            self.assertFalse((out / "stale.marker").exists())
            self.assertFalse((tmp / "BrowserOS.app.tmp").exists())


if __name__ == "__main__":
    unittest.main()
