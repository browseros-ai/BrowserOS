#!/usr/bin/env python3
"""Tests for version parsing and derivation."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bos_build.core.context import Context
from bos_build.lib.testing import MockBrowserOSRoot

from bos_build.lib import versions


class VersionsTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def test_chromium_version_parses_pin_file(self):
        (self.root / "CHROMIUM_VERSION").write_text(
            "MAJOR=148\nMINOR=0\nBUILD=7402\nPATCH=57\n"
        )
        version, parts = versions.load_chromium_version(self.root)
        self.assertEqual(version, "148.0.7402.57")
        self.assertEqual(parts["BUILD"], "7402")

    def test_chromium_version_missing_file_is_empty(self):
        self.assertEqual(versions.load_chromium_version(self.root), ("", {}))

    def test_build_offset(self):
        offset_file = self.root / "bos_build" / "config" / "BROWSEROS_BUILD_OFFSET"
        offset_file.parent.mkdir(parents=True)
        offset_file.write_text("80\n")
        self.assertEqual(versions.load_build_offset(self.root), "80")

    def test_semantic_version_patch_only_when_nonzero(self):
        res = self.root / "resources"
        res.mkdir()
        f = res / "BROWSEROS_VERSION"

        f.write_text(
            "BROWSEROS_MAJOR=0\nBROWSEROS_MINOR=31\nBROWSEROS_BUILD=0\nBROWSEROS_PATCH=0\n"
        )
        self.assertEqual(versions.load_semantic_version(self.root), "0.31.0")

        f.write_text(
            "BROWSEROS_MAJOR=0\nBROWSEROS_MINOR=31\nBROWSEROS_BUILD=2\nBROWSEROS_PATCH=0\n"
        )
        self.assertEqual(versions.load_semantic_version(self.root), "0.31.2")

        f.write_text(
            "BROWSEROS_MAJOR=0\nBROWSEROS_MINOR=31\nBROWSEROS_BUILD=2\nBROWSEROS_PATCH=5\n"
        )
        self.assertEqual(versions.load_semantic_version(self.root), "0.31.2.5")

    def test_browseros_chromium_version_adds_offset_to_build(self):
        parts = {"MAJOR": "148", "MINOR": "0", "BUILD": "7402", "PATCH": "57"}
        self.assertEqual(
            versions.derive_browseros_chromium_version(parts, "80"),
            "148.0.7482.57",
        )
        self.assertEqual(versions.derive_browseros_chromium_version({}, "80"), "")
        self.assertEqual(versions.derive_browseros_chromium_version(parts, ""), "")

    def test_browseros_version_parts_parsed_as_ints(self):
        res = self.root / "resources"
        res.mkdir(exist_ok=True)
        (res / "BROWSEROS_VERSION").write_text(
            "BROWSEROS_MAJOR=0\nBROWSEROS_MINOR=47\nBROWSEROS_BUILD=0\nBROWSEROS_PATCH=2\n"
        )
        self.assertEqual(
            versions.load_browseros_version_parts(self.root), (0, 47, 0, 2)
        )
        self.assertEqual(versions.load_browseros_version_parts(self.root / "x"), ())

    def test_update_feed_version_is_epoch_prefixed(self):
        # parity with main's update-feed scheme (PR #1496)
        self.assertEqual(
            versions.update_feed_version((0, 47, 0, 2)), "10000.0.47.0.2"
        )
        with self.assertRaises(ValueError):
            versions.update_feed_version(())


class ReleaseChromiumFloorTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.package = MockBrowserOSRoot(
            Path(tmp.name) / "main", chromium_version="155.0.8059.26"
        )
        self.floor = (
            self.package.root
            / "bos_build/config/BROWSEROS_RELEASE_MIN_CHROMIUM_VERSION"
        )
        self.floor.write_text("158.0.8127.18\n")

    def test_release_old_pin_rejected_without_a_source_checkout(self):
        with self.assertRaisesRegex(
            ValueError, "Release builds require Chromium >= 158.0.8127.18"
        ):
            Context(
                root_dir=self.package.root,
                chromium_src=self.package.root / "absent",
                build_type="release",
            )

    def test_release_external_pin_meets_main_owned_floor(self):
        external = MockBrowserOSRoot(
            self.package.root.parent / "external", chromium_version="158.0.8127.18"
        ).with_patch_store()
        ctx = Context(
            root_dir=self.package.root, patch_root=external.root, build_type="release"
        )
        self.assertEqual(ctx.chromium_version, "158.0.8127.18")

    def test_release_synced_internal_pin_meets_floor(self):
        (self.package.root / "CHROMIUM_VERSION").write_text(
            "MAJOR=158\nMINOR=0\nBUILD=8127\nPATCH=18\n"
        )
        self.assertEqual(
            Context(root_dir=self.package.root, build_type="release").chromium_version,
            "158.0.8127.18",
        )

    def test_debug_old_pin_remains_supported(self):
        self.assertEqual(
            Context(root_dir=self.package.root, build_type="debug").chromium_version,
            "155.0.8059.26",
        )

    def test_malformed_release_floor_is_rejected(self):
        for invalid in ("", "158", "158.0.8127.bad", "158.0.8127.18.1"):
            with self.subTest(invalid=invalid):
                self.floor.write_text(invalid)
                with self.assertRaisesRegex(
                    ValueError, "Invalid release Chromium floor"
                ):
                    Context(root_dir=self.package.root, build_type="release")

    def test_older_layout_without_policy_remains_supported(self):
        self.floor.unlink()
        self.assertEqual(
            Context(root_dir=self.package.root, build_type="release").chromium_version,
            "155.0.8059.26",
        )

    def test_artifact_factories_preserve_their_pin_without_build_validation(self):
        from bos_build.cli import release, ota, ext, release_feeds

        with (
            mock.patch.object(
                release, "get_package_root", return_value=self.package.root
            ),
            mock.patch.object(ext, "get_package_root", return_value=self.package.root),
            mock.patch.object(
                release_feeds, "get_package_root", return_value=self.package.root
            ),
            mock.patch.object(
                ota,
                "Context",
                side_effect=lambda **kwargs: Context(
                    root_dir=self.package.root, **kwargs
                ),
            ),
        ):
            contexts = (
                release.create_release_context("0.52.2"),
                ota.create_ota_context(),
                ext._create_context("0.52.2"),
                release_feeds._create_context("0.52.2"),
            )
        for ctx in contexts:
            self.assertTrue(ctx.artifact_only)
            self.assertEqual(ctx.build_type, "release")
            self.assertEqual(ctx.chromium_version, "155.0.8059.26")

    def test_full_version_comparison_is_numeric(self):
        versions.validate_release_chromium_version(
            self.package.root, "158.0.9000.0", "release"
        )
        with self.assertRaisesRegex(ValueError, "Release builds require"):
            versions.validate_release_chromium_version(
                self.package.root, "158.0.8127.17", "release"
            )


if __name__ == "__main__":
    unittest.main()
