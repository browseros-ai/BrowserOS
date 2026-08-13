#!/usr/bin/env python3
"""Tests for immutable R2 component allocation discovery."""

import unittest

from bos_build.release.components import AllocationRecord
from bos_build.release.r2_allocations import discover_r2_component_allocations


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


def _draft(component: str, version: str, tag: str) -> AllocationRecord:
    return AllocationRecord(
        component=component,
        version=version,
        kind="release",
        source_sha=SOURCE_SHA,
        reference=tag,
        reusable=True,
    )


class R2AllocationDiscoveryTest(unittest.TestCase):
    def test_each_supported_component_lists_only_its_immutable_namespace(self) -> None:
        cases = {
            "server": "artifacts/server/",
            "claw-server-rust": "claw-server-rust/prod-resources/",
            "claw-onboard": "claw-onboard/prod-resources/",
            "agent": "extensions/agent-",
            "browserclaw": "extensions/browserclaw-",
        }
        for component, prefix in cases.items():
            client = FakeR2Client(
                {None: {"Contents": [], "IsTruncated": False}}, {}
            )
            with self.subTest(component=component):
                allocations = discover_r2_component_allocations(
                    client, "browseros", component, SOURCE_SHA, ()
                )
                self.assertEqual(allocations, ())
                self.assertEqual(
                    client.list_calls,
                    [{"Bucket": "browseros", "Prefix": prefix}],
                )

    def test_server_objects_block_stale_versions_and_reuse_matching_retries(
        self,
    ) -> None:
        stale_key = "artifacts/server/0.0.129/browseros-server-resources-linux-x64.zip"
        retry_key = (
            "artifacts/server/0.0.130/browseros-server-resources-darwin-arm64.zip"
        )
        client = FakeR2Client(
            {
                None: {
                    "Contents": [{"Key": stale_key}, {"Key": retry_key}],
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
                retry_key: {
                    "component": "artifacts/server",
                    "release-sha": SOURCE_SHA,
                    "sha256": SHA256,
                    "target": "darwin-arm64",
                    "version": "0.0.130",
                },
            },
        )

        allocations = discover_r2_component_allocations(
            client,
            "browseros",
            "server",
            SOURCE_SHA,
            (
                _draft("server", "0.0.129", "agent-server/v0.0.129"),
                _draft("server", "0.0.130", "agent-server/v0.0.130"),
            ),
        )

        self.assertEqual(
            [record.version for record in allocations], ["0.0.129", "0.0.130"]
        )
        self.assertFalse(allocations[0].reusable)
        self.assertEqual(allocations[0].kind, "resource")
        self.assertTrue(allocations[1].reusable)
        self.assertEqual(allocations[1].source_sha, SOURCE_SHA)
        self.assertEqual(allocations[1].reference, "agent-server/v0.0.130")
        self.assertEqual(
            client.head_calls,
            [("browseros", stale_key), ("browseros", retry_key)],
        )

    def test_occupied_versions_without_a_matching_draft_do_not_need_heads(self) -> None:
        key = "artifacts/server/0.0.200/browseros-server-resources-linux-x64.zip"
        client = FakeR2Client(
            {None: {"Contents": [{"Key": key}], "IsTruncated": False}},
            {},
        )

        allocations = discover_r2_component_allocations(
            client, "browseros", "server", SOURCE_SHA, ()
        )

        self.assertEqual(len(allocations), 1)
        self.assertFalse(allocations[0].reusable)
        self.assertEqual(client.head_calls, [])

    def test_extension_listing_paginates_and_validates_source_binding(self) -> None:
        key = "extensions/agent-0.0.42.0.crx"
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
                    "extension": "agent",
                    "source-sha": SOURCE_SHA,
                    "sha256": SHA256,
                    "version": "0.0.42.0",
                }
            },
        )

        allocations = discover_r2_component_allocations(
            client,
            "browseros",
            "agent",
            SOURCE_SHA,
            (_draft("agent", "0.0.42.0", "ext-agent/v0.0.42.0"),),
        )

        self.assertEqual(len(allocations), 1)
        self.assertTrue(allocations[0].reusable)
        self.assertEqual(
            client.list_calls,
            [
                {"Bucket": "browseros", "Prefix": "extensions/agent-"},
                {
                    "Bucket": "browseros",
                    "Prefix": "extensions/agent-",
                    "ContinuationToken": "next",
                },
            ],
        )


if __name__ == "__main__":
    unittest.main()
