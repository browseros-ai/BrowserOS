#!/usr/bin/env python3
"""Tests for release common helpers."""

import unittest
from types import SimpleNamespace
from typing import cast
from unittest import mock

from ..core.products import get_product_descriptor
from ..lib.env import EnvConfig
from . import common
from .common import (
    generate_release_notes,
    generate_appcast_item,
    get_download_path_mapping,
    list_all_versions,
    list_legacy_versions,
    validate_release_metadata,
)

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


def _release_metadata(
    platform: str,
    artifact_keys: set[str],
    *,
    product: str = "browserclaw",
    version: str = "0.49.0",
    source_sha: str = "a" * 40,
    run_id: str = "123",
    run_attempt: str = "1",
) -> dict:
    prefix = "BrowserOS_neo" if product == "browserclaw" else "BrowserOS"
    return {
        "product": product,
        "version": version,
        "platform": platform,
        "source_sha": source_sha,
        "workflow_run_id": run_id,
        "workflow_run_attempt": run_attempt,
        "artifacts": {
            key: {
                "filename": f"{prefix}_{key}",
                "url": f"https://cdn.browseros.com/{prefix}_{key}",
            }
            for key in artifact_keys
        },
    }


class ReleaseContractTest(unittest.TestCase):
    def setUp(self) -> None:
        self.metadata = {
            "macos": _release_metadata(
                "macos",
                {"arm64", "x64", "universal"},
            ),
            "win": _release_metadata(
                "win",
                {"x64_installer", "x64_zip"},
            ),
            "linux": _release_metadata(
                "linux",
                {"x64_appimage", "x64_deb"},
            ),
        }

    def test_all_universal_contract_selects_exactly_seven_current_run_assets(self):
        selected = validate_release_metadata(
            self.metadata,
            version="0.49.0",
            product_id="browserclaw",
            platforms="all",
            macos_arch="universal",
            source_sha="a" * 40,
            workflow_run_id="123",
            workflow_run_attempt="1",
        )

        self.assertEqual(set(selected), {"macos", "win", "linux"})
        self.assertEqual(
            sum(len(release["artifacts"]) for release in selected.values()),
            7,
        )

    def test_stale_run_provenance_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "workflow_run_id"):
            validate_release_metadata(
                self.metadata,
                version="0.49.0",
                product_id="browserclaw",
                platforms="all",
                macos_arch="universal",
                source_sha="a" * 40,
                workflow_run_id="999",
                workflow_run_attempt="1",
            )

    def test_one_run_accepts_platforms_from_different_rerun_attempts(self):
        self.metadata["win"]["workflow_run_attempt"] = "2"

        selected = validate_release_metadata(
            self.metadata,
            version="0.49.0",
            product_id="browserclaw",
            platforms="all",
            macos_arch="universal",
            source_sha="a" * 40,
            workflow_run_id="123",
        )

        self.assertEqual(selected["linux"]["workflow_run_attempt"], "1")
        self.assertEqual(selected["win"]["workflow_run_attempt"], "2")

    def test_missing_or_extra_artifact_keys_are_rejected(self):
        del self.metadata["win"]["artifacts"]["x64_zip"]
        self.metadata["win"]["artifacts"]["arm64_zip"] = {
            "filename": "stale.zip",
            "url": "https://cdn.browseros.com/stale.zip",
        }

        with self.assertRaisesRegex(RuntimeError, "artifact keys"):
            validate_release_metadata(
                self.metadata,
                version="0.49.0",
                product_id="browserclaw",
                platforms="all",
                macos_arch="universal",
            )

    def test_partial_platform_contract_excludes_stale_other_platforms(self):
        selected = validate_release_metadata(
            self.metadata,
            version="0.49.0",
            product_id="browserclaw",
            platforms="windows",
            macos_arch="universal",
        )

        self.assertEqual(set(selected), {"win"})

    def test_release_notes_use_product_display_name(self):
        notes = generate_release_notes(
            "0.49.0",
            {"win": self.metadata["win"]},
            get_product_descriptor("browserclaw"),
        )

        self.assertIn("## BrowserOS neo v0.49.0", notes)
        self.assertNotIn("## BrowserOS v", notes)


# Golden copy of the pre-productization DOWNLOAD_PATH_MAPPING constant —
# get_download_path_mapping(browseros) must stay byte-identical to it.
BROWSEROS_DOWNLOAD_GOLDEN = {
    "macos": {
        "arm64": "download/BrowserOS-arm64.dmg",
        "x64": "download/BrowserOS-x86_64.dmg",
        "universal": "download/BrowserOS.dmg",
    },
    "win": {
        "x64_installer": "download/BrowserOS_installer.exe",
    },
    "linux": {
        "x64_appimage": "download/BrowserOS.AppImage",
        "x64_deb": "download/BrowserOS.deb",
        "arm64_appimage": "download/BrowserOS-arm64.AppImage",
        "arm64_deb": "download/BrowserOS-arm64.deb",
    },
}

BROWSERCLAW_DOWNLOAD_GOLDEN = {
    "macos": {
        "arm64": "download/BrowserOS_neo-arm64.dmg",
        "x64": "download/BrowserOS_neo-x86_64.dmg",
        "universal": "download/BrowserOS_neo.dmg",
    },
    "win": {
        "x64_installer": "download/BrowserOS_neo_installer.exe",
    },
    "linux": {
        "x64_appimage": "download/BrowserOS_neo.AppImage",
        "x64_deb": "download/BrowserOS_neo.deb",
        "arm64_appimage": "download/BrowserOS_neo-arm64.AppImage",
        "arm64_deb": "download/BrowserOS_neo-arm64.deb",
    },
}


class DownloadPathMappingTest(unittest.TestCase):
    def test_browseros_mapping_matches_golden_constant(self):
        self.assertEqual(
            get_download_path_mapping(get_product_descriptor("browseros")),
            BROWSEROS_DOWNLOAD_GOLDEN,
        )

    def test_default_product_mapping_is_browseros(self):
        self.assertEqual(get_download_path_mapping(), BROWSEROS_DOWNLOAD_GOLDEN)

    def test_browserclaw_mapping_fully_prefixed(self):
        self.assertEqual(
            get_download_path_mapping(get_product_descriptor("browserclaw")),
            BROWSERCLAW_DOWNLOAD_GOLDEN,
        )


class _FakeR2Client:
    """Serves canned delimiter-listing pages; records call kwargs."""

    def __init__(self, pages_by_prefix):
        self.pages_by_prefix = pages_by_prefix
        self.calls = []

    def list_objects_v2(self, **kwargs):
        self.calls.append(kwargs)
        pages = self.pages_by_prefix[kwargs["Prefix"]]
        return pages[int(kwargs.get("ContinuationToken", 0))]


def _page(prefix, names, next_token=None):
    page: dict = {"CommonPrefixes": [{"Prefix": f"{prefix}{name}/"} for name in names]}
    if next_token is not None:
        page["IsTruncated"] = True
        page["NextContinuationToken"] = str(next_token)
    return page


def _env(configured=True):
    return cast(
        EnvConfig,
        SimpleNamespace(r2_bucket="bucket", has_r2_config=lambda: configured),
    )


class ListAllVersionsTest(unittest.TestCase):
    def test_lists_productized_versions_newest_first(self):
        prefix = "releases/browseros/"
        client = _FakeR2Client({prefix: [_page(prefix, ["0.30.0", "0.31.0"])]})

        with mock.patch.object(common, "get_r2_client", return_value=client):
            versions = list_all_versions("browseros", _env())

        self.assertEqual(versions, ["0.31.0", "0.30.0"])

    def test_paginates_truncated_listings(self):
        prefix = "releases/browseros/"
        client = _FakeR2Client(
            {
                prefix: [
                    _page(prefix, ["0.29.0"], next_token=1),
                    _page(prefix, ["0.31.0"]),
                ]
            }
        )

        with mock.patch.object(common, "get_r2_client", return_value=client):
            versions = list_all_versions("browseros", _env())

        self.assertEqual(versions, ["0.31.0", "0.29.0"])

    def test_requests_product_release_prefix_with_delimiter(self):
        prefix = "releases/browserclaw/"
        client = _FakeR2Client({prefix: [_page(prefix, [])]})

        with mock.patch.object(common, "get_r2_client", return_value=client):
            list_all_versions("browserclaw", _env())

        self.assertEqual(
            client.calls[0],
            {"Bucket": "bucket", "Prefix": prefix, "Delimiter": "/"},
        )

    def test_returns_empty_without_r2_config(self):
        self.assertEqual(list_all_versions("browseros", _env(configured=False)), [])


class ListLegacyVersionsTest(unittest.TestCase):
    def test_keeps_only_version_shaped_prefixes(self):
        client = _FakeR2Client(
            {
                "releases/": [
                    _page("releases/", ["browseros", "browserclaw", "0.28.4", "0.29.0"])
                ]
            }
        )

        with mock.patch.object(common, "get_r2_client", return_value=client):
            versions = list_legacy_versions(_env())

        self.assertEqual(versions, ["0.29.0", "0.28.4"])

    def test_rejects_decorated_version_names(self):
        client = _FakeR2Client(
            {"releases/": [_page("releases/", ["v0.31.0", "0.31.0-beta", "1.2.3.4"])]}
        )

        with mock.patch.object(common, "get_r2_client", return_value=client):
            versions = list_legacy_versions(_env())

        self.assertEqual(versions, ["1.2.3.4"])

    def test_returns_empty_without_r2_config(self):
        self.assertEqual(list_legacy_versions(_env(configured=False)), [])


if __name__ == "__main__":
    unittest.main()
