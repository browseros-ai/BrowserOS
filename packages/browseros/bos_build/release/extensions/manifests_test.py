#!/usr/bin/env python3
"""Tests for coherent extension-manifest generation."""

import unittest
from types import SimpleNamespace
from typing import cast

from ...core.context import Context
from ...core.step import ValidationError
from ..feeds.render import extract_manifest_versions, render_update_manifest
from .manifests import ExtensionsFeedModule, parse_set_options

AGENT_ID = "bflpfmnmnokmjhmgnolecpppdbdophmk"
BUGREPORTER_ID = "adlpneommgkgeanpaekgoaolcpncohkf"
BROWSERCLAW_ID = "pjimfkbpehlcllblajnpfamdfjhhlgkc"

LIVE_ALPHA_MANIFEST = render_update_manifest(
    {"agent": "0.0.117.0", "bugreporter": "54.0.0.0"}
)
LIVE_BUNDLED_MANIFEST = render_update_manifest(
    {"agent": "0.0.115.0", "bugreporter": "52.0.0.0", "browserclaw": "0.0.0.2"}
)


class FakePublisher:
    def __init__(self, live=None, head_status=200, refuse=()):
        self.live = dict(live or {})
        self.head_status = head_status
        self.refuse = set(refuse)
        self.calls = []
        self.head_calls = []
        self.stage_calls = []

    def fetch_live(self, key):
        return self.live.get(key)

    def http_head(self, url):
        self.head_calls.append(url)
        if isinstance(self.head_status, dict):
            return self.head_status.get(url, 200)
        return self.head_status

    def publish(self, spec, content, publish=False, allow_downgrade=False,
                verbose=True, stage=True):
        self.calls.append(
            SimpleNamespace(
                key=spec.key,
                content=content,
                publish=publish,
                allow_downgrade=allow_downgrade,
                verbose=verbose,
                stage=stage,
            )
        )
        return spec.key not in self.refuse

    def stage(self, spec, content):
        self.stage_calls.append(SimpleNamespace(key=spec.key, content=content))
        return f"/staged/{spec.key.rsplit('/', 1)[-1]}"


def _live_feeds():
    return {
        "extensions/update-manifest.alpha.xml": LIVE_ALPHA_MANIFEST,
        "extensions/bundled-manifest.xml": LIVE_BUNDLED_MANIFEST,
    }


class ExtensionsFeedModuleTest(unittest.TestCase):
    def _ctx(self):
        return cast(
            Context,
            SimpleNamespace(env=SimpleNamespace(has_r2_config=lambda: True)),
        )

    def _run(self, channel="alpha", set_versions=None, publisher=None, **kwargs):
        self.publisher = publisher or FakePublisher(live=_live_feeds())
        module = ExtensionsFeedModule(
            channel=channel,
            set_versions=set_versions or {},
            publisher=self.publisher,
            **kwargs,
        )
        module.execute(self._ctx())
        return module

    def test_regenerates_all_three_files_coherently(self):
        self._run(set_versions={"agent": "0.0.118.0"})

        self.assertEqual(
            [c.key for c in self.publisher.calls],
            [
                "extensions/update-manifest.alpha.xml",
                "extensions/extensions.alpha.json",
                "extensions/bundled-manifest.xml",
            ],
        )
        manifest, json_content, bundled = [c.content for c in self.publisher.calls]

        # Channel manifest: --set wins and other versions carry from live sources.
        self.assertEqual(
            extract_manifest_versions(manifest),
            {
                AGENT_ID: "0.0.118.0",
                BUGREPORTER_ID: "54.0.0.0",
                BROWSERCLAW_ID: "0.0.0.2",
            },
        )
        # JSON points every update-feed id at the channel manifest URL.
        self.assertIn(
            "https://cdn.browseros.com/extensions/update-manifest.alpha.xml",
            json_content,
        )
        self.assertIn(AGENT_ID, json_content)
        self.assertIn(BUGREPORTER_ID, json_content)
        self.assertIn(BROWSERCLAW_ID, json_content)
        # Bundled retains all resolved extension versions.
        self.assertEqual(
            extract_manifest_versions(bundled),
            {
                AGENT_ID: "0.0.118.0",
                BUGREPORTER_ID: "54.0.0.0",
                BROWSERCLAW_ID: "0.0.0.2",
            },
        )

    def test_carry_over_without_set_uses_live_versions(self):
        self._run()

        manifest = self.publisher.calls[0].content
        self.assertEqual(
            extract_manifest_versions(manifest),
            {
                AGENT_ID: "0.0.117.0",
                BUGREPORTER_ID: "54.0.0.0",
                BROWSERCLAW_ID: "0.0.0.2",
            },
        )

    def test_bundled_never_regresses_below_live_on_channel_run(self):
        # Alpha runs push bundled ahead; a prod run must neither downgrade
        # bundled nor need --allow-downgrade to proceed.
        publisher = FakePublisher(
            live={
                "extensions/update-manifest.xml": render_update_manifest(
                    {"agent": "0.0.118.0", "bugreporter": "54.0.0.0"}
                ),
                "extensions/bundled-manifest.xml": render_update_manifest(
                    {
                        "agent": "0.0.119.0",
                        "bugreporter": "54.0.0.0",
                        "browserclaw": "0.0.0.2",
                    }
                ),
            }
        )

        self._run(channel="prod", publisher=publisher, publish=True)

        manifest = publisher.calls[0].content
        bundled = publisher.calls[2].content
        self.assertEqual(
            extract_manifest_versions(manifest)[AGENT_ID], "0.0.118.0"
        )
        self.assertEqual(
            extract_manifest_versions(manifest)[BROWSERCLAW_ID], "0.0.0.2"
        )
        self.assertEqual(extract_manifest_versions(bundled)[AGENT_ID], "0.0.119.0")
        # Both agent crx versions are referenced somewhere — both checked.
        self.assertIn(
            "https://cdn.browseros.com/extensions/agent-0.0.118.0.crx",
            publisher.head_calls,
        )
        self.assertIn(
            "https://cdn.browseros.com/extensions/agent-0.0.119.0.crx",
            publisher.head_calls,
        )

    def test_allow_downgrade_forces_bundled_to_run_versions(self):
        publisher = FakePublisher(
            live={
                "extensions/update-manifest.xml": render_update_manifest(
                    {"agent": "0.0.118.0", "bugreporter": "54.0.0.0"}
                ),
                "extensions/bundled-manifest.xml": render_update_manifest(
                    {
                        "agent": "0.0.119.0",
                        "bugreporter": "54.0.0.0",
                        "browserclaw": "0.0.0.2",
                    }
                ),
            }
        )

        self._run(channel="prod", publisher=publisher, allow_downgrade=True)

        bundled = publisher.calls[2].content
        self.assertEqual(extract_manifest_versions(bundled)[AGENT_ID], "0.0.118.0")

    def test_missing_version_with_no_live_source_raises(self):
        publisher = FakePublisher(live={})

        with self.assertRaisesRegex(RuntimeError, "agent"):
            self._run(publisher=publisher, set_versions={"bugreporter": "54.0.0.0"})

        self.assertEqual(publisher.calls, [])

    def test_missing_crx_refuses_before_any_write(self):
        url = "https://cdn.browseros.com/extensions/agent-0.0.118.0.crx"
        publisher = FakePublisher(live=_live_feeds(), head_status={url: 404})

        with self.assertRaisesRegex(RuntimeError, "agent-0.0.118.0.crx"):
            self._run(publisher=publisher, set_versions={"agent": "0.0.118.0"})

        self.assertEqual(publisher.calls, [])
        self.assertIn(url, publisher.head_calls)

    def test_every_crx_in_output_head_checked(self):
        self._run(set_versions={"agent": "0.0.118.0"})

        expected = {
            "https://cdn.browseros.com/extensions/agent-0.0.118.0.crx",
            "https://cdn.browseros.com/extensions/bugreporter-54.0.0.0.crx",
            "https://cdn.browseros.com/extensions/browserclaw-0.0.0.2.crx",
        }
        self.assertEqual(set(self.publisher.head_calls), expected)

    def test_dry_run_is_default_and_runs_single_pass(self):
        self._run(set_versions={"agent": "0.0.118.0"})

        self.assertEqual(len(self.publisher.calls), 3)
        self.assertTrue(all(not c.publish for c in self.publisher.calls))
        self.assertTrue(all(c.verbose for c in self.publisher.calls))
        self.assertTrue(all(not c.stage for c in self.publisher.calls))
        self.assertEqual(
            [c.key for c in self.publisher.stage_calls],
            [
                "extensions/update-manifest.alpha.xml",
                "extensions/extensions.alpha.json",
                "extensions/bundled-manifest.xml",
            ],
        )

    def test_publish_preflights_all_three_before_writing(self):
        self._run(
            set_versions={"agent": "0.0.118.0"}, publish=True, allow_downgrade=True
        )

        # Quiet preflight over the trio, then the writing pass.
        self.assertEqual(
            [(c.publish, c.verbose) for c in self.publisher.calls],
            [(False, False)] * 3 + [(True, True)] * 3,
        )
        self.assertTrue(all(c.allow_downgrade for c in self.publisher.calls))
        self.assertTrue(all(not c.stage for c in self.publisher.calls[:3]))

    def test_refused_preflight_blocks_every_write(self):
        # The bundled manifest can hold a newer version from an earlier alpha
        # run; its guard refusal must abort BEFORE any file is written.
        publisher = FakePublisher(
            live=_live_feeds(), refuse={"extensions/bundled-manifest.xml"}
        )

        with self.assertRaisesRegex(
            RuntimeError, "extensions/bundled-manifest.xml"
        ):
            self._run(publisher=publisher, publish=True)

        self.assertTrue(all(not c.publish for c in publisher.calls))
        self.assertEqual(publisher.stage_calls, [])

    def test_refused_late_dry_run_does_not_stage_earlier_files(self):
        publisher = FakePublisher(
            live=_live_feeds(), refuse={"extensions/bundled-manifest.xml"}
        )

        with self.assertRaisesRegex(
            RuntimeError, "extensions/bundled-manifest.xml"
        ):
            self._run(publisher=publisher)

        self.assertEqual(
            [c.key for c in publisher.calls],
            [
                "extensions/update-manifest.alpha.xml",
                "extensions/extensions.alpha.json",
                "extensions/bundled-manifest.xml",
            ],
        )
        self.assertEqual(publisher.stage_calls, [])

    def test_refused_publish_aborts_remaining_files(self):
        publisher = FakePublisher(
            live=_live_feeds(), refuse={"extensions/update-manifest.alpha.xml"}
        )

        with self.assertRaisesRegex(
            RuntimeError, "extensions/update-manifest.alpha.xml"
        ):
            self._run(publisher=publisher)

        self.assertEqual(len(publisher.calls), 1)

    def test_validate_rejects_unknown_extension_and_bad_channel(self):
        ctx = self._ctx()

        with self.assertRaisesRegex(ValidationError, "nope"):
            ExtensionsFeedModule(
                channel="alpha", set_versions={"nope": "1.0"}
            ).validate(ctx)

        with self.assertRaisesRegex(ValidationError, "channel"):
            ExtensionsFeedModule(channel="beta", set_versions={}).validate(ctx)


class ParseSetOptionsTest(unittest.TestCase):
    def test_parses_name_version_pairs(self):
        self.assertEqual(
            parse_set_options(["agent=0.0.118.0", "bugreporter=54.0.0.0"]),
            {"agent": "0.0.118.0", "bugreporter": "54.0.0.0"},
        )

    def test_malformed_entry_raises(self):
        with self.assertRaisesRegex(ValueError, "name=version"):
            parse_set_options(["agent0.0.118.0"])

    def test_empty_version_raises(self):
        with self.assertRaisesRegex(ValueError, "name=version"):
            parse_set_options(["agent="])


if __name__ == "__main__":
    unittest.main()
