#!/usr/bin/env python3
"""Tests for Context app path resolution."""

import tempfile
import unittest
from pathlib import Path

from .context import Context


class GetAppPathTest(unittest.TestCase):
    def test_extensions_manifest_url_uses_dedicated_bundled_manifest(self):
        ctx = Context(
            chromium_src=Path("/nonexistent-src"),
            architecture="arm64",
            build_type="release",
        )

        self.assertEqual(
            ctx.get_extensions_manifest_url(),
            "https://cdn.browseros.com/extensions/bundled-manifest.xml",
        )

    def test_arch_build_ignores_stale_universal_app(self):
        # Regression: a leftover out/Default_universal app must never hijack
        # an arch-specific build's sign/package stages.
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src = Path(tmp)
            ctx = Context(
                chromium_src=chromium_src,
                architecture="arm64",
                build_type="release",
            )

            stale_universal = (
                chromium_src / "out" / "Default_universal" / ctx.BROWSEROS_APP_NAME
            )
            stale_universal.mkdir(parents=True)

            fresh_arm64 = chromium_src / ctx.out_dir / ctx.BROWSEROS_APP_NAME
            fresh_arm64.mkdir(parents=True, exist_ok=True)

            self.assertEqual(ctx.get_app_path(), fresh_arm64)

    def test_universal_architecture_resolves_universal_out_dir(self):
        ctx = Context(
            chromium_src=Path("/nonexistent-src"),
            architecture="universal",
            build_type="release",
        )

        expected = (
            Path("/nonexistent-src") / ctx.out_dir / ctx.BROWSEROS_APP_NAME
        )
        self.assertTrue(str(ctx.out_dir).endswith("Default_universal"))
        self.assertEqual(ctx.get_app_path(), expected)

    def test_fixed_app_path_short_circuits_resolution(self):
        ctx = Context(
            chromium_src=Path("/nonexistent-src"),
            architecture="arm64",
            build_type="release",
        )
        pinned = Path("/pinned") / ctx.BROWSEROS_APP_NAME
        ctx._fixed_app_path = pinned

        self.assertEqual(ctx.get_app_path(), pinned)


if __name__ == "__main__":
    unittest.main()
