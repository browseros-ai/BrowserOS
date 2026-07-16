#!/usr/bin/env python3
"""Golden-output tests for the feed renderers."""

import unittest
from pathlib import Path

from .render import (
    ExistingAppcast,
    SignedArtifact,
    extract_appcast_version,
    extract_channel_metadata,
    extract_enclosure_urls,
    extract_manifest_versions,
    parse_dotted_version,
    parse_server_appcast_content,
    render_browser_appcast,
    render_extensions_json,
    render_server_appcast,
    render_update_manifest,
)
from .spec import feed_by_key, server_feed

MAC_ARTIFACT = {
    "filename": "BrowserOS_v0.47.0.2_arm64.dmg",
    "url": "https://cdn.browseros.com/releases/browseros/0.47.0.2/macos/BrowserOS_v0.47.0.2_arm64.dmg",
    "sparkle_signature": "MACSIG==",
    "sparkle_length": 265462841,
}

WIN_ARTIFACT = {
    "filename": "BrowserOS_v0.47.0.2_x64_installer.exe",
    "url": "https://cdn.browseros.com/releases/browseros/0.47.0.2/win/BrowserOS_v0.47.0.2_x64_installer.exe",
    "sparkle_signature": "WINSIG==",
    "sparkle_length": 190943800,
}

GOLDEN_MAC_APPCAST = """\
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>BrowserOS</title>
    <link>https://cdn.browseros.com/appcast.xml</link>
    <description>Most recent changes with links to updates.</description>
    <language>en</language>

    <item>
      <title>BrowserOS - 0.47.0.2</title>
      <description sparkle:format="plain-text">
      </description>
      <sparkle:version>10000.0.47.0.2</sparkle:version>
      <sparkle:shortVersionString>0.47.0.2</sparkle:shortVersionString>
      <pubDate>Fri, 19 Jun 2026 06:41:33 +0000</pubDate>
      <link>https://www.browseros.com/</link>
      <enclosure
        url="https://cdn.browseros.com/releases/browseros/0.47.0.2/macos/BrowserOS_v0.47.0.2_arm64.dmg"
        sparkle:edSignature="MACSIG=="
        length="265462841"
        type="application/octet-stream" />
      <sparkle:minimumSystemVersion>10.15</sparkle:minimumSystemVersion>
    </item>

  </channel>
</rss>
"""

GOLDEN_WIN_APPCAST = """\
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>BrowserOS Windows Updates</title>
    <link>https://cdn.browseros.com/appcast-win.xml</link>
    <description>Most recent changes with links to updates.</description>
    <language>en</language>

    <item>
      <title>BrowserOS - 0.47.0.2</title>
      <description sparkle:format="plain-text">
      </description>
      <sparkle:version>10000.0.47.0.2</sparkle:version>
      <sparkle:shortVersionString>0.47.0.2</sparkle:shortVersionString>
      <pubDate>Fri, 19 Jun 2026 06:41:33 +0000</pubDate>
      <link>https://www.browseros.com/</link>
      <enclosure
        url="https://cdn.browseros.com/releases/browseros/0.47.0.2/win/BrowserOS_v0.47.0.2_x64_installer.exe"
        sparkle:os="windows"
        sparkle:edSignature="WINSIG=="
        length="190943800"
        type="application/octet-stream" />
    </item>

  </channel>
</rss>
"""

GOLDEN_CLAW_SERVER_APPCAST = """\
<?xml version="1.0" encoding="utf-8"?>
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle" version="2.0">
  <channel>
    <title>BrowserOS Claw Server</title>
    <link>https://cdn.browseros.com/appcast-claw-server.xml</link>
    <description>BrowserOS Claw Server binary updates</description>
    <language>en</language>

    <item>
      <sparkle:version>0.0.5</sparkle:version>
      <pubDate>Wed, 01 Jul 2026 01:18:52 +0000</pubDate>

      <!-- macOS arm64 -->
      <enclosure
        url="https://cdn.browseros.com/server/browserclaw_server_0.0.5_darwin_arm64.zip"
        sparkle:os="macos"
        sparkle:arch="arm64"
        sparkle:edSignature="CLAWSIG=="
        length="123"
        type="application/zip"/>
    </item>

  </channel>
</rss>
"""

GOLDEN_UPDATE_MANIFEST = """\
<?xml version='1.0' encoding='UTF-8'?>
<gupdate xmlns="http://www.google.com/update2/response" protocol="2.0">
  <app appid="adlpneommgkgeanpaekgoaolcpncohkf">
    <updatecheck codebase="https://cdn.browseros.com/extensions/bugreporter-54.0.0.0.crx" version="54.0.0.0" />
  </app>
  <app appid="bflpfmnmnokmjhmgnolecpppdbdophmk">
    <updatecheck codebase="https://cdn.browseros.com/extensions/agent-0.0.118.0.crx" version="0.0.118.0" />
  </app>
</gupdate>
"""

GOLDEN_EXTENSIONS_JSON = """\
{
  "extensions": {
    "adlpneommgkgeanpaekgoaolcpncohkf": {
      "external_update_url": "https://cdn.browseros.com/extensions/update-manifest.alpha.xml"
    },
    "bflpfmnmnokmjhmgnolecpppdbdophmk": {
      "external_update_url": "https://cdn.browseros.com/extensions/update-manifest.alpha.xml"
    },
    "pjimfkbpehlcllblajnpfamdfjhhlgkc": {
      "external_update_url": "https://cdn.browseros.com/extensions/update-manifest.alpha.xml"
    }
  }
}
"""


class BrowserAppcastRenderTest(unittest.TestCase):
    def test_golden_mac_appcast(self):
        content = render_browser_appcast(
            feed_by_key("appcast.xml"),
            MAC_ARTIFACT,
            "0.47.0.2",
            "10000.0.47.0.2",
            "2026-06-19T06:41:33Z",
        )
        self.assertEqual(content, GOLDEN_MAC_APPCAST)

    def test_golden_win_appcast(self):
        content = render_browser_appcast(
            feed_by_key("appcast-win.xml"),
            WIN_ARTIFACT,
            "0.47.0.2",
            "10000.0.47.0.2",
            "2026-06-19T06:41:33Z",
        )
        self.assertEqual(content, GOLDEN_WIN_APPCAST)

    def test_missing_sparkle_signature_raises(self):
        artifact = {k: v for k, v in MAC_ARTIFACT.items() if k != "sparkle_signature"}
        with self.assertRaisesRegex(ValueError, "sparkle_signature"):
            render_browser_appcast(
                feed_by_key("appcast.xml"),
                artifact,
                "0.47.0.2",
                "10000.0.47.0.2",
                "2026-06-19T06:41:33Z",
            )

    def test_missing_length_raises(self):
        artifact = {
            k: v for k, v in MAC_ARTIFACT.items() if k != "sparkle_length"
        }
        with self.assertRaisesRegex(ValueError, "sparkle_length"):
            render_browser_appcast(
                feed_by_key("appcast.xml"),
                artifact,
                "0.47.0.2",
                "10000.0.47.0.2",
                "2026-06-19T06:41:33Z",
            )

    def test_missing_sparkle_version_raises(self):
        with self.assertRaisesRegex(ValueError, "sparkle_version"):
            render_browser_appcast(
                feed_by_key("appcast.xml"),
                MAC_ARTIFACT,
                "0.47.0.2",
                "",
                "2026-06-19T06:41:33Z",
            )


class ServerAppcastRenderTest(unittest.TestCase):
    def test_golden_claw_server_appcast_uses_spec_metadata(self):
        artifact = SignedArtifact(
            platform="darwin_arm64",
            zip_path=Path("browserclaw_server_0.0.5_darwin_arm64.zip"),
            signature="CLAWSIG==",
            length=123,
            os="macos",
            arch="arm64",
        )
        # Same-version existing pins the pubDate so the golden is deterministic.
        existing = ExistingAppcast(
            version="0.0.5",
            pub_date="Wed, 01 Jul 2026 01:18:52 +0000",
            artifacts={},
        )
        content = render_server_appcast(
            server_feed("browserclaw-server", "prod"), "0.0.5", [artifact], existing
        )
        self.assertEqual(content, GOLDEN_CLAW_SERVER_APPCAST)

    def test_same_version_merge_keeps_pub_date_and_other_platforms(self):
        spec = server_feed("browseros-server", "alpha")
        existing_artifact = SignedArtifact(
            platform="darwin_arm64",
            zip_path=Path("browseros_server_0.0.9_darwin_arm64.zip"),
            signature="OLDSIG==",
            length=11,
            os="macos",
            arch="arm64",
        )
        existing = ExistingAppcast(
            version="0.0.9",
            pub_date="Thu, 16 Apr 2026 18:58:59 +0000",
            artifacts={"darwin_arm64": existing_artifact},
        )
        new_artifact = SignedArtifact(
            platform="linux_x64",
            zip_path=Path("browseros_server_0.0.9_linux_x64.zip"),
            signature="NEWSIG==",
            length=22,
            os="linux",
            arch="x86_64",
        )

        content = render_server_appcast(spec, "0.0.9", [new_artifact], existing)

        self.assertIn("Thu, 16 Apr 2026 18:58:59 +0000", content)
        self.assertIn("OLDSIG==", content)
        self.assertIn("NEWSIG==", content)
        self.assertIn("BrowserOS Server (Alpha)", content)

    def test_parse_server_appcast_content_round_trips(self):
        parsed = parse_server_appcast_content(GOLDEN_CLAW_SERVER_APPCAST)
        self.assertIsNotNone(parsed)
        self.assertEqual(parsed.version, "0.0.5")
        self.assertEqual(parsed.pub_date, "Wed, 01 Jul 2026 01:18:52 +0000")
        self.assertEqual(list(parsed.artifacts), ["darwin_arm64"])
        self.assertEqual(parsed.artifacts["darwin_arm64"].signature, "CLAWSIG==")

    def test_parse_server_appcast_content_malformed_returns_none(self):
        self.assertIsNone(parse_server_appcast_content("<rss><channel>"))

    def test_hand_edited_garbage_length_drops_enclosure_not_crash(self):
        content = GOLDEN_CLAW_SERVER_APPCAST.replace('length="123"', 'length=""')

        parsed = parse_server_appcast_content(content)

        self.assertIsNotNone(parsed)
        self.assertEqual(parsed.version, "0.0.5")
        self.assertEqual(parsed.artifacts, {})


class ExtensionsRenderTest(unittest.TestCase):
    def test_golden_update_manifest(self):
        content = render_update_manifest(
            {"agent": "0.0.118.0", "bugreporter": "54.0.0.0"}
        )
        self.assertEqual(content, GOLDEN_UPDATE_MANIFEST)

    def test_golden_extensions_json(self):
        self.assertEqual(render_extensions_json("alpha"), GOLDEN_EXTENSIONS_JSON)

    def test_extensions_json_prod_points_at_prod_manifest(self):
        content = render_extensions_json("prod")
        self.assertIn("pjimfkbpehlcllblajnpfamdfjhhlgkc", content)
        self.assertIn(
            "https://cdn.browseros.com/extensions/update-manifest.xml", content
        )
        self.assertNotIn("alpha", content)

    def test_update_manifest_unknown_name_raises(self):
        with self.assertRaises(ValueError):
            render_update_manifest({"nope": "1.0"})


class VersionHelpersTest(unittest.TestCase):
    def test_epoch_version_sorts_above_legacy_scheme(self):
        self.assertGreater(
            parse_dotted_version("10000.0.47.0.2"), parse_dotted_version("7948.97")
        )

    def test_non_numeric_parts_parse_as_zero(self):
        self.assertEqual(parse_dotted_version("1.x.2"), (1, 0, 2))

    def test_trailing_zeros_do_not_matter(self):
        self.assertEqual(
            parse_dotted_version("0.0.118"), parse_dotted_version("0.0.118.0")
        )

    def test_extract_appcast_version(self):
        self.assertEqual(
            extract_appcast_version(GOLDEN_MAC_APPCAST), "10000.0.47.0.2"
        )
        self.assertEqual(extract_appcast_version(GOLDEN_CLAW_SERVER_APPCAST), "0.0.5")
        self.assertIsNone(extract_appcast_version("<rss><channel/></rss>"))

    def test_extract_manifest_versions(self):
        self.assertEqual(
            extract_manifest_versions(GOLDEN_UPDATE_MANIFEST),
            {
                "adlpneommgkgeanpaekgoaolcpncohkf": "54.0.0.0",
                "bflpfmnmnokmjhmgnolecpppdbdophmk": "0.0.118.0",
            },
        )

    def test_extract_enclosure_urls_covers_appcasts_and_manifests(self):
        self.assertEqual(
            extract_enclosure_urls(GOLDEN_MAC_APPCAST),
            [
                "https://cdn.browseros.com/releases/browseros/0.47.0.2/macos/BrowserOS_v0.47.0.2_arm64.dmg"
            ],
        )
        self.assertEqual(
            extract_enclosure_urls(GOLDEN_UPDATE_MANIFEST),
            [
                "https://cdn.browseros.com/extensions/bugreporter-54.0.0.0.crx",
                "https://cdn.browseros.com/extensions/agent-0.0.118.0.crx",
            ],
        )

    def test_extract_channel_metadata(self):
        self.assertEqual(
            extract_channel_metadata(GOLDEN_WIN_APPCAST),
            ("BrowserOS Windows Updates", "https://cdn.browseros.com/appcast-win.xml"),
        )


if __name__ == "__main__":
    unittest.main()
