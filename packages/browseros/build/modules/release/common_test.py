#!/usr/bin/env python3
"""Tests for appcast item generation."""

import unittest

from ...common.products import get_product_descriptor
from .common import generate_appcast_item, get_download_path_mapping

ARTIFACT = {
    "url": "https://cdn.browseros.com/releases/0.31.0/win/BrowserOS_v0.31.0_x64_installer.exe",
    "sparkle_signature": "c2lnbmF0dXJl",
    "sparkle_length": 12345,
}


class GenerateAppcastItemTest(unittest.TestCase):
    def test_windows_item_has_os_attr_and_no_min_system_version(self):
        item = generate_appcast_item(
            ARTIFACT, "0.31.0", "7778.97", "2026-06-11T00:00:00Z", platform="win"
        )
        self.assertIn('sparkle:os="windows"', item)
        self.assertIn('sparkle:edSignature="c2lnbmF0dXJl"', item)
        self.assertIn('length="12345"', item)
        self.assertIn("<sparkle:version>7778.97</sparkle:version>", item)
        self.assertIn(
            "<sparkle:shortVersionString>0.31.0</sparkle:shortVersionString>", item
        )
        self.assertNotIn("minimumSystemVersion", item)

    def test_macos_item_unchanged_by_default(self):
        item = generate_appcast_item(
            ARTIFACT, "0.31.0", "7778.97", "2026-06-11T00:00:00Z"
        )
        self.assertIn(
            "<sparkle:minimumSystemVersion>10.15</sparkle:minimumSystemVersion>",
            item,
        )
        self.assertNotIn("sparkle:os=", item)


class DownloadPathMappingTest(unittest.TestCase):
    def test_browserclaw_mapping_uses_product_artifact_prefix(self):
        mapping = get_download_path_mapping(get_product_descriptor("browserclaw"))

        self.assertEqual(
            mapping["macos"]["universal"],
            "download/BrowserClaw.dmg",
        )
        self.assertEqual(
            mapping["win"]["x64_installer"],
            "download/BrowserClaw_installer.exe",
        )


if __name__ == "__main__":
    unittest.main()
