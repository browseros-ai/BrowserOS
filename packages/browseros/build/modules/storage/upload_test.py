#!/usr/bin/env python3
"""Tests for release artifact upload metadata helpers."""

import unittest

from build.modules.storage.upload import _get_artifact_key, merge_release_metadata


class UploadMetadataTest(unittest.TestCase):
    def test_linux_x64_artifacts_use_x64_keys(self) -> None:
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_x64.AppImage", "linux"),
            "x64_appimage",
        )
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_amd64.deb", "linux"),
            "x64_deb",
        )

    def test_linux_arm64_artifacts_use_arm64_keys(self) -> None:
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_arm64.AppImage", "linux"),
            "arm64_appimage",
        )
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_arm64.deb", "linux"),
            "arm64_deb",
        )
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_aarch64.deb", "linux"),
            "arm64_deb",
        )

    def test_merge_release_metadata_preserves_existing_artifacts(self) -> None:
        existing = {
            "platform": "linux",
            "version": "1.2.3",
            "build_date": "old",
            "artifacts": {
                "x64_appimage": {"filename": "BrowserOS_v1.2.3_x64.AppImage"},
                "x64_deb": {"filename": "BrowserOS_v1.2.3_amd64.deb"},
            },
        }
        new = {
            "platform": "linux",
            "version": "1.2.3",
            "build_date": "new",
            "artifacts": {
                "arm64_appimage": {"filename": "BrowserOS_v1.2.3_arm64.AppImage"},
                "arm64_deb": {"filename": "BrowserOS_v1.2.3_arm64.deb"},
            },
        }

        merged = merge_release_metadata(existing, new)

        self.assertEqual(merged["build_date"], "new")
        self.assertEqual(
            sorted(merged["artifacts"]),
            ["arm64_appimage", "arm64_deb", "x64_appimage", "x64_deb"],
        )

    def test_merge_release_metadata_overwrites_matching_artifact_keys(self) -> None:
        existing = {
            "platform": "linux",
            "version": "1.2.3",
            "artifacts": {
                "x64_appimage": {"filename": "old.AppImage", "size": 1},
            },
        }
        new = {
            "platform": "linux",
            "version": "1.2.3",
            "artifacts": {
                "x64_appimage": {"filename": "new.AppImage", "size": 2},
            },
        }

        merged = merge_release_metadata(existing, new)

        self.assertEqual(merged["artifacts"]["x64_appimage"]["filename"], "new.AppImage")
        self.assertEqual(merged["artifacts"]["x64_appimage"]["size"], 2)


if __name__ == "__main__":
    unittest.main()
