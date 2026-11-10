#!/usr/bin/env python3
"""Tests for immutable R2 component allocation probes."""

import unittest

from bos_build.release.r2_allocations import discover_r2_component_allocation


SOURCE_SHA = "1" * 40
SHA256 = "a" * 64


class FakeR2Client:
    def __init__(self, pages, metadata):
        self.pages = pages
        self.metadata = metadata
        self.list_calls = []
        self.head_calls = []

    def list_objects_v2(self, **request):
        self.list_calls.append(request)
        token = request.get("ContinuationToken")
        return self.pages[token]

    def head_object(self, *, Bucket, Key):
        self.head_calls.append((Bucket, Key))
        return {"Metadata": self.metadata[Key]}


class R2AllocationDiscoveryTest(unittest.TestCase):
    def test_each_supported_component_probes_only_the_requested_version(self) -> None:
        cases = {
            "claw-server-rust": (
                "0.0.30",
                "claw-server-rust/prod-resources/0.0.30/",
            ),
            "claw-onboard": (
                "0.0.20",
                "claw-onboard/prod-resources/0.0.20/",
            ),
            "browserclaw": (
                "0.0.9.0",
                "extensions/browserclaw-0.0.9.0.crx",
            ),
        }
        for component, (version, prefix) in cases.items():
            client = FakeR2Client({None: {"Contents": [], "IsTruncated": False}}, {})
            with self.subTest(component=component):
                allocation = discover_r2_component_allocation(
                    client, "browseros", component, version, SOURCE_SHA
                )
                self.assertIsNone(allocation)
                self.assertEqual(
                    client.list_calls,
                    [{"Bucket": "browseros", "Prefix": prefix}],
                )
        for retired in ("server", "agent", "app-onboard"):
            client = FakeR2Client({None: {"Contents": [], "IsTruncated": False}}, {})
            with self.subTest(component=retired):
                with self.assertRaisesRegex(ValueError, "Unknown component"):
                    discover_r2_component_allocation(
                        client, "browseros", retired, "0.0.1", SOURCE_SHA
                    )
                self.assertEqual(client.list_calls, [])

    def test_server_probe_ignores_out_of_band_versions_and_blocks_mismatch(
        self,
    ) -> None:
        stale_key = "claw-server-rust/prod-resources/0.0.129/browseros-claw-server-rust-resources-linux-x64.zip"
        out_of_band_key = (
            "claw-server-rust/prod-resources/99.0.0/browseros-claw-server-rust-resources-linux-x64.zip"
        )
        client = FakeR2Client(
            {
                None: {
                    "Contents": [{"Key": stale_key}, {"Key": out_of_band_key}],
                    "IsTruncated": False,
                }
            },
            {
                stale_key: {
                    "component": "legacy/server",
                    "release-sha": SOURCE_SHA,
                    "sha256": SHA256,
                    "target": "linux-x64",
                    "version": "0.0.129",
                },
            },
        )

        allocation = discover_r2_component_allocation(
            client, "browseros", "claw-server-rust", "0.0.129", SOURCE_SHA
        )

        self.assertIsNotNone(allocation)
        assert allocation is not None
        self.assertEqual(allocation.version, "0.0.129")
        self.assertFalse(allocation.reusable)
        self.assertEqual(allocation.kind, "resource")
        self.assertEqual(client.head_calls, [("browseros", stale_key)])
        self.assertEqual(
            client.list_calls,
            [{"Bucket": "browseros", "Prefix": "claw-server-rust/prod-resources/0.0.129/"}],
        )

    def test_partial_server_retry_reuses_exact_source_binding(self) -> None:
        key = "claw-server-rust/prod-resources/0.0.130/browseros-claw-server-rust-resources-darwin-arm64.zip"
        client = FakeR2Client(
            {None: {"Contents": [{"Key": key}], "IsTruncated": False}},
            {
                key: {
                    "component": "claw-server-rust/prod-resources",
                    "release-sha": SOURCE_SHA,
                    "sha256": SHA256,
                    "target": "darwin-arm64",
                    "version": "0.0.130",
                }
            },
        )

        allocation = discover_r2_component_allocation(
            client, "browseros", "claw-server-rust", "0.0.130", SOURCE_SHA
        )

        self.assertIsNotNone(allocation)
        assert allocation is not None
        self.assertTrue(allocation.reusable)
        self.assertEqual(allocation.source_sha, SOURCE_SHA)
        self.assertEqual(allocation.reference, "claw-server/v0.0.130")

    def test_extension_probe_paginates_and_validates_source_binding(self) -> None:
        key = "extensions/browserclaw-0.0.42.0.crx"
        client = FakeR2Client(
            {
                None: {
                    "Contents": [{"Key": key}],
                    "IsTruncated": True,
                    "NextContinuationToken": "next",
                },
                "next": {
                    "Contents": [{"Key": "extensions/update-manifest.xml"}],
                    "IsTruncated": False,
                },
            },
            {
                key: {
                    "binding-schema": "browseros-extension-crx-v1",
                    "extension": "browserclaw",
                    "source-sha": SOURCE_SHA,
                    "sha256": SHA256,
                    "version": "0.0.42.0",
                }
            },
        )

        allocation = discover_r2_component_allocation(
            client, "browseros", "browserclaw", "0.0.42.0", SOURCE_SHA
        )

        self.assertIsNotNone(allocation)
        assert allocation is not None
        self.assertTrue(allocation.reusable)
        self.assertEqual(
            client.list_calls,
            [
                {"Bucket": "browseros", "Prefix": key},
                {
                    "Bucket": "browseros",
                    "Prefix": key,
                    "ContinuationToken": "next",
                },
            ],
        )


if __name__ == "__main__":
    unittest.main()
