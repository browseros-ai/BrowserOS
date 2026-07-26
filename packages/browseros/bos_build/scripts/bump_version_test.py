#!/usr/bin/env python3
"""Tests for the BrowserOS CI version bump script."""

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("bump_version.py")


def _load_bump_version_module():
    spec = importlib.util.spec_from_file_location("bump_version", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Could not load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BumpVersionTest(unittest.TestCase):
    def setUp(self) -> None:
        self.module = _load_bump_version_module()
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "bos_build" / "config").mkdir(parents=True)
        (self.root / "resources").mkdir()
        self.offset_file = self.root / "bos_build" / "config" / "BROWSEROS_BUILD_OFFSET"
        self.version_file = self.root / "resources" / "BROWSEROS_VERSION"
        self.offset_file.write_text("146\n")
        self.version_file.write_text(
            "\n".join(
                [
                    "BROWSEROS_MAJOR=0",
                    "BROWSEROS_MINOR=46",
                    "BROWSEROS_BUILD=0",
                    "BROWSEROS_PATCH=0",
                    "",
                ]
            )
        )

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_offset_and_build_bumps_internal_and_semantic_versions(self) -> None:
        version = self.module.bump_version(self.root, "offset+build")

        self.assertEqual(version, "0.46.1")
        self.assertEqual(self.offset_file.read_text(), "147\n")
        self.assertIn("BROWSEROS_BUILD=1\n", self.version_file.read_text())
        self.assertIn("BROWSEROS_PATCH=0\n", self.version_file.read_text())

    def test_offset_and_build_resets_stale_patch(self) -> None:
        self.version_file.write_text(
            "\n".join(
                [
                    "BROWSEROS_MAJOR=0",
                    "BROWSEROS_MINOR=46",
                    "BROWSEROS_BUILD=2",
                    "BROWSEROS_PATCH=1",
                    "",
                ]
            )
        )

        version = self.module.bump_version(self.root, "offset+build")

        self.assertEqual(version, "0.46.3")
        self.assertEqual(self.offset_file.read_text(), "147\n")
        self.assertIn("BROWSEROS_BUILD=3\n", self.version_file.read_text())
        self.assertIn("BROWSEROS_PATCH=0\n", self.version_file.read_text())

    def test_offset_only_preserves_semantic_version(self) -> None:
        version = self.module.bump_version(self.root, "offset-only")

        self.assertEqual(version, "0.46.0")
        self.assertEqual(self.offset_file.read_text(), "147\n")
        self.assertIn("BROWSEROS_BUILD=0\n", self.version_file.read_text())

    def test_offset_and_patch_outputs_four_part_version(self) -> None:
        self.version_file.write_text(
            "\n".join(
                [
                    "BROWSEROS_MAJOR=0",
                    "BROWSEROS_MINOR=46",
                    "BROWSEROS_BUILD=2",
                    "BROWSEROS_PATCH=0",
                    "",
                ]
            )
        )

        version = self.module.bump_version(self.root, "offset+patch")

        self.assertEqual(version, "0.46.2.1")
        self.assertEqual(self.offset_file.read_text(), "147\n")
        self.assertIn("BROWSEROS_PATCH=1\n", self.version_file.read_text())

    def test_none_only_reads_current_version(self) -> None:
        version = self.module.bump_version(self.root, "none")

        self.assertEqual(version, "0.46.0")
        self.assertEqual(self.offset_file.read_text(), "146\n")
        self.assertIn("BROWSEROS_BUILD=0\n", self.version_file.read_text())

    def test_malformed_version_does_not_write_offset(self) -> None:
        self.version_file.write_text("BROWSEROS_MAJOR=0\n")

        with self.assertRaisesRegex(ValueError, "Missing BROWSEROS_BUILD"):
            self.module.bump_version(self.root, "offset+build")

        self.assertEqual(self.offset_file.read_text(), "146\n")

    def test_version_write_failure_does_not_write_offset(self) -> None:
        self.version_file.chmod(0o400)

        try:
            with self.assertRaises(OSError):
                self.module.bump_version(self.root, "offset+build")
        finally:
            self.version_file.chmod(0o600)

        self.assertEqual(self.offset_file.read_text(), "146\n")


if __name__ == "__main__":
    unittest.main()
