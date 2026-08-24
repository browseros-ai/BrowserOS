#!/usr/bin/env python3
"""Source-resource catalog tests."""

import unittest

from bos_build.products.resource_sources import source_resources_for_product


class SourceResourceCatalogTest(unittest.TestCase):
    def test_browseros_source_resources(self) -> None:
        source = source_resources_for_product("browseros")

        self.assertEqual(source.server_component, "server")
        self.assertEqual(source.extension_component, "agent")
        self.assertEqual(source.extension_name, "agent")
        self.assertEqual(source.onboarding_component, "app-onboard")
        self.assertEqual(source.external_extension_names, ("bugreporter",))

    def test_browserclaw_source_resources(self) -> None:
        source = source_resources_for_product("browserclaw")

        self.assertEqual(source.server_component, "claw-server-rust")
        self.assertEqual(source.extension_component, "browserclaw")
        self.assertEqual(source.extension_name, "browserclaw")
        self.assertEqual(source.onboarding_component, "claw-onboard")
        self.assertEqual(source.external_extension_names, ("bugreporter",))

    def test_unknown_product_fails(self) -> None:
        with self.assertRaisesRegex(ValueError, "Unknown product"):
            source_resources_for_product("missing")


if __name__ == "__main__":
    unittest.main()
