#!/usr/bin/env python3
"""Tests for release publish helpers."""

import unittest
from types import SimpleNamespace
from typing import cast
from unittest import mock

from ..core.context import Context
from ..core.products import get_product_descriptor
from .publish import PublishModule, _release_source_key


class ReleaseSourceKeyTest(unittest.TestCase):
    def test_uses_metadata_url_when_it_points_at_cdn(self):
        ctx = self._ctx("browseros")

        key = _release_source_key(
            ctx,
            "win",
            "0.31.0",
            {
                "filename": "BrowserOS_v0.31.0_x64_installer.exe",
                "url": "https://cdn.browseros.com/releases/0.31.0/win/BrowserOS_v0.31.0_x64_installer.exe",
            },
        )

        self.assertEqual(
            key,
            "releases/0.31.0/win/BrowserOS_v0.31.0_x64_installer.exe",
        )

    def test_falls_back_to_product_prefixed_release_path(self):
        ctx = self._ctx("browserclaw")

        key = _release_source_key(
            ctx,
            "macos",
            "0.31.0",
            {"filename": "BrowserOS_neo_v0.31.0_universal.dmg"},
        )

        self.assertEqual(
            key,
            "releases/browserclaw/0.31.0/macos/BrowserOS_neo_v0.31.0_universal.dmg",
        )

    def _ctx(self, product: str) -> Context:
        return cast(
            Context,
            SimpleNamespace(
                env=SimpleNamespace(r2_cdn_base_url="https://cdn.browseros.com"),
                product=get_product_descriptor(product),
            ),
        )


class PublishModuleIntegrityTest(unittest.TestCase):
    def test_missing_r2_client_raises(self):
        module = PublishModule(platforms=["win"])
        ctx = SimpleNamespace(
            release_version="0.49.0",
            env=SimpleNamespace(
                r2_cdn_base_url="https://cdn.browseros.com",
                r2_bucket="bucket",
            ),
            product=get_product_descriptor("browserclaw"),
        )

        with (
            mock.patch(
                "bos_build.release.publish.fetch_all_release_metadata",
                return_value={
                    "win": {
                        "artifacts": {
                            "x64_installer": {
                                "filename": "BrowserOS_neo_installer.exe",
                                "url": "https://cdn.browseros.com/installer.exe",
                            }
                        }
                    }
                },
            ),
            mock.patch(
                "bos_build.release.publish.get_r2_client",
                return_value=None,
            ),
            self.assertRaisesRegex(RuntimeError, "R2 client"),
        ):
            module.execute(ctx)

    def test_zero_mapped_artifacts_raises(self):
        module = PublishModule(platforms=["win"])
        ctx = SimpleNamespace(
            release_version="0.49.0",
            env=SimpleNamespace(
                r2_bucket="bucket",
                r2_cdn_base_url="https://cdn.browseros.com",
            ),
            product=get_product_descriptor("browserclaw"),
        )
        metadata = {
            "win": {
                "artifacts": {
                    "unknown": {
                        "filename": "unknown.bin",
                        "url": "https://cdn.browseros.com/unknown.bin",
                    }
                }
            }
        }

        with (
            mock.patch(
                "bos_build.release.publish.fetch_all_release_metadata",
                return_value=metadata,
            ),
            mock.patch(
                "bos_build.release.publish.get_r2_client",
                return_value=object(),
            ),
            self.assertRaisesRegex(RuntimeError, "No promotable artifacts"),
        ):
            module.execute(ctx)

    def test_each_requested_platform_requires_a_promotable_artifact(self):
        module = PublishModule(platforms=["macos", "win"])
        ctx = SimpleNamespace(
            release_version="0.49.0",
            env=SimpleNamespace(
                r2_bucket="bucket",
                r2_cdn_base_url="https://cdn.browseros.com",
            ),
            product=get_product_descriptor("browserclaw"),
        )
        metadata = {
            "macos": {
                "artifacts": {
                    "universal": {
                        "filename": "BrowserOS_neo_v0.49.0_universal.dmg",
                        "url": (
                            "https://cdn.browseros.com/releases/browserclaw/"
                            "0.49.0/macos/BrowserOS_neo_v0.49.0_universal.dmg"
                        ),
                    }
                }
            },
            "win": {
                "artifacts": {
                    "unknown": {
                        "filename": "unknown.bin",
                        "url": "https://cdn.browseros.com/unknown.bin",
                    }
                }
            },
        }

        with (
            mock.patch(
                "bos_build.release.publish.fetch_all_release_metadata",
                return_value=metadata,
            ),
            mock.patch(
                "bos_build.release.publish.get_r2_client",
                return_value=object(),
            ),
            mock.patch(
                "bos_build.release.publish.copy_to_download_path",
                return_value=True,
            ) as copy,
            self.assertRaisesRegex(RuntimeError, "requested platform win"),
        ):
            module.execute(ctx)

        copy.assert_not_called()


if __name__ == "__main__":
    unittest.main()
