#!/usr/bin/env python3
"""Tests for Context app path resolution."""

import tempfile
import unittest
from pathlib import Path
from typing import cast
from unittest import mock

from . import context as context_mod
from .context import Context
from .products import ProductDescriptor, get_product_descriptor


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

    def test_bundle_local_extensions_defaults_off(self):
        ctx = Context(
            chromium_src=Path("/nonexistent-src"),
            architecture="arm64",
            build_type="release",
        )

        self.assertFalse(ctx.bundle_local_extensions)

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
                chromium_src
                / "out"
                / "Default_browseros_universal"
                / ctx.BROWSEROS_APP_NAME
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

        expected = Path("/nonexistent-src") / ctx.out_dir / ctx.BROWSEROS_APP_NAME
        self.assertTrue(str(ctx.out_dir).endswith("Default_browseros_universal"))
        self.assertEqual(ctx.get_app_path(), expected)

    def test_browserclaw_context_derives_names_and_paths(self):
        with (
            mock.patch.object(context_mod, "IS_MACOS", return_value=True),
            mock.patch.object(context_mod, "IS_WINDOWS", return_value=False),
        ):
            ctx = Context(
                chromium_src=Path("/nonexistent-src"),
                architecture="arm64",
                build_type="release",
                product=get_product_descriptor("browserclaw"),
            )

            self.assertEqual(ctx.BROWSEROS_APP_BASE_NAME, "BrowserClaw")
            self.assertEqual(ctx.BROWSEROS_APP_NAME, "BrowserClaw.app")
            self.assertEqual(ctx.out_dir, "out/Default_browserclaw_arm64")
            self.assertEqual(
                ctx.get_artifact_name("dmg"),
                f"BrowserClaw_v{ctx.semantic_version}_arm64.dmg",
            )
            self.assertEqual(
                ctx.get_release_path("macos"),
                f"releases/browserclaw/{ctx.semantic_version}/macos/",
            )

    def test_context_accepts_product_id(self):
        with (
            mock.patch.object(context_mod, "IS_MACOS", return_value=True),
            mock.patch.object(context_mod, "IS_WINDOWS", return_value=False),
        ):
            ctx = Context(
                chromium_src=Path("/nonexistent-src"),
                architecture="arm64",
                build_type="release",
                product=cast(ProductDescriptor, "browserclaw"),
            )

            self.assertEqual(ctx.product.id, "browserclaw")
            self.assertEqual(ctx.BROWSEROS_APP_NAME, "BrowserClaw.app")

    def test_context_accepts_product_id_string(self):
        ctx = Context(
            chromium_src=Path("/nonexistent-src"),
            architecture="arm64",
            build_type="release",
            product="browserclaw",
        )

        self.assertEqual(ctx.build_type, "release")
        self.assertEqual(ctx.product.id, "browserclaw")
        self.assertEqual(ctx.BROWSEROS_APP_BASE_NAME, "BrowserClaw")

    def test_debug_gn_args_allow_override_and_package_all(self):
        ctx = Context(
            chromium_src=Path("/nonexistent-src"),
            architecture="x64",
            build_type="debug",
        )

        self.assertEqual(
            ctx.get_product_gn_args(),
            [
                'browseros_product = "browseros"',
                "browseros_allow_runtime_product_override = true",
                "browseros_package_all_server_resources = true",
            ],
        )

    def test_release_gn_args_select_product_for_baked_identity(self):
        for product in ("browseros", "browserclaw"):
            with self.subTest(product=product):
                ctx = Context(
                    chromium_src=Path("/nonexistent-src"),
                    architecture="arm64",
                    build_type="release",
                    product=get_product_descriptor(product),
                )

                self.assertEqual(
                    ctx.get_product_gn_args(),
                    [
                        f'browseros_product = "{product}"',
                        "browseros_allow_runtime_product_override = false",
                        "browseros_package_all_server_resources = false",
                    ],
                )


if __name__ == "__main__":
    unittest.main()
