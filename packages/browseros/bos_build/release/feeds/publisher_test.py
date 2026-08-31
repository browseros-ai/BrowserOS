#!/usr/bin/env python3
"""Tests for the rails-enforcing feed publisher (all R2/HTTP faked)."""

import hashlib
import io
import json
import tempfile
import unittest
from dataclasses import replace
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from .publisher import FeedPublisher
from .render import (
    ExistingAppcast,
    SignedArtifact,
    extract_appcast_version,
    extract_channel_metadata,
    extract_manifest_versions,
    render_browser_appcast,
    render_extensions_json,
    render_server_appcast,
    render_update_manifest,
)
from .spec import (
    EXTENSIONS,
    all_feeds,
    browser_feeds_for_product,
    feed_by_key,
    server_feed,
    update_manifest_feed,
)

FIXED_NOW = datetime(2026, 7, 1, 12, 0, 0, tzinfo=timezone.utc)
EXTENSION_VERSIONS = {
    "agent": "0.0.118.0",
    "bugreporter": "54.0.0.0",
    "browserclaw": "0.1.7.0",
}


class _FakeExceptions:
    class NoSuchKey(Exception):
        pass


class FakeR2Client:
    exceptions = _FakeExceptions

    def __init__(self, objects=None):
        self.objects = dict(objects or {})
        self.calls = []

    def get_object(self, Bucket, Key):
        if Key not in self.objects:
            raise self.exceptions.NoSuchKey(Key)
        return {"Body": io.BytesIO(self.objects[Key])}

    def copy_object(self, Bucket, CopySource, Key):
        self.calls.append(("copy", CopySource["Key"], Key))
        self.objects[Key] = self.objects[CopySource["Key"]]

    def put_object(self, Bucket, Key, Body, ContentType):
        self.calls.append(("put", Key, ContentType))
        self.objects[Key] = Body if isinstance(Body, bytes) else Body.encode()

    def list_objects_v2(self, Bucket, Prefix, **kwargs):
        keys = sorted(key for key in self.objects if key.startswith(Prefix))
        return {
            "Contents": [{"Key": key} for key in keys],
            "IsTruncated": False,
        }


def _artifact(
    url="https://cdn.browseros.com/releases/browseros/0.47.0.2/macos/BrowserOS_v0.47.0.2_arm64.dmg",
):
    return {
        "filename": url.rsplit("/", 1)[-1],
        "url": url,
        "sparkle_signature": "SIG==",
        "sparkle_length": 1234,
    }


def _mac_appcast(sparkle_version="10000.0.47.0.2"):
    return render_browser_appcast(
        feed_by_key("appcast.xml"),
        _artifact(),
        "0.47.0.2",
        sparkle_version,
        "2026-06-19T06:41:33Z",
    )


def _browserclaw_appcast(sparkle_version="10000.0.47.0.2"):
    return render_browser_appcast(
        feed_by_key("appcast-claw.xml"),
        _artifact(
            "https://cdn.browseros.com/releases/browserclaw/0.47.0.2/"
            "macos/BrowserOS_neo_v0.47.0.2_universal.dmg"
        ),
        "0.47.0.2",
        sparkle_version,
        "2026-06-19T06:41:33Z",
    )


def _legacy_browserclaw_appcast(sparkle_version="10000.0.47.0.2"):
    return _browserclaw_appcast(sparkle_version).replace("BrowserOS neo", "BrowserClaw")


def _empty_mac_appcast():
    spec = feed_by_key("appcast.xml")
    return f"""\
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>{spec.title}</title>
    <link>{spec.link}</link>
  </channel>
</rss>
"""


def _empty_item_mac_appcast():
    return _empty_mac_appcast().replace(
        "  </channel>",
        "    <item>\n    </item>\n  </channel>",
    )


def _empty_browserclaw_win_arm_appcast():
    spec = feed_by_key("appcast-claw-win-arm64.xml")
    return f"""\
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>{spec.title}</title>
    <link>{spec.link}</link>
    <item>
    </item>
  </channel>
</rss>
"""


def _server_appcast(bundle_id, channel, version):
    spec = server_feed(bundle_id, channel)
    artifact = SignedArtifact(
        platform="darwin_arm64",
        zip_path=Path("unused.zip"),
        signature="SIG==",
        length=1234,
        os="macos",
        arch="arm64",
    )
    return render_server_appcast(
        spec,
        version,
        [artifact],
        ExistingAppcast(
            version=version,
            pub_date="Wed, 01 Jul 2026 01:18:52 +0000",
            artifacts={},
        ),
    )


def _wrong_channel_server_appcast(bundle_id, version):
    content = _server_appcast(bundle_id, "prod", version)
    prod = server_feed(bundle_id, "prod")
    alpha = server_feed(bundle_id, "alpha")
    return (
        content.replace(f"<title>{prod.title}</title>", f"<title>{alpha.title}</title>")
        .replace(f"<link>{prod.link}</link>", f"<link>{alpha.link}</link>")
        .replace(
            f"<description>{prod.title} binary updates</description>",
            f"<description>{alpha.title} binary updates</description>",
        )
    )


def _malformed_manifest(versions):
    return render_update_manifest(versions).replace(
        "</gupdate>", "  </app>\n</gupdate>"
    )


class PublisherTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        root = Path(self._tmp.name)
        self.staging_root = root / "updates"
        self.head_calls = []

    def _publisher(self, objects=None, head_status=200):
        self.client = FakeR2Client(objects)

        def fake_head(url):
            self.head_calls.append(url)
            if isinstance(head_status, dict):
                return head_status.get(url, 200)
            return head_status

        return FeedPublisher(
            env=SimpleNamespace(r2_bucket="browseros"),
            r2_client=self.client,
            http_head=fake_head,
            staging_root=self.staging_root,
            now=lambda: FIXED_NOW,
        )

    def _write_staged(self, publisher, key, content):
        path = publisher.staging_path(feed_by_key(key))
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def test_staging_path_routes_each_feed_kind(self):
        publisher = self._publisher()

        self.assertEqual(
            publisher.staging_path(feed_by_key("appcast.xml")),
            self.staging_root / "browser" / "appcast.xml",
        )
        self.assertEqual(
            publisher.staging_path(feed_by_key("appcast-server.xml")),
            self.staging_root / "server" / "appcast-server.xml",
        )
        self.assertEqual(
            publisher.staging_path(feed_by_key("extensions/bundled-manifest.xml")),
            self.staging_root / "extensions" / "bundled-manifest.xml",
        )

    def test_dry_run_stages_without_r2_writes(self):
        publisher = self._publisher(
            {"appcast.xml": _mac_appcast("10000.0.46.0.0").encode()}
        )

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast())

        self.assertTrue(ok)
        self.assertEqual(self.client.calls, [])
        staged = self.staging_root / "browser" / "appcast.xml"
        self.assertEqual(staged.read_text(), _mac_appcast())

    def test_dry_run_stage_false_does_not_write_staging_file(self):
        publisher = self._publisher(
            {"appcast.xml": _mac_appcast("10000.0.46.0.0").encode()}
        )

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast(), stage=False)

        self.assertTrue(ok)
        self.assertEqual(self.client.calls, [])
        self.assertFalse((self.staging_root / "browser" / "appcast.xml").exists())

    def test_publish_backs_up_live_before_put(self):
        live = _mac_appcast("10000.0.46.0.0").encode()
        publisher = self._publisher({"appcast.xml": live})

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast(), publish=True)

        self.assertTrue(ok)
        self.assertEqual(
            self.client.calls,
            [
                ("copy", "appcast.xml", "feeds-history/appcast.xml.20260701T120000Z"),
                ("put", "appcast.xml", "application/xml"),
            ],
        )
        staged = self.staging_root / "browser" / "appcast.xml"
        self.assertEqual(staged.read_text(), _mac_appcast())

    def test_publish_identical_live_feed_is_idempotent(self):
        content = _mac_appcast()
        publisher = self._publisher({"appcast.xml": content.encode()})

        ok = publisher.publish(feed_by_key("appcast.xml"), content, publish=True)

        self.assertTrue(ok)
        self.assertEqual(self.client.calls, [])
        staged = self.staging_root / "browser" / "appcast.xml"
        self.assertEqual(staged.read_text(), content)

    def test_publish_without_live_object_skips_backup(self):
        publisher = self._publisher()

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast(), publish=True)

        self.assertTrue(ok)
        self.assertEqual(self.client.calls, [("put", "appcast.xml", "application/xml")])

    def test_empty_live_shell_is_valid_first_population(self):
        publisher = self._publisher({"appcast.xml": _empty_mac_appcast().encode()})

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast())

        self.assertTrue(ok)
        self.assertEqual(self.client.calls, [])
        self.assertEqual(
            (self.staging_root / "browser" / "appcast.xml").read_text(),
            _mac_appcast(),
        )

    def test_empty_live_item_is_backed_up_before_first_population(self):
        publisher = self._publisher({"appcast.xml": _empty_item_mac_appcast().encode()})

        ok = publisher.publish(
            feed_by_key("appcast.xml"),
            _mac_appcast(),
            publish=True,
        )

        self.assertTrue(ok)
        self.assertEqual(
            self.client.calls,
            [
                ("copy", "appcast.xml", "feeds-history/appcast.xml.20260701T120000Z"),
                ("put", "appcast.xml", "application/xml"),
            ],
        )

    def test_wrong_channel_empty_live_shell_fails_closed(self):
        live = _empty_item_mac_appcast().replace(
            "https://cdn.browseros.com/appcast.xml",
            "https://cdn.browseros.com/appcast-claw.xml",
        )
        publisher = self._publisher({"appcast.xml": live.encode()})

        ok = publisher.publish(
            feed_by_key("appcast.xml"),
            _mac_appcast(),
            publish=True,
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_populated_versionless_live_appcast_fails_closed(self):
        live = _mac_appcast().replace(
            "<sparkle:version>10000.0.47.0.2</sparkle:version>",
            "",
        )
        publisher = self._publisher({"appcast.xml": live.encode()})

        ok = publisher.publish(
            feed_by_key("appcast.xml"),
            _mac_appcast("10000.0.48.0.0"),
            publish=True,
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_duplicate_channel_with_populated_item_is_not_an_empty_shell(self):
        populated_channel = (
            _mac_appcast().split("<channel>", 1)[1].split("</channel>", 1)[0]
        )
        live = _empty_mac_appcast().replace(
            "</rss>",
            f"  <channel>{populated_channel}</channel>\n</rss>",
        )
        publisher = self._publisher({"appcast.xml": live.encode()})

        ok = publisher.publish(
            feed_by_key("appcast.xml"),
            _mac_appcast("10000.0.48.0.0"),
            publish=True,
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_item_outside_the_single_channel_is_not_an_empty_shell(self):
        live = _empty_mac_appcast().replace(
            "</rss>",
            "<item><title>meaningful old release</title></item>\n</rss>",
        )
        publisher = self._publisher({"appcast.xml": live.encode()})

        ok = publisher.publish(
            feed_by_key("appcast.xml"),
            _mac_appcast("10000.0.48.0.0"),
            publish=True,
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_malformed_live_appcast_version_fails_closed(self):
        for live_version in ("garbage", "10000.0.beta"):
            with self.subTest(live_version=live_version):
                live = _mac_appcast().replace(
                    "10000.0.47.0.2",
                    live_version,
                )
                publisher = self._publisher({"appcast.xml": live.encode()})

                ok = publisher.publish(
                    feed_by_key("appcast.xml"),
                    _mac_appcast("10000.0.48.0.0"),
                    publish=True,
                )

                self.assertFalse(ok)
                self.assertEqual(self.client.calls, [])

    def test_every_populated_live_item_requires_one_strict_version(self):
        bad_items = (
            (
                "malformed",
                "<item><sparkle:version>garbage</sparkle:version></item>",
            ),
            ("versionless", "<item><title>old release</title></item>"),
        )
        for label, bad_item in bad_items:
            with self.subTest(item=label):
                live = _mac_appcast("10000.0.46.0.0").replace(
                    "  </channel>",
                    f"    {bad_item}\n  </channel>",
                )
                publisher = self._publisher({"appcast.xml": live.encode()})

                ok = publisher.publish(
                    feed_by_key("appcast.xml"),
                    _mac_appcast("10000.0.48.0.0"),
                    publish=True,
                )

                self.assertFalse(ok)
                self.assertEqual(self.client.calls, [])

    def test_every_new_appcast_item_requires_one_strict_version(self):
        content = _mac_appcast().replace(
            "  </channel>",
            "    <item><title>versionless</title></item>\n  </channel>",
        )
        publisher = self._publisher()

        ok = publisher.publish(
            feed_by_key("appcast.xml"),
            content,
            publish=True,
            allow_downgrade=True,
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_populated_wrong_channel_live_appcast_fails_closed(self):
        live = _mac_appcast("10000.0.46.0.0").replace(
            "<title>BrowserOS</title>",
            "<title>BrowserClaw</title>",
        )
        publisher = self._publisher({"appcast.xml": live.encode()})

        ok = publisher.publish(
            feed_by_key("appcast.xml"),
            _mac_appcast(),
            publish=True,
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_browserclaw_legacy_title_migrates_with_backup(self):
        spec = feed_by_key("appcast-claw.xml")
        publisher = self._publisher(
            {spec.key: _legacy_browserclaw_appcast("10000.0.46.0.0").encode()}
        )

        ok = publisher.publish(spec, _browserclaw_appcast(), publish=True)

        self.assertTrue(ok)
        self.assertEqual(
            self.client.calls,
            [
                (
                    "copy",
                    spec.key,
                    f"feeds-history/{spec.key}.20260701T120000Z",
                ),
                ("put", spec.key, "application/xml"),
            ],
        )

    def test_browserclaw_legacy_title_migrates_on_empty_placeholder(self):
        spec = feed_by_key("appcast-claw-win-arm64.xml")
        canonical = _empty_browserclaw_win_arm_appcast()
        legacy = canonical.replace(spec.title, spec.legacy_titles[0])
        publisher = self._publisher({spec.key: legacy.encode()})

        ok = publisher.publish(spec, canonical, publish=True)

        self.assertTrue(ok)
        self.assertEqual(
            self.client.calls,
            [
                (
                    "copy",
                    spec.key,
                    f"feeds-history/{spec.key}.20260701T120000Z",
                ),
                ("put", spec.key, "application/xml"),
            ],
        )

    def test_empty_placeholder_migration_rejects_release_markers(self):
        spec = feed_by_key("appcast-claw-win-arm64.xml")
        canonical = _empty_browserclaw_win_arm_appcast()
        legacy = canonical.replace(spec.title, spec.legacy_titles[0])
        malformed_live = {
            "channel release children": legacy.replace(
                "    <item>\n    </item>",
                "    <sparkle:version>10000.0.99.0</sparkle:version>\n"
                '    <enclosure url="https://cdn.browseros.com/release.dmg"/>',
            ),
            "root release child": legacy.replace(
                "</rss>",
                "  <sparkle:version>10000.0.99.0</sparkle:version>\n</rss>",
            ),
            "root release attribute": legacy.replace(
                '<rss version="2.0"',
                '<rss version="2.0" release-version="10000.0.99.0"',
            ),
            "channel release attribute": legacy.replace(
                "  <channel>", '  <channel release-version="10000.0.99.0">'
            ),
            "channel mixed content": legacy.replace(
                "  <channel>\n", "  <channel>release-version=10000.0.99.0\n"
            ),
            "metadata tail content": legacy.replace(
                f"<title>{spec.legacy_titles[0]}</title>",
                f"<title>{spec.legacy_titles[0]}</title>release-version=10000.0.99.0",
            ),
        }

        for label, live in malformed_live.items():
            with self.subTest(shape=label):
                publisher = self._publisher({spec.key: live.encode()})

                ok = publisher.publish(spec, canonical, publish=True)

                self.assertFalse(ok)
                self.assertEqual(self.client.calls, [])

    def test_browserclaw_legacy_title_migration_refuses_downgrade(self):
        spec = feed_by_key("appcast-claw.xml")
        publisher = self._publisher(
            {spec.key: _legacy_browserclaw_appcast("10000.0.48.0.0").encode()}
        )

        ok = publisher.publish(spec, _browserclaw_appcast(), publish=True)

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_browserclaw_legacy_title_requires_canonical_link(self):
        spec = feed_by_key("appcast-claw.xml")
        live = _legacy_browserclaw_appcast().replace(
            spec.link, "https://cdn.browseros.com/appcast.xml"
        )
        publisher = self._publisher({spec.key: live.encode()})

        ok = publisher.publish(spec, _browserclaw_appcast(), publish=True)

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_downgrade_refused_without_flag(self):
        publisher = self._publisher(
            {"appcast.xml": _mac_appcast("10000.0.48.0.0").encode()}
        )

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast("10000.0.47.0.2"), publish=True
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])
        self.assertFalse((self.staging_root / "browser").exists())

    def test_downgrade_allowed_with_flag(self):
        publisher = self._publisher(
            {"appcast.xml": _mac_appcast("10000.0.48.0.0").encode()}
        )

        ok = publisher.publish(
            feed_by_key("appcast.xml"),
            _mac_appcast("10000.0.47.0.2"),
            publish=True,
            allow_downgrade=True,
        )

        self.assertTrue(ok)
        self.assertEqual(len(self.client.calls), 2)

    def test_equal_version_passes(self):
        publisher = self._publisher({"appcast.xml": _mac_appcast().encode()})

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast(), publish=True)

        self.assertTrue(ok)

    def test_legacy_scheme_live_version_is_older(self):
        legacy = _mac_appcast().replace("10000.0.47.0.2", "7948.97")
        publisher = self._publisher({"appcast.xml": legacy.encode()})

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast(), publish=True)

        self.assertTrue(ok)

    def test_channel_metadata_mismatch_refused(self):
        alpha_content = render_server_appcast(
            server_feed("browseros-server", "alpha"),
            "0.0.9",
            [],
            ExistingAppcast(
                version="0.0.9",
                pub_date="Wed, 01 Jul 2026 01:18:52 +0000",
                artifacts={},
            ),
        )
        publisher = self._publisher()

        ok = publisher.publish(
            server_feed("browseros-server", "prod"), alpha_content, publish=True
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_head_failure_refuses_and_blocks_put(self):
        url = _artifact()["url"]
        publisher = self._publisher(head_status={url: 404})

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast(), publish=True)

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])
        self.assertIn(url, self.head_calls)

    def test_failed_backup_blocks_the_put(self):
        publisher = self._publisher(
            {"appcast.xml": _mac_appcast("10000.0.46.0.0").encode()}
        )

        def broken_copy(**kwargs):
            raise RuntimeError("copy exploded")

        self.client.copy_object = broken_copy

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast(), publish=True)

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_manifest_entry_removal_refused_even_with_override(self):
        spec = update_manifest_feed("alpha")
        live = render_update_manifest(EXTENSION_VERSIONS)
        publisher = self._publisher({spec.key: live.encode()})

        missing_browserclaw = render_update_manifest(
            {
                name: version
                for name, version in EXTENSION_VERSIONS.items()
                if name != "browserclaw"
            }
        )
        self.assertFalse(publisher.publish(spec, missing_browserclaw, publish=True))
        self.assertEqual(self.client.calls, [])

        self.assertFalse(
            publisher.publish(
                spec,
                missing_browserclaw,
                publish=True,
                allow_downgrade=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_update_manifest_guards_per_extension(self):
        spec = update_manifest_feed("alpha")
        live = render_update_manifest(EXTENSION_VERSIONS)
        publisher = self._publisher({spec.key: live.encode()})

        downgraded = render_update_manifest(
            {**EXTENSION_VERSIONS, "agent": "0.0.117.0"}
        )
        self.assertFalse(publisher.publish(spec, downgraded, publish=True))
        self.assertEqual(self.client.calls, [])

        upgraded = render_update_manifest({**EXTENSION_VERSIONS, "agent": "0.0.119.0"})
        self.assertTrue(publisher.publish(spec, upgraded, publish=True))

    def test_default_rejects_all_invalid_live_repair_pairs(self):
        extension_versions = {
            "agent": "0.0.123.0",
            "bugreporter": "54.0.0.0",
            "browserclaw": "0.1.7.0",
        }
        cases = (
            (
                server_feed("browseros-server", "prod"),
                _server_appcast("browseros-server", "prod", "0.0.127"),
                _wrong_channel_server_appcast("browseros-server", "0.0.127"),
            ),
            (
                server_feed("browserclaw-server", "prod"),
                _server_appcast("browserclaw-server", "prod", "0.0.15"),
                _wrong_channel_server_appcast("browserclaw-server", "0.0.15"),
            ),
            (
                update_manifest_feed("alpha"),
                render_update_manifest(extension_versions),
                _malformed_manifest(extension_versions),
            ),
        )

        for spec, corrected, invalid_live in cases:
            with self.subTest(spec=spec.key):
                publisher = self._publisher({spec.key: invalid_live.encode()})

                self.assertFalse(publisher.publish(spec, corrected, publish=True))
                self.assertEqual(self.client.calls, [])

    def test_allow_downgrade_does_not_repair_wrong_channel_live(self):
        for bundle_id, version in (
            ("browseros-server", "0.0.127"),
            ("browserclaw-server", "0.0.15"),
        ):
            spec = server_feed(bundle_id, "prod")
            publisher = self._publisher(
                {spec.key: _wrong_channel_server_appcast(bundle_id, version).encode()}
            )

            self.assertFalse(
                publisher.publish(
                    spec,
                    _server_appcast(bundle_id, "prod", version),
                    publish=True,
                    allow_downgrade=True,
                )
            )
            self.assertEqual(self.client.calls, [])

    def test_repair_accepts_same_version_wrong_channel_appcasts(self):
        for bundle_id, version in (
            ("browseros-server", "0.0.127"),
            ("browserclaw-server", "0.0.15"),
        ):
            spec = server_feed(bundle_id, "prod")
            publisher = self._publisher(
                {spec.key: _wrong_channel_server_appcast(bundle_id, version).encode()}
            )

            self.assertTrue(
                publisher.publish(
                    spec,
                    _server_appcast(bundle_id, "prod", version),
                    publish=True,
                    repair_invalid_live=True,
                )
            )
            self.assertEqual(
                self.client.calls,
                [
                    (
                        "copy",
                        spec.key,
                        f"feeds-history/{spec.key}.20260701T120000Z",
                    ),
                    ("put", spec.key, "application/xml"),
                ],
            )

    def test_repair_accepts_nonempty_manifest_when_live_is_malformed(self):
        versions = {
            "agent": "0.0.123.0",
            "bugreporter": "54.0.0.0",
            "browserclaw": "0.1.7.0",
        }
        spec = update_manifest_feed("alpha")
        publisher = self._publisher({spec.key: _malformed_manifest(versions).encode()})

        self.assertTrue(
            publisher.publish(
                spec,
                render_update_manifest(versions),
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_refuses_recoverable_appcast_downgrade(self):
        spec = server_feed("browseros-server", "prod")
        publisher = self._publisher(
            {
                spec.key: _wrong_channel_server_appcast(
                    "browseros-server", "0.0.128"
                ).encode()
            }
        )

        self.assertFalse(
            publisher.publish(
                spec,
                _server_appcast("browseros-server", "prod", "0.0.127"),
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_wrong_channel_requires_recoverable_live_version(self):
        spec = server_feed("browseros-server", "prod")
        live = _wrong_channel_server_appcast("browseros-server", "0.0.127").replace(
            "<sparkle:version>0.0.127</sparkle:version>", ""
        )
        publisher = self._publisher({spec.key: live.encode()})

        self.assertFalse(
            publisher.publish(
                spec,
                _server_appcast("browseros-server", "prod", "0.0.127"),
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_malformed_appcast_compares_recoverable_version(self):
        spec = server_feed("browseros-server", "prod")
        live = _server_appcast("browseros-server", "prod", "0.0.128").replace(
            "</rss>", "</rss></rss>"
        )
        publisher = self._publisher({spec.key: live.encode()})

        self.assertFalse(
            publisher.publish(
                spec,
                _server_appcast("browseros-server", "prod", "0.0.127"),
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_malformed_versionless_appcast_accepts_strict_replacement(self):
        spec = server_feed("browseros-server", "prod")
        publisher = self._publisher({spec.key: b"garbage <not xml"})

        self.assertTrue(
            publisher.publish(
                spec,
                _server_appcast("browseros-server", "prod", "0.0.127"),
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_refuses_parseable_manifest_downgrade(self):
        spec = update_manifest_feed("alpha")
        live = render_update_manifest({**EXTENSION_VERSIONS, "agent": "0.0.124.0"})
        publisher = self._publisher({spec.key: live.encode()})

        self.assertFalse(
            publisher.publish(
                spec,
                render_update_manifest({**EXTENSION_VERSIONS, "agent": "0.0.123.0"}),
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_refuses_invalid_new_content(self):
        spec = server_feed("browseros-server", "prod")
        invalid_live = _wrong_channel_server_appcast("browseros-server", "0.0.127")
        cases = (
            "<rss><channel>",
            _server_appcast("browseros-server", "alpha", "0.0.127"),
            _empty_mac_appcast(),
        )

        for content in cases:
            with self.subTest(content=content[:30]):
                publisher = self._publisher({spec.key: invalid_live.encode()})
                self.assertFalse(
                    publisher.publish(
                        spec,
                        content,
                        publish=True,
                        repair_invalid_live=True,
                    )
                )
                self.assertEqual(self.client.calls, [])

    def test_repair_refuses_empty_manifest(self):
        versions = {"agent": "0.0.123.0"}
        spec = update_manifest_feed("alpha")
        publisher = self._publisher({spec.key: _malformed_manifest(versions).encode()})
        empty = (
            '<?xml version="1.0"?><gupdate '
            'xmlns="http://www.google.com/update2/response" protocol="2.0"/>'
        )

        self.assertFalse(
            publisher.publish(
                spec,
                empty,
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_refuses_failed_download_url(self):
        spec = server_feed("browseros-server", "prod")
        publisher = self._publisher(
            {
                spec.key: _wrong_channel_server_appcast(
                    "browseros-server", "0.0.127"
                ).encode()
            },
            head_status=404,
        )

        self.assertFalse(
            publisher.publish(
                spec,
                _server_appcast("browseros-server", "prod", "0.0.127"),
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_refuses_appcast_with_any_missing_download_url(self):
        spec = server_feed("browseros-server", "prod")
        publisher = self._publisher(
            {
                spec.key: _wrong_channel_server_appcast(
                    "browseros-server", "0.0.127"
                ).encode()
            }
        )
        content = _server_appcast("browseros-server", "prod", "0.0.127").replace(
            "</item>", '<enclosure length="1" />\n</item>'
        )

        self.assertFalse(
            publisher.publish(
                spec,
                content,
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_refuses_manifest_with_missing_download_url(self):
        versions = {**EXTENSION_VERSIONS, "agent": "0.0.123.0"}
        spec = update_manifest_feed("alpha")
        publisher = self._publisher({spec.key: _malformed_manifest(versions).encode()})
        content = render_update_manifest(versions).replace(
            ' codebase="', ' data-codebase="'
        )

        self.assertFalse(
            publisher.publish(
                spec,
                content,
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_refuses_appcast_missing_download_url_with_valid_live(self):
        spec = server_feed("browseros-server", "prod")
        live = _server_appcast("browseros-server", "prod", "0.0.127")
        content = live.replace(' url="', ' data-url="')
        publisher = self._publisher({spec.key: live.encode()})

        self.assertFalse(
            publisher.publish(
                spec,
                content,
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_refuses_manifest_missing_download_url_with_valid_live(self):
        spec = update_manifest_feed("alpha")
        live = render_update_manifest({**EXTENSION_VERSIONS, "agent": "0.0.123.0"})
        content = live.replace(' codebase="', ' data-codebase="')
        publisher = self._publisher({spec.key: live.encode()})

        self.assertFalse(
            publisher.publish(
                spec,
                content,
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_refuses_unpublishable_spec(self):
        spec = replace(feed_by_key("appcast.xml"), publishable=False)
        publisher = self._publisher({spec.key: b"garbage <not xml"})

        self.assertFalse(
            publisher.publish(
                spec,
                _mac_appcast(),
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_logs_hashes_and_backs_up_before_put(self):
        spec = server_feed("browseros-server", "prod")
        corrected = _server_appcast("browseros-server", "prod", "0.0.127")
        invalid_live = _wrong_channel_server_appcast("browseros-server", "0.0.127")
        publisher = self._publisher({spec.key: invalid_live.encode()})

        with mock.patch("bos_build.release.feeds.publisher.log_warning") as warning:
            self.assertTrue(
                publisher.publish(
                    spec,
                    corrected,
                    publish=True,
                    repair_invalid_live=True,
                )
            )

        message = " ".join(call.args[0] for call in warning.call_args_list)
        self.assertIn("REPAIR INVALID LIVE", message)
        self.assertIn(hashlib.sha256(invalid_live.encode()).hexdigest(), message)
        self.assertIn(hashlib.sha256(corrected.encode()).hexdigest(), message)
        self.assertEqual(self.client.calls[0][0], "copy")
        self.assertEqual(self.client.calls[1][0], "put")

    def test_repair_backup_failure_blocks_put(self):
        spec = server_feed("browseros-server", "prod")
        publisher = self._publisher(
            {
                spec.key: _wrong_channel_server_appcast(
                    "browseros-server", "0.0.127"
                ).encode()
            }
        )

        def broken_copy(**kwargs):
            raise RuntimeError("copy exploded")

        self.client.copy_object = broken_copy

        self.assertFalse(
            publisher.publish(
                spec,
                _server_appcast("browseros-server", "prod", "0.0.127"),
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_repair_batch_invalid_new_file_performs_zero_live_calls(self):
        browseros_spec = server_feed("browseros-server", "prod")
        browserclaw_spec = server_feed("browserclaw-server", "prod")
        extension_spec = update_manifest_feed("alpha")
        publisher = self._publisher(
            {
                browseros_spec.key: _wrong_channel_server_appcast(
                    "browseros-server", "0.0.127"
                ).encode(),
                browserclaw_spec.key: _wrong_channel_server_appcast(
                    "browserclaw-server", "0.0.15"
                ).encode(),
                extension_spec.key: _malformed_manifest(
                    {"agent": "0.0.123.0"}
                ).encode(),
            }
        )
        self._write_staged(
            publisher,
            browseros_spec.key,
            _server_appcast("browseros-server", "prod", "0.0.127"),
        )
        self._write_staged(
            publisher,
            browserclaw_spec.key,
            _server_appcast("browserclaw-server", "prod", "0.0.15"),
        )
        self._write_staged(publisher, extension_spec.key, "<gupdate>")

        self.assertFalse(
            publisher.publish_staged(
                [browseros_spec.key, browserclaw_spec.key, extension_spec.key],
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_mixed_repair_batch_is_retry_safe(self):
        browseros_spec = server_feed("browseros-server", "prod")
        browserclaw_spec = server_feed("browserclaw-server", "prod")
        browseros = _server_appcast("browseros-server", "prod", "0.0.127")
        browserclaw = _server_appcast("browserclaw-server", "prod", "0.0.15")
        publisher = self._publisher(
            {
                browseros_spec.key: browseros.encode(),
                browserclaw_spec.key: _wrong_channel_server_appcast(
                    "browserclaw-server", "0.0.15"
                ).encode(),
            }
        )
        self._write_staged(publisher, browseros_spec.key, browseros)
        self._write_staged(publisher, browserclaw_spec.key, browserclaw)
        keys = [browseros_spec.key, browserclaw_spec.key]

        self.assertTrue(
            publisher.publish_staged(
                keys,
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(len(self.client.calls), 2)

        self.client.calls.clear()
        self.assertTrue(
            publisher.publish_staged(
                keys,
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_tracked_snapshots_have_canonical_metadata(self):
        updates = Path(__file__).resolve().parents[5] / "updates"

        # These files are release outputs, so pin stable schema and ownership
        # invariants instead of versions or whole-file hashes. Otherwise every
        # valid snapshot promotion makes the default branch's test suite stale.
        for bundle_id in ("browseros-server", "browserclaw-server"):
            spec = server_feed(bundle_id, "prod")
            with self.subTest(key=spec.key):
                content = (updates / "server" / spec.key).read_text()
                self.assertEqual((spec.kind, spec.channel), ("server", "prod"))
                self.assertEqual(
                    extract_channel_metadata(content),
                    (spec.title, spec.link),
                )
                self.assertIn(
                    f"<description>{spec.title} binary updates</description>",
                    content,
                )
                self.assertIsNotNone(extract_appcast_version(content))

        expected_extension_ids = {
            extension.extension_id
            for extension in EXTENSIONS
            if extension.in_update_feed
        }
        for channel in ("alpha", "prod"):
            spec = update_manifest_feed(channel)
            with self.subTest(key=spec.key):
                manifest = (updates / spec.key).read_text()
                self.assertEqual(
                    (spec.kind, spec.channel), ("extensions", channel)
                )
                self.assertEqual(
                    set(extract_manifest_versions(manifest)),
                    expected_extension_ids,
                )

    def test_browserclaw_snapshots_use_current_product_title(self):
        updates = Path(__file__).resolve().parents[5] / "updates" / "browser"

        for spec in browser_feeds_for_product("browserclaw"):
            with self.subTest(key=spec.key):
                content = (updates / spec.key).read_text()
                self.assertEqual(
                    extract_channel_metadata(content),
                    (spec.title, spec.link),
                )
                self.assertNotIn("<title>BrowserClaw", content)

    def test_extensions_json_skips_head_and_publishes(self):
        spec = feed_by_key("extensions/extensions.alpha.json")
        publisher = self._publisher()

        ok = publisher.publish(spec, render_extensions_json("alpha"), publish=True)

        self.assertTrue(ok)
        self.assertEqual(self.head_calls, [])
        self.assertEqual(
            self.client.calls,
            [("put", "extensions/extensions.alpha.json", "application/json")],
        )
        staged = self.staging_root / "extensions" / "extensions.alpha.json"
        self.assertTrue(staged.exists())

    def test_extension_json_requires_the_exact_canonical_value(self):
        spec = feed_by_key("extensions/extensions.alpha.json")
        invalid = [{"extensions": {}}]

        missing = json.loads(render_extensions_json("alpha"))
        missing["extensions"].pop(next(iter(missing["extensions"])))
        invalid.append(missing)

        extra = json.loads(render_extensions_json("alpha"))
        extra["extensions"]["a" * 32] = {
            "external_update_url": update_manifest_feed("alpha").url
        }
        invalid.append(extra)

        alternate_install = json.loads(render_extensions_json("alpha"))
        next(iter(alternate_install["extensions"].values()))["external_crx"] = (
            "https://cdn.browseros.com/extensions/agent-0.0.118.0.crx"
        )
        invalid.append(alternate_install)

        duplicate_key = render_extensions_json("alpha").replace(
            '"extensions": {',
            '"extensions": {}, "extensions": {',
            1,
        )
        invalid_content = [json.dumps(document) for document in invalid]
        invalid_content.append(duplicate_key)

        for content in invalid_content:
            with self.subTest(content=content):
                publisher = self._publisher()
                self.assertFalse(
                    publisher.publish(
                        spec,
                        content,
                        publish=True,
                        allow_downgrade=True,
                        repair_invalid_live=True,
                    )
                )
                self.assertEqual(self.client.calls, [])

    def test_extension_manifests_require_exact_ids_and_canonical_crx_binding(self):
        canonical = render_update_manifest(EXTENSION_VERSIONS)
        invalid = (
            render_update_manifest(
                {
                    name: version
                    for name, version in EXTENSION_VERSIONS.items()
                    if name != "browserclaw"
                }
            ),
            canonical.replace("pjimfkbpehlcllblajnpfamdfjhhlgkc", "a" * 32),
            canonical.replace(
                "pjimfkbpehlcllblajnpfamdfjhhlgkc",
                "bflpfmnmnokmjhmgnolecpppdbdophmk",
            ),
            canonical.replace(
                "agent-0.0.118.0.crx",
                "browserclaw-0.1.7.0.crx",
            ),
            canonical.replace(
                'version="0.0.118.0"',
                'version="0.0.119.0"',
            ),
        )

        for key in (
            "extensions/update-manifest.alpha.xml",
            "extensions/bundled-manifest.xml",
        ):
            for content in invalid:
                with self.subTest(key=key, content=content):
                    publisher = self._publisher()
                    self.assertFalse(
                        publisher.publish(
                            feed_by_key(key),
                            content,
                            publish=True,
                            allow_downgrade=True,
                            repair_invalid_live=True,
                        )
                    )
                    self.assertEqual(self.client.calls, [])
                    self.assertEqual(self.head_calls, [])

    def test_publish_staged_extension_pair_writes_manifest_before_json(self):
        publisher = self._publisher()
        json_key = "extensions/extensions.alpha.json"
        manifest_key = "extensions/update-manifest.alpha.xml"
        self._write_staged(publisher, json_key, render_extensions_json("alpha"))
        self._write_staged(
            publisher,
            manifest_key,
            render_update_manifest(EXTENSION_VERSIONS),
        )

        self.assertTrue(
            publisher.publish_staged([json_key, manifest_key], publish=True)
        )

        self.assertEqual(
            [call[1] for call in self.client.calls if call[0] == "put"],
            [manifest_key, json_key],
        )

    def test_publish_staged_invalid_extension_json_blocks_the_whole_pair(self):
        publisher = self._publisher()
        json_key = "extensions/extensions.alpha.json"
        manifest_key = "extensions/update-manifest.alpha.xml"
        self._write_staged(publisher, json_key, '{"extensions": {}}')
        self._write_staged(
            publisher,
            manifest_key,
            render_update_manifest(EXTENSION_VERSIONS),
        )

        self.assertFalse(
            publisher.publish_staged(
                [json_key, manifest_key],
                publish=True,
                allow_downgrade=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(self.client.calls, [])

    def test_publish_staged_json_requires_selected_or_valid_live_manifest(self):
        json_key = "extensions/extensions.alpha.json"
        manifest_key = "extensions/update-manifest.alpha.xml"

        publisher = self._publisher()
        self._write_staged(publisher, json_key, render_extensions_json("alpha"))
        self.assertFalse(publisher.publish_staged([json_key], publish=True))
        self.assertEqual(self.client.calls, [])

        publisher = self._publisher(
            {
                manifest_key: render_update_manifest(
                    {
                        name: version
                        for name, version in EXTENSION_VERSIONS.items()
                        if name != "browserclaw"
                    }
                ).encode()
            }
        )
        self._write_staged(publisher, json_key, render_extensions_json("alpha"))
        self.assertFalse(publisher.publish_staged([json_key], publish=True))
        self.assertEqual(self.client.calls, [])

    def test_publish_staged_keeps_manifest_repair_and_bundled_independent(self):
        manifest_key = "extensions/update-manifest.alpha.xml"
        corrected = render_update_manifest(EXTENSION_VERSIONS)
        publisher = self._publisher(
            {manifest_key: _malformed_manifest(EXTENSION_VERSIONS).encode()}
        )
        self._write_staged(publisher, manifest_key, corrected)

        self.assertTrue(
            publisher.publish_staged(
                [manifest_key],
                publish=True,
                repair_invalid_live=True,
            )
        )
        self.assertEqual(
            [call[0] for call in self.client.calls],
            ["copy", "put"],
        )

        bundled_key = "extensions/bundled-manifest.xml"
        publisher = self._publisher()
        self._write_staged(publisher, bundled_key, corrected)
        self.assertTrue(publisher.publish_staged([bundled_key], publish=True))
        self.assertEqual(
            [call[1] for call in self.client.calls if call[0] == "put"],
            [bundled_key],
        )

    def test_publish_staged_preflights_full_batch_before_live_writes(self):
        publisher = self._publisher()
        self._write_staged(publisher, "appcast.xml", _mac_appcast())
        self._write_staged(
            publisher,
            "appcast-x86_64.xml",
            "<rss><channel>",
        )

        ok = publisher.publish_staged(
            ["appcast.xml", "appcast-x86_64.xml"], publish=True
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_publish_staged_defaults_to_dry_run(self):
        manifest_key = "extensions/update-manifest.alpha.xml"
        publisher = self._publisher(
            {manifest_key: render_update_manifest(EXTENSION_VERSIONS).encode()}
        )
        key = "extensions/extensions.alpha.json"
        self._write_staged(publisher, key, render_extensions_json("alpha"))

        self.assertTrue(publisher.publish_staged([key]))

        self.assertEqual(self.client.calls, [])

    def test_publish_staged_writes_after_successful_preflight(self):
        manifest_key = "extensions/update-manifest.alpha.xml"
        publisher = self._publisher(
            {manifest_key: render_update_manifest(EXTENSION_VERSIONS).encode()}
        )
        key = "extensions/extensions.alpha.json"
        self._write_staged(publisher, key, render_extensions_json("alpha"))

        self.assertTrue(publisher.publish_staged([key], publish=True))

        self.assertEqual(
            self.client.calls,
            [("put", key, "application/json")],
        )

    def test_publish_staged_refuses_unknown_key_before_rails(self):
        publisher = self._publisher()

        with mock.patch.object(publisher, "publish") as publish:
            self.assertFalse(publisher.publish_staged(["unknown.xml"]))

        publish.assert_not_called()

    def test_publish_staged_refuses_missing_local_file_before_rails(self):
        publisher = self._publisher()

        with mock.patch.object(publisher, "publish") as publish:
            self.assertFalse(publisher.publish_staged(["appcast.xml"]))

        publish.assert_not_called()

    def test_malformed_new_content_refused(self):
        publisher = self._publisher()

        ok = publisher.publish(
            feed_by_key("appcast.xml"), "<rss><channel>", publish=True
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_empty_appcast_refused_with_item_error(self):
        publisher = self._publisher()

        with mock.patch("bos_build.release.feeds.publisher.log_error") as log_error:
            ok = publisher.publish(
                feed_by_key("appcast.xml"), _empty_mac_appcast(), publish=True
            )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])
        log_error.assert_called_with("appcast.xml: new content has no <item> entries")

    def test_versionless_appcast_refused_with_version_error(self):
        content = _mac_appcast().replace(
            "<sparkle:version>10000.0.47.0.2</sparkle:version>", ""
        )
        publisher = self._publisher()

        with mock.patch("bos_build.release.feeds.publisher.log_error") as log_error:
            ok = publisher.publish(feed_by_key("appcast.xml"), content, publish=True)

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])
        log_error.assert_called_with(
            "appcast.xml: new content has <item> entries but no sparkle:version"
        )

    def test_live_guard_distinguishes_empty_appcast(self):
        publisher = self._publisher()

        with mock.patch("bos_build.release.feeds.publisher.log_error") as log_error:
            ok = publisher._guard_appcast_version(
                feed_by_key("appcast.xml"),
                _empty_mac_appcast(),
                _mac_appcast(),
                False,
            )

        self.assertFalse(ok)
        log_error.assert_called_with("appcast.xml: new content has no <item> entries")

    def test_live_guard_distinguishes_versionless_items(self):
        content = _mac_appcast().replace(
            "<sparkle:version>10000.0.47.0.2</sparkle:version>", ""
        )
        publisher = self._publisher()

        with mock.patch("bos_build.release.feeds.publisher.log_error") as log_error:
            ok = publisher._guard_appcast_version(
                feed_by_key("appcast.xml"), content, _mac_appcast(), False
            )

        self.assertFalse(ok)
        log_error.assert_called_with(
            "appcast.xml: new content has <item> entries but no sparkle:version"
        )

    def test_json_array_document_refused_cleanly(self):
        publisher = self._publisher()

        ok = publisher.publish(
            feed_by_key("extensions/extensions.json"), "[1, 2]", publish=True
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_unparseable_live_fails_closed_without_flag(self):
        publisher = self._publisher({"appcast.xml": b"garbage <not xml"})

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast(), publish=True)

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_unparseable_live_remains_blocked_with_override(self):
        publisher = self._publisher({"appcast.xml": b"garbage <not xml"})

        ok = publisher.publish(
            feed_by_key("appcast.xml"),
            _mac_appcast(),
            publish=True,
            allow_downgrade=True,
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_extensions_json_for_wrong_channel_refused(self):
        publisher = self._publisher()

        ok = publisher.publish(
            feed_by_key("extensions/extensions.json"),
            render_extensions_json("alpha"),
            publish=True,
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_quiet_preflight_skips_output_but_keeps_rails(self):
        publisher = self._publisher(
            {"appcast.xml": _mac_appcast("10000.0.48.0.0").encode()}
        )

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast(), verbose=False
        )

        self.assertFalse(ok)

    def test_collect_status_covers_every_feed(self):
        publisher = self._publisher(
            {
                "appcast.xml": _mac_appcast().encode(),
                "feeds-history/appcast.xml.20260630T000000Z": b"old",
                "feeds-history/appcast.xml.20260701T120000Z": b"older backup",
                "extensions/update-manifest.alpha.xml": render_update_manifest(
                    {"agent": "0.0.118.0", "bugreporter": "54.0.0.0"}
                ).encode(),
                "extensions/extensions.alpha.json": render_extensions_json(
                    "alpha"
                ).encode(),
            }
        )

        statuses = {s.spec.key: s for s in publisher.collect_status()}

        self.assertEqual(set(statuses), {feed.key for feed in all_feeds()})

        appcast = statuses["appcast.xml"]
        self.assertEqual(appcast.live_version, "10000.0.47.0.2")
        self.assertEqual(appcast.last_published, "20260701T120000Z")

        manifest = statuses["extensions/update-manifest.alpha.xml"]
        self.assertIn("agent=0.0.118.0", manifest.live_version)
        self.assertIn("bugreporter=54.0.0.0", manifest.live_version)
        self.assertIsNone(manifest.last_published)

        self.assertEqual(statuses["extensions/extensions.alpha.json"].live_version, "-")

        absent = statuses["appcast-win-arm64.xml"]
        self.assertIsNone(absent.live_version)
        self.assertIsNone(absent.last_published)

    def test_unpublishable_spec_refuses_publish_allows_dry_run(self):
        spec = replace(feed_by_key("appcast-claw.xml"), publishable=False)
        content = render_browser_appcast(
            spec, _artifact(), "0.47.0.2", "10000.0.47.0.2", "2026-06-19T06:41:33Z"
        )
        publisher = self._publisher()

        self.assertTrue(publisher.publish(spec, content))
        self.assertFalse(publisher.publish(spec, content, publish=True))
        self.assertEqual(self.client.calls, [])


if __name__ == "__main__":
    unittest.main()
