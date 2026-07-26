#!/usr/bin/env python3
"""CLI tests for product-aware server appcast publication."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from typer.testing import CliRunner

from bos_build.browseros import app
from bos_build.cli import ota as ota_cli
from bos_build.release.feeds.publisher import FeedPublisher
from bos_build.release.feeds.spec import server_feed

runner = CliRunner()


class _MissingR2Client:
    class exceptions:
        class NoSuchKey(Exception):
            pass

    def get_object(self, Bucket, Key):
        raise self.exceptions.NoSuchKey(Key)


def _appcast(bundle_id: str, channel: str, version: str | None = None) -> str:
    spec = server_feed(bundle_id, channel)
    item = ""
    if version is not None:
        item = f"""
    <item>
      <sparkle:version>{version}</sparkle:version>
    </item>
"""
    return f"""\
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>{spec.title}</title>
    <link>{spec.link}</link>{item}
  </channel>
</rss>
"""


def _combined_output(result) -> str:
    output = result.output
    try:
        output += result.stderr
    except (AttributeError, ValueError):
        pass
    return output


class ReleaseAppcastCliTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.paths = {
            "browseros-server": self.root / "appcast-server.xml",
            "browserclaw-server": self.root / "appcast-claw-server.xml",
        }

    def _get_appcast_path(self, channel: str, bundle_id: str) -> Path:
        self.assertEqual(channel, "prod")
        return self.paths[bundle_id]

    def _publisher(self) -> FeedPublisher:
        return FeedPublisher(
            env=SimpleNamespace(r2_bucket="browseros"),
            r2_client=_MissingR2Client(),
            http_head=lambda _: 200,
            appcast_staging_dir=self.root / "staged",
        )

    def _invoke(self, *args: str):
        return runner.invoke(
            app, ["ota", "server", "release-appcast", "--channel", "prod", *args]
        )

    def test_empty_default_feed_names_selection_and_hints_browserclaw(self):
        self.paths["browseros-server"].write_text(
            _appcast("browseros-server", "prod")
        )
        self.paths["browserclaw-server"].write_text(
            _appcast("browserclaw-server", "prod", "0.0.12")
        )

        with (
            mock.patch.object(
                ota_cli, "get_appcast_path", side_effect=self._get_appcast_path
            ),
            mock.patch.object(ota_cli, "_feed_publisher") as publisher_factory,
        ):
            result = self._invoke("--publish")

        output = _combined_output(result)
        self.assertEqual(result.exit_code, 1, output)
        self.assertIn(
            "Resolved appcast: product=browseros bundle=browseros-server "
            "channel=prod spec=appcast-server.xml "
            f"source={self.paths['browseros-server'].resolve()}",
            output,
        )
        self.assertIn(
            f"appcast-server.xml has no <item> entries: "
            f"{self.paths['browseros-server'].resolve()}",
            output,
        )
        self.assertIn(
            "Did you mean --product browserclaw? "
            "appcast-claw-server.xml carries 0.0.12",
            output,
        )
        publisher_factory.assert_not_called()

    def test_missing_sibling_feed_does_not_mask_empty_error(self):
        self.paths["browseros-server"].write_text(
            _appcast("browseros-server", "prod")
        )

        with (
            mock.patch.object(
                ota_cli, "get_appcast_path", side_effect=self._get_appcast_path
            ),
            mock.patch.object(ota_cli, "_feed_publisher") as publisher_factory,
        ):
            result = self._invoke()

        output = _combined_output(result)
        self.assertEqual(result.exit_code, 1, output)
        self.assertIn("appcast-server.xml has no <item> entries", output)
        self.assertNotIn("Did you mean --product", output)
        publisher_factory.assert_not_called()

    def test_explicit_browserclaw_uses_versioned_claw_feed(self):
        self.paths["browserclaw-server"].write_text(
            _appcast("browserclaw-server", "prod", "0.0.12")
        )

        with (
            mock.patch.object(
                ota_cli, "get_appcast_path", side_effect=self._get_appcast_path
            ),
            mock.patch.object(
                ota_cli, "_feed_publisher", side_effect=self._publisher
            ),
        ):
            result = self._invoke("--product", "browserclaw")

        output = _combined_output(result)
        self.assertEqual(result.exit_code, 0, output)
        self.assertIn(
            "Resolved appcast: product=browserclaw bundle=browserclaw-server "
            "channel=prod spec=appcast-claw-server.xml "
            f"source={self.paths['browserclaw-server'].resolve()}",
            output,
        )
        self.assertNotIn("no sparkle:version", output)

    def test_custom_empty_file_does_not_suggest_another_product(self):
        custom = self.root / "custom.xml"
        custom.write_text(_appcast("browseros-server", "prod"))
        self.paths["browserclaw-server"].write_text(
            _appcast("browserclaw-server", "prod", "0.0.12")
        )

        with mock.patch.object(ota_cli, "_feed_publisher") as publisher_factory:
            result = self._invoke("--file", str(custom))

        output = _combined_output(result)
        self.assertEqual(result.exit_code, 1, output)
        self.assertIn(f"source={custom.resolve()}", output)
        self.assertNotIn("Did you mean --product", output)
        publisher_factory.assert_not_called()


if __name__ == "__main__":
    unittest.main()
