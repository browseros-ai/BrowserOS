#!/usr/bin/env python3
"""Tests for the server OTA flow's alignment on the FeedSpec table."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock

from ...core.context import Context
from ...core.step import ValidationError
from ...products.browserclaw.product import BROWSERCLAW_SERVER_BUNDLE
from ..feeds.render import ExistingAppcast, SignedArtifact, render_server_appcast
from ..feeds.spec import server_feed
from . import server as ota_server
from .common import get_appcast_path, merge_base_appcast, promote_appcast_content
from .server import ServerOTAModule


def _artifact(platform="darwin_arm64", os="macos", arch="arm64"):
    return SignedArtifact(
        platform=platform,
        zip_path=Path(f"zip_{platform}.zip"),
        signature="ALPHASIG==",
        length=99,
        os=os,
        arch=arch,
    )


def _alpha_content(bundle_id="browseros-server"):
    return render_server_appcast(
        server_feed(bundle_id, "alpha"),
        "0.0.9",
        [_artifact()],
        ExistingAppcast(
            version="0.0.9",
            pub_date="Thu, 16 Apr 2026 18:58:59 +0000",
            artifacts={},
        ),
    )


class BundleDerivationTest(unittest.TestCase):
    def test_browseros_defaults_keep_todays_literals(self):
        module = ServerOTAModule(version="0.0.9", channel="alpha")

        self.assertEqual(
            module.artifact_key("darwin-arm64"),
            "artifacts/server/latest/browseros-server-resources-darwin-arm64.zip",
        )
        self.assertEqual(
            module.zip_filename("darwin_arm64"),
            "browseros_server_0.0.9_darwin_arm64.zip",
        )

    def test_browserclaw_derives_claw_keys(self):
        module = ServerOTAModule(
            version="0.0.9", channel="alpha", product_id="browserclaw"
        )

        self.assertEqual(
            module.artifact_key("darwin-arm64"),
            "claw-server/prod-resources/latest/browseros-claw-server-resources-darwin-arm64.zip",
        )
        self.assertEqual(
            module.zip_filename("darwin_arm64"),
            "browserclaw_server_0.0.9_darwin_arm64.zip",
        )

    def test_browserclaw_windows_signing_receives_claw_bundle(self):
        module = ServerOTAModule(
            version="0.0.9", channel="alpha", product_id="browserclaw"
        )
        resources = Path("/tmp/staged/resources")
        ctx = cast(Context, SimpleNamespace(env=object()))

        with mock.patch.object(
            ota_server, "sign_server_bundle_windows", return_value=True
        ) as signer:
            self.assertTrue(module._sign_bundle(resources, {"os": "windows"}, ctx))

        signer.assert_called_once_with(resources, ctx.env, BROWSERCLAW_SERVER_BUNDLE)

    def test_appcast_staging_paths_per_bundle(self):
        self.assertEqual(get_appcast_path("alpha").name, "appcast-server.alpha.xml")
        self.assertEqual(get_appcast_path("prod").name, "appcast-server.xml")
        self.assertEqual(
            get_appcast_path("prod", "browserclaw-server").name,
            "appcast-claw-server.xml",
        )
        self.assertEqual(
            get_appcast_path("alpha", "browserclaw-server").name,
            "appcast-claw-server.alpha.xml",
        )

    def test_validate_rejects_product_without_server_bundle(self):
        module = ServerOTAModule(
            version="0.0.9", channel="alpha", product_id="nope"
        )
        ctx = cast(
            Context,
            SimpleNamespace(
                env=SimpleNamespace(
                    macos_certificate_name="cert",
                    code_sign_tool_path="tool",
                    has_r2_config=lambda: True,
                )
            ),
        )

        with self.assertRaisesRegex(ValidationError, "nope"):
            module.validate(ctx)


class MergeBaseAppcastTest(unittest.TestCase):
    class _FakePublisher:
        def __init__(self, live=None):
            self.live = live

        def fetch_live(self, key):
            return self.live

    def test_live_feed_wins_over_staging_file(self):
        spec = server_feed("browseros-server", "alpha")
        with tempfile.TemporaryDirectory() as tmp:
            staging = Path(tmp) / "appcast-server.alpha.xml"
            staging.write_text(_alpha_content())

            base = merge_base_appcast(
                self._FakePublisher(live=_alpha_content().replace("0.0.9", "0.0.12")),
                spec,
                staging,
            )

        self.assertEqual(base.version, "0.0.12")

    def test_staging_file_is_fallback_when_no_live(self):
        spec = server_feed("browseros-server", "alpha")
        with tempfile.TemporaryDirectory() as tmp:
            staging = Path(tmp) / "appcast-server.alpha.xml"
            staging.write_text(_alpha_content())

            base = merge_base_appcast(self._FakePublisher(live=None), spec, staging)

        self.assertEqual(base.version, "0.0.9")

    def test_no_live_and_no_staging_returns_none(self):
        spec = server_feed("browseros-server", "alpha")
        base = merge_base_appcast(
            self._FakePublisher(live=None), spec, Path("/nonexistent/appcast.xml")
        )
        self.assertIsNone(base)


class PromoteContentTest(unittest.TestCase):
    def test_promote_rerenders_with_prod_metadata(self):
        prod_spec = server_feed("browseros-server", "prod")

        promoted = promote_appcast_content(_alpha_content(), prod_spec)

        self.assertIn("<title>BrowserOS Server</title>", promoted)
        self.assertIn(
            "<link>https://cdn.browseros.com/appcast-server.xml</link>", promoted
        )
        self.assertNotIn("(Alpha)", promoted)
        self.assertNotIn("appcast-server.alpha.xml", promoted)
        # Payload facts survive the re-render.
        self.assertIn("<sparkle:version>0.0.9</sparkle:version>", promoted)
        self.assertIn("Thu, 16 Apr 2026 18:58:59 +0000", promoted)
        self.assertIn("ALPHASIG==", promoted)
        self.assertIn("browseros_server_0.0.9_darwin_arm64.zip", promoted)

    def test_promote_refuses_malformed_source(self):
        with self.assertRaises(ValueError):
            promote_appcast_content(
                "<rss><channel>", server_feed("browseros-server", "prod")
            )

    def test_promote_refuses_source_without_enclosures(self):
        empty = render_server_appcast(
            server_feed("browseros-server", "alpha"),
            "0.0.9",
            [],
            ExistingAppcast(
                version="0.0.9",
                pub_date="Thu, 16 Apr 2026 18:58:59 +0000",
                artifacts={},
            ),
        )

        with self.assertRaisesRegex(ValueError, "enclosure"):
            promote_appcast_content(empty, server_feed("browseros-server", "prod"))


if __name__ == "__main__":
    unittest.main()
