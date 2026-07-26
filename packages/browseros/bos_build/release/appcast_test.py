#!/usr/bin/env python3
"""Tests for the full-file browser appcast module."""

import unittest
from dataclasses import replace
from types import SimpleNamespace
from typing import cast
from unittest.mock import patch

from ..core.context import Context
from ..core.step import ValidationError
from .appcast import AppcastModule
from .feeds.spec import browser_feeds_for_product


def _artifact(filename: str) -> dict:
    return {
        "filename": filename,
        "url": f"https://cdn.browseros.com/releases/browseros/0.47.0.2/{filename}",
        "sparkle_signature": "SIG==",
        "sparkle_length": 100,
    }


def _macos_release(*artifact_keys: str) -> dict:
    names = {
        "arm64": "BrowserOS_v0.47.0.2_arm64.dmg",
        "x64": "BrowserOS_v0.47.0.2_x86_64.dmg",
        "universal": "BrowserOS_v0.47.0.2_universal.dmg",
    }
    return {
        "sparkle_version": "10000.0.47.0.2",
        "build_date": "2026-06-19T06:41:33Z",
        "artifacts": {key: _artifact(names[key]) for key in artifact_keys},
    }


def _win_release(*artifact_keys: str) -> dict:
    names = {
        "x64_installer": "BrowserOS_v0.47.0.2_x64_installer.exe",
        "arm64_installer": "BrowserOS_v0.47.0.2_arm64_installer.exe",
    }
    return {
        "sparkle_version": "10000.0.47.0.2",
        "build_date": "2026-06-19T06:41:33Z",
        "artifacts": {key: _artifact(names[key]) for key in artifact_keys},
    }


class FakePublisher:
    def __init__(self, refuse=()):
        self.calls = []
        self.refuse = set(refuse)

    def publish(self, spec, content, publish=False, allow_downgrade=False):
        self.calls.append(
            SimpleNamespace(
                key=spec.key,
                content=content,
                publish=publish,
                allow_downgrade=allow_downgrade,
            )
        )
        return spec.key not in self.refuse


class AppcastModuleTest(unittest.TestCase):
    def _ctx(self, version="0.47.0.2"):
        return cast(
            Context,
            SimpleNamespace(
                release_version=version,
                env=SimpleNamespace(has_r2_config=lambda: True),
            ),
        )

    def _module(self, metadata, publisher=None, **kwargs):
        self.publisher = publisher or FakePublisher()
        self.fetch_calls = []

        def fetch(version, env, product_id):
            self.fetch_calls.append((version, product_id))
            return metadata

        return AppcastModule(
            publisher=self.publisher, fetch_metadata=fetch, **kwargs
        )

    def test_mac_arm64_only_feeds_appcast_xml_and_skips_x86_feed(self):
        module = self._module({"macos": _macos_release("arm64")})

        module.execute(self._ctx())

        self.assertEqual([c.key for c in self.publisher.calls], ["appcast.xml"])
        self.assertIn("BrowserOS_v0.47.0.2_arm64.dmg", self.publisher.calls[0].content)

    def test_universal_wins_over_arm64_for_appcast_xml(self):
        module = self._module(
            {"macos": _macos_release("arm64", "x64", "universal")}
        )

        module.execute(self._ctx())

        by_key = {c.key: c for c in self.publisher.calls}
        self.assertIn("universal.dmg", by_key["appcast.xml"].content)
        self.assertIn("x86_64.dmg", by_key["appcast-x86_64.xml"].content)

    def test_win_feeds_render_per_installer_arch(self):
        module = self._module({"win": _win_release("x64_installer")})

        module.execute(self._ctx())

        self.assertEqual([c.key for c in self.publisher.calls], ["appcast-win.xml"])
        self.assertIn("x64_installer.exe", self.publisher.calls[0].content)

    def test_publish_and_allow_downgrade_thread_through(self):
        module = self._module(
            {"macos": _macos_release("arm64")}, publish=True, allow_downgrade=True
        )

        module.execute(self._ctx())

        self.assertTrue(self.publisher.calls[0].publish)
        self.assertTrue(self.publisher.calls[0].allow_downgrade)

    def test_no_metadata_at_all_raises(self):
        module = self._module({})

        with self.assertRaisesRegex(RuntimeError, "No release metadata"):
            module.execute(self._ctx())

    def test_refused_feed_raises_naming_the_key(self):
        module = self._module(
            {"macos": _macos_release("arm64")},
            publisher=FakePublisher(refuse={"appcast.xml"}),
        )

        with self.assertRaisesRegex(RuntimeError, "appcast.xml"):
            module.execute(self._ctx())

    def test_missing_sparkle_signature_fails_the_feed(self):
        release = _macos_release("arm64")
        del release["artifacts"]["arm64"]["sparkle_signature"]
        module = self._module({"macos": release})

        with self.assertRaisesRegex(RuntimeError, "appcast.xml"):
            module.execute(self._ctx())

        self.assertEqual(self.publisher.calls, [])

    def test_fetch_uses_module_product(self):
        module = self._module(
            {"macos": _macos_release("arm64")}, product_id="browserclaw"
        )

        module.execute(self._ctx())

        self.assertEqual(self.fetch_calls, [("0.47.0.2", "browserclaw")])
        self.assertEqual(
            [c.key for c in self.publisher.calls], ["appcast-claw.xml"]
        )

    def test_validate_refuses_publishing_unpublishable_feed(self):
        # Every real product now has a client, so synthesize an unpublishable
        # feed set to exercise the gate itself.
        unpublishable = tuple(
            replace(feed, publishable=False)
            for feed in browser_feeds_for_product("browseros")
        )
        module = AppcastModule(product_id="browseros", publish=True)

        with patch(
            "bos_build.release.appcast.browser_feeds_for_product",
            return_value=unpublishable,
        ):
            with self.assertRaisesRegex(ValidationError, "not publishable"):
                module.validate(self._ctx())

    def test_validate_requires_version(self):
        module = AppcastModule()

        with self.assertRaises(ValidationError):
            module.validate(self._ctx(version=""))


if __name__ == "__main__":
    unittest.main()
