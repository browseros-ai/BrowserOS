#!/usr/bin/env python3
"""Tests for release publish helpers."""

import unittest
from types import SimpleNamespace
from typing import cast

from ...common.context import Context
from ...common.products import get_product_descriptor
from .publish import _release_source_key


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
            {"filename": "BrowserClaw_v0.31.0_universal.dmg"},
        )

        self.assertEqual(
            key,
            "releases/browserclaw/0.31.0/macos/BrowserClaw_v0.31.0_universal.dmg",
        )

    def _ctx(self, product: str) -> Context:
        return cast(
            Context,
            SimpleNamespace(
                env=SimpleNamespace(r2_cdn_base_url="https://cdn.browseros.com"),
                product=get_product_descriptor(product),
            ),
        )


if __name__ == "__main__":
    unittest.main()
