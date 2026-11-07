#!/usr/bin/env python3
"""Tests for product-specific icon generation paths."""

import unittest

from bos_build.scripts.icon_generation import generate_icons


class IconGenerationProductPathsTest(unittest.TestCase):
    def test_browserclaw_uses_product_source_and_resource_tree(self) -> None:
        self.assertEqual(
            generate_icons.resolve_input_path("source/app_icon.png", "browserclaw"),
            generate_icons.SOURCE_DIR / "browserclaw" / "app_icon.png",
        )
        self.assertEqual(
            generate_icons.resolve_input_path("static/product_logo.svg", "browserclaw"),
            generate_icons.STATIC_DIR / "browserclaw" / "product_logo.svg",
        )
        self.assertEqual(
            generate_icons.product_output_dir("browserclaw"),
            generate_icons.RESOURCE_DIR / "browserclaw" / "icons",
        )

    def test_unknown_product_is_rejected(self) -> None:
        # The original BrowserOS artwork moved to browseros-human.
        for product in ("chrome", "browseros"):
            with self.subTest(product=product):
                with self.assertRaisesRegex(ValueError, "Unknown product"):
                    generate_icons.product_roots(product)
                with self.assertRaisesRegex(ValueError, "Unknown product"):
                    generate_icons.product_output_dir(product)


if __name__ == "__main__":
    unittest.main()
