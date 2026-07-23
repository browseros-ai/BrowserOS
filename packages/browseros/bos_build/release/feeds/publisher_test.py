#!/usr/bin/env python3
"""Tests for the rails-enforcing feed publisher (all R2/HTTP faked)."""

import io
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
    render_browser_appcast,
    render_extensions_json,
    render_server_appcast,
    render_update_manifest,
)
from .spec import all_feeds, feed_by_key, server_feed, update_manifest_feed

FIXED_NOW = datetime(2026, 7, 1, 12, 0, 0, tzinfo=timezone.utc)


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


def _artifact(url="https://cdn.browseros.com/releases/browseros/0.47.0.2/macos/BrowserOS_v0.47.0.2_arm64.dmg"):
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


class PublisherTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        root = Path(self._tmp.name)
        self.appcast_staging = root / "config" / "appcast"
        self.extensions_staging = root / "updates" / "extensions"
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
            appcast_staging_dir=self.appcast_staging,
            extensions_staging_dir=self.extensions_staging,
            now=lambda: FIXED_NOW,
        )

    def test_dry_run_stages_without_r2_writes(self):
        publisher = self._publisher(
            {"appcast.xml": _mac_appcast("10000.0.46.0.0").encode()}
        )

        ok = publisher.publish(feed_by_key("appcast.xml"), _mac_appcast())

        self.assertTrue(ok)
        self.assertEqual(self.client.calls, [])
        staged = self.appcast_staging / "appcast.xml"
        self.assertEqual(staged.read_text(), _mac_appcast())

    def test_dry_run_stage_false_does_not_write_staging_file(self):
        publisher = self._publisher(
            {"appcast.xml": _mac_appcast("10000.0.46.0.0").encode()}
        )

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast(), stage=False
        )

        self.assertTrue(ok)
        self.assertEqual(self.client.calls, [])
        self.assertFalse((self.appcast_staging / "appcast.xml").exists())

    def test_publish_backs_up_live_before_put(self):
        live = _mac_appcast("10000.0.46.0.0").encode()
        publisher = self._publisher({"appcast.xml": live})

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast(), publish=True
        )

        self.assertTrue(ok)
        self.assertEqual(
            self.client.calls,
            [
                ("copy", "appcast.xml", "feeds-history/appcast.xml.20260701T120000Z"),
                ("put", "appcast.xml", "application/xml"),
            ],
        )
        staged = self.appcast_staging / "appcast.xml"
        self.assertEqual(staged.read_text(), _mac_appcast())

    def test_publish_without_live_object_skips_backup(self):
        publisher = self._publisher()

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast(), publish=True
        )

        self.assertTrue(ok)
        self.assertEqual(
            self.client.calls, [("put", "appcast.xml", "application/xml")]
        )

    def test_downgrade_refused_without_flag(self):
        publisher = self._publisher(
            {"appcast.xml": _mac_appcast("10000.0.48.0.0").encode()}
        )

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast("10000.0.47.0.2"), publish=True
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])
        self.assertFalse(self.appcast_staging.exists())

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

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast(), publish=True
        )

        self.assertTrue(ok)

    def test_legacy_scheme_live_version_is_older(self):
        legacy = _mac_appcast().replace("10000.0.47.0.2", "7948.97")
        publisher = self._publisher({"appcast.xml": legacy.encode()})

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast(), publish=True
        )

        self.assertTrue(ok)

    def test_channel_metadata_mismatch_refused(self):
        # The alpha→prod byte-copy bug: alpha-rendered content on the prod key.
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

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast(), publish=True
        )

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

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast(), publish=True
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_manifest_entry_removal_refused_without_flag(self):
        spec = update_manifest_feed("alpha")
        live = render_update_manifest(
            {"agent": "0.0.118.0", "bugreporter": "54.0.0.0"}
        )
        publisher = self._publisher({spec.key: live.encode()})

        only_agent = render_update_manifest({"agent": "0.0.118.0"})
        self.assertFalse(publisher.publish(spec, only_agent, publish=True))
        self.assertEqual(self.client.calls, [])

        self.assertTrue(
            publisher.publish(spec, only_agent, publish=True, allow_downgrade=True)
        )

    def test_update_manifest_guards_per_extension(self):
        spec = update_manifest_feed("alpha")
        live = render_update_manifest(
            {"agent": "0.0.118.0", "bugreporter": "54.0.0.0"}
        )
        publisher = self._publisher({spec.key: live.encode()})

        downgraded = render_update_manifest(
            {"agent": "0.0.117.0", "bugreporter": "54.0.0.0"}
        )
        self.assertFalse(publisher.publish(spec, downgraded, publish=True))
        self.assertEqual(self.client.calls, [])

        upgraded = render_update_manifest(
            {"agent": "0.0.119.0", "bugreporter": "54.0.0.0"}
        )
        self.assertTrue(publisher.publish(spec, upgraded, publish=True))

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
        staged = self.extensions_staging / "extensions.alpha.json"
        self.assertTrue(staged.exists())

    def test_malformed_new_content_refused(self):
        publisher = self._publisher()

        ok = publisher.publish(
            feed_by_key("appcast.xml"), "<rss><channel>", publish=True
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_empty_appcast_refused_with_item_error(self):
        publisher = self._publisher()

        with mock.patch(
            "bos_build.release.feeds.publisher.log_error"
        ) as log_error:
            ok = publisher.publish(
                feed_by_key("appcast.xml"), _empty_mac_appcast(), publish=True
            )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])
        log_error.assert_called_with(
            "appcast.xml: new content has no <item> entries"
        )

    def test_versionless_appcast_refused_with_version_error(self):
        content = _mac_appcast().replace(
            "<sparkle:version>10000.0.47.0.2</sparkle:version>", ""
        )
        publisher = self._publisher()

        with mock.patch(
            "bos_build.release.feeds.publisher.log_error"
        ) as log_error:
            ok = publisher.publish(
                feed_by_key("appcast.xml"), content, publish=True
            )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])
        log_error.assert_called_with(
            "appcast.xml: new content has <item> entries but no sparkle:version"
        )

    def test_live_guard_distinguishes_empty_appcast(self):
        publisher = self._publisher()

        with mock.patch(
            "bos_build.release.feeds.publisher.log_error"
        ) as log_error:
            ok = publisher._guard_appcast_version(
                feed_by_key("appcast.xml"),
                _empty_mac_appcast(),
                _mac_appcast(),
                False,
            )

        self.assertFalse(ok)
        log_error.assert_called_with(
            "appcast.xml: new content has no <item> entries"
        )

    def test_live_guard_distinguishes_versionless_items(self):
        content = _mac_appcast().replace(
            "<sparkle:version>10000.0.47.0.2</sparkle:version>", ""
        )
        publisher = self._publisher()

        with mock.patch(
            "bos_build.release.feeds.publisher.log_error"
        ) as log_error:
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

        ok = publisher.publish(
            feed_by_key("appcast.xml"), _mac_appcast(), publish=True
        )

        self.assertFalse(ok)
        self.assertEqual(self.client.calls, [])

    def test_unparseable_live_replaced_with_flag_after_backup(self):
        publisher = self._publisher({"appcast.xml": b"garbage <not xml"})

        ok = publisher.publish(
            feed_by_key("appcast.xml"),
            _mac_appcast(),
            publish=True,
            allow_downgrade=True,
        )

        self.assertTrue(ok)
        self.assertEqual(self.client.calls[0][0], "copy")
        self.assertEqual(self.client.calls[1][0], "put")

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

        self.assertEqual(
            statuses["extensions/extensions.alpha.json"].live_version, "-"
        )

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
