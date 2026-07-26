#!/usr/bin/env python3
"""Tests for the update-feed spec table."""

import unittest

from ...core.products import (
    BROWSEROS_AGENT_EXTENSION_ID,
    BROWSEROS_BUG_REPORTER_EXTENSION_ID,
    BROWSERCLAW_EXTENSION_ID,
)
from .spec import (
    CDN_BASE_URL,
    EXTENSIONS,
    all_feeds,
    browser_feeds_for_product,
    bundled_manifest_feed,
    extension_by_name,
    extensions_json_feed,
    feed_by_key,
    server_feed,
    update_manifest_feed,
)

# Every key a shipped client polls (chromium_patches ground truth).
CLIENT_POLLED_KEYS = [
    "appcast.xml",
    "appcast-x86_64.xml",
    "appcast-win.xml",
    "appcast-win-arm64.xml",
    "appcast-server.xml",
    "appcast-server.alpha.xml",
    "appcast-claw-server.xml",
    "appcast-claw-server.alpha.xml",
    "extensions/update-manifest.xml",
    "extensions/update-manifest.alpha.xml",
    "extensions/extensions.json",
    "extensions/extensions.alpha.json",
    "extensions/bundled-manifest.xml",
]


class FeedTableTest(unittest.TestCase):
    def test_every_client_polled_key_exists_exactly_once(self):
        keys = [feed.key for feed in all_feeds()]
        for key in CLIENT_POLLED_KEYS:
            self.assertEqual(keys.count(key), 1, key)

    def test_no_key_appears_twice(self):
        keys = [feed.key for feed in all_feeds()]
        self.assertEqual(len(keys), len(set(keys)))

    def test_browserclaw_browser_feeds_are_publishable(self):
        claw_feeds = browser_feeds_for_product("browserclaw")
        self.assertEqual(
            [feed.key for feed in claw_feeds],
            [
                "appcast-claw.xml",
                "appcast-claw-x86_64.xml",
                "appcast-claw-win.xml",
                "appcast-claw-win-arm64.xml",
            ],
        )
        # Publishable since the product-aware URL chromium patch: both
        # sparkle_glue.mm and winsparkle_glue.cc select the claw feed.
        self.assertTrue(all(feed.publishable for feed in claw_feeds))

    def test_browseros_browser_feeds_are_publishable(self):
        feeds = browser_feeds_for_product("browseros")
        self.assertTrue(all(feed.publishable for feed in feeds))

    def test_browseros_browser_feed_derivation(self):
        by_key = {feed.key: feed for feed in browser_feeds_for_product("browseros")}

        mac = by_key["appcast.xml"]
        self.assertEqual(mac.platform, "macos")
        self.assertEqual(mac.artifact_keys, ("universal", "arm64"))
        self.assertEqual(mac.title, "BrowserOS")

        mac_x64 = by_key["appcast-x86_64.xml"]
        self.assertEqual(mac_x64.artifact_keys, ("x64", "universal"))

        win = by_key["appcast-win.xml"]
        self.assertEqual(win.platform, "win")
        self.assertEqual(win.artifact_keys, ("x64_installer",))
        self.assertEqual(win.title, "BrowserOS Windows Updates")

        win_arm = by_key["appcast-win-arm64.xml"]
        self.assertEqual(win_arm.artifact_keys, ("arm64_installer",))

    def test_feed_links_point_at_their_own_key(self):
        for feed in all_feeds():
            if feed.kind in ("browser", "server"):
                self.assertEqual(feed.link, f"{CDN_BASE_URL}/{feed.key}", feed.key)

    def test_server_feed_lookup_and_titles(self):
        prod = server_feed("browseros-server", "prod")
        self.assertEqual(prod.key, "appcast-server.xml")
        self.assertEqual(prod.title, "BrowserOS Server")
        self.assertEqual(prod.product, "browseros")

        alpha = server_feed("browserclaw-server", "alpha")
        self.assertEqual(alpha.key, "appcast-claw-server.alpha.xml")
        self.assertEqual(alpha.title, "BrowserOS Claw Server (Alpha)")
        self.assertEqual(alpha.bundle_id, "browserclaw-server")

    def test_extension_feed_lookups(self):
        self.assertEqual(
            update_manifest_feed("prod").key, "extensions/update-manifest.xml"
        )
        self.assertEqual(
            update_manifest_feed("alpha").key, "extensions/update-manifest.alpha.xml"
        )
        self.assertEqual(
            extensions_json_feed("alpha").key, "extensions/extensions.alpha.json"
        )
        self.assertEqual(
            bundled_manifest_feed().key, "extensions/bundled-manifest.xml"
        )

    def test_feed_by_key_round_trips_every_key(self):
        for feed in all_feeds():
            self.assertIs(feed_by_key(feed.key), feed)

    def test_feed_by_key_unknown_raises(self):
        with self.assertRaises(ValueError):
            feed_by_key("appcast-nope.xml")


class ExtensionRegistryTest(unittest.TestCase):
    def test_registry_matches_product_constants(self):
        by_name = {ext.name: ext for ext in EXTENSIONS}
        self.assertEqual(
            by_name["agent"].extension_id, BROWSEROS_AGENT_EXTENSION_ID
        )
        self.assertEqual(
            by_name["bugreporter"].extension_id,
            BROWSEROS_BUG_REPORTER_EXTENSION_ID,
        )
        self.assertEqual(
            by_name["browserclaw"].extension_id, BROWSERCLAW_EXTENSION_ID
        )

    def test_update_feed_membership_mirrors_live(self):
        by_name = {ext.name: ext for ext in EXTENSIONS}
        self.assertTrue(by_name["agent"].in_update_feed)
        self.assertTrue(by_name["bugreporter"].in_update_feed)
        self.assertTrue(by_name["browserclaw"].in_update_feed)

    def test_crx_url_scheme(self):
        agent = extension_by_name("agent")
        self.assertEqual(
            agent.crx_url("0.0.118.0"),
            "https://cdn.browseros.com/extensions/agent-0.0.118.0.crx",
        )
        self.assertEqual(agent.crx_key("0.0.118.0"), "extensions/agent-0.0.118.0.crx")

    def test_unknown_extension_name_raises_listing_valid_names(self):
        with self.assertRaisesRegex(ValueError, "agent.*browserclaw.*bugreporter"):
            extension_by_name("nope")


if __name__ == "__main__":
    unittest.main()
