#!/usr/bin/env python3
"""Tests for standalone component release resolution."""

import unittest
import subprocess
import tempfile
from dataclasses import replace
from pathlib import Path
from unittest import mock

from bos_build.release.component_release import (
    GitComponentReleaseOperations,
    StandaloneReleaseRequest,
    TagState,
    resolve_standalone_release,
)
from bos_build.release.candidate import CandidateRecord
from bos_build.release.components import AllocationRecord
from bos_build.release.suite_test import STATE_SHA, suite_record


SOURCE_SHA = "1" * 40


class FakeOperations:
    remote = "origin"

    def __init__(self) -> None:
        self.version = "0.0.127"
        self.records = []
        self.tags = {}
        self.ancestor = True
        self.synced = []
        self.resource_records = {}
        self.resource_probes = []

    def sync(self, default_branch: str) -> None:
        self.synced.append(default_branch)

    def resolve_commit(self, ref: str) -> str:
        return SOURCE_SHA

    def read_version(self, component: str, ref: str) -> str:
        return self.version

    def tag_state(self, tag: str):
        return self.tags.get(tag)

    def is_default_branch_ancestor(self, sha: str, default_branch: str) -> bool:
        return self.ancestor

    def allocations(self, component: str):
        return self.records

    def resource_allocation(self, component: str, version: str, source_sha: str):
        self.resource_probes.append((component, version, source_sha))
        return self.resource_records.get(version)


def _request(**overrides) -> StandaloneReleaseRequest:
    values = {
        "component": "server",
        "event_name": "workflow_dispatch",
        "default_branch": "main",
        "release_ref": "HEAD",
    }
    values.update(overrides)
    return StandaloneReleaseRequest(**values)


class StandaloneReleaseTest(unittest.TestCase):
    def test_uses_committed_unpublished_version(self) -> None:
        operations = FakeOperations()

        record = resolve_standalone_release(_request(), operations)

        self.assertEqual(record.version, "0.0.127")
        self.assertEqual(record.tag, "agent-server/v0.0.127")
        self.assertEqual(record.release_sha, SOURCE_SHA)
        self.assertEqual(record.reservation, "create")
        self.assertEqual(operations.synced, ["main"])

    def test_schedule_uses_the_called_workflow_checkout(self) -> None:
        operations = FakeOperations()

        record = resolve_standalone_release(_request(event_name="schedule"), operations)

        self.assertEqual(record.release_sha, SOURCE_SHA)
        self.assertEqual(record.tag, "agent-server/v0.0.127")

    def test_skips_tags_and_open_candidate_reservations(self) -> None:
        operations = FakeOperations()
        operations.records = [
            AllocationRecord(
                component="server",
                version="0.0.127",
                kind="tag",
                source_sha="2" * 40,
                reference="agent-server/v0.0.127",
                public=True,
            ),
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="candidate",
                candidate_id="bot/release-browseros",
                reference="bot/release-browseros",
            ),
        ]

        record = resolve_standalone_release(_request(), operations)

        self.assertEqual(record.version, "0.0.129")
        self.assertEqual(record.previous_tag, "agent-server/v0.0.127")

    def test_reuses_source_bound_draft(self) -> None:
        operations = FakeOperations()
        operations.records = [
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="release",
                source_sha=SOURCE_SHA,
                reference="agent-server/v0.0.128",
                reusable=True,
            )
        ]

        record = resolve_standalone_release(_request(), operations)

        self.assertEqual(record.version, "0.0.128")
        self.assertEqual(record.reservation, "reuse")

    def test_probes_only_resolved_versions_until_one_is_unoccupied(self) -> None:
        operations = FakeOperations()
        operations.records = [
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="release",
                source_sha=SOURCE_SHA,
                reference="agent-server/v0.0.128",
                reusable=True,
            )
        ]
        operations.resource_records = {
            "0.0.128": AllocationRecord(
                component="server",
                version="0.0.128",
                kind="resource",
                reference="r2://browseros/artifacts/server/0.0.128",
            ),
            "0.0.129": AllocationRecord(
                component="server",
                version="0.0.129",
                kind="resource",
                reference="r2://browseros/artifacts/server/0.0.129",
            ),
        }

        record = resolve_standalone_release(_request(), operations)

        self.assertEqual(record.version, "0.0.130")
        self.assertEqual(
            operations.resource_probes,
            [
                ("server", "0.0.128", SOURCE_SHA),
                ("server", "0.0.129", SOURCE_SHA),
                ("server", "0.0.130", SOURCE_SHA),
            ],
        )

    def test_matching_resource_binding_preserves_source_bound_retry(self) -> None:
        operations = FakeOperations()
        operations.records = [
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="release",
                source_sha=SOURCE_SHA,
                reference="agent-server/v0.0.128",
                reusable=True,
            )
        ]
        operations.resource_records = {
            "0.0.128": AllocationRecord(
                component="server",
                version="0.0.128",
                kind="resource",
                source_sha=SOURCE_SHA,
                reference="agent-server/v0.0.128",
                reusable=True,
            )
        }

        record = resolve_standalone_release(_request(), operations)

        self.assertEqual(record.version, "0.0.128")
        self.assertEqual(record.reservation, "reuse")
        self.assertEqual(
            operations.resource_probes,
            [("server", "0.0.128", SOURCE_SHA)],
        )

    def test_explicit_version_rejects_conflicting_resource_binding(self) -> None:
        operations = FakeOperations()
        operations.resource_records = {
            "0.0.129": AllocationRecord(
                component="server",
                version="0.0.129",
                kind="resource",
                reference="r2://browseros/artifacts/server/0.0.129",
            )
        }

        with self.assertRaisesRegex(ValueError, "already allocated"):
            resolve_standalone_release(
                _request(requested_version="0.0.129"), operations
            )

        self.assertEqual(
            operations.resource_probes,
            [("server", "0.0.129", SOURCE_SHA)],
        )

    def test_tag_push_requires_annotated_source_bound_tag(self) -> None:
        operations = FakeOperations()
        tag = "agent-server/v0.0.127"
        operations.tags[tag] = TagState(SOURCE_SHA, True)
        operations.records = [
            AllocationRecord(
                component="server",
                version="0.0.127",
                kind="tag",
                source_sha=SOURCE_SHA,
                reference=tag,
                reusable=True,
                public=True,
            )
        ]

        record = resolve_standalone_release(
            _request(event_name="push", ref_name=tag, release_ref=""), operations
        )

        self.assertEqual(record.reservation, "tag")
        operations.tags[tag] = TagState(SOURCE_SHA, False)
        with self.assertRaisesRegex(ValueError, "annotated"):
            resolve_standalone_release(
                _request(event_name="push", ref_name=tag, release_ref=""),
                operations,
            )

    def test_rejects_non_default_branch_source_and_explicit_collision(self) -> None:
        operations = FakeOperations()
        operations.ancestor = False
        with self.assertRaisesRegex(ValueError, "not reachable"):
            resolve_standalone_release(_request(), operations)

        operations.ancestor = True
        operations.records = [
            AllocationRecord(
                component="server",
                version="0.0.129",
                kind="candidate",
                source_sha="2" * 40,
                candidate_id="other",
                reference="other",
            )
        ]
        with self.assertRaisesRegex(ValueError, "already allocated"):
            resolve_standalone_release(
                _request(requested_version="0.0.129"), operations
            )

    def test_additional_feed_allocation_blocks_extension_version(self) -> None:
        operations = FakeOperations()
        operations.version = "0.0.127.0"
        feed = AllocationRecord(
            component="agent",
            version="0.0.127.0",
            kind="release",
            reference="appcast",
            public=True,
        )

        record = resolve_standalone_release(
            _request(component="agent"), operations, (feed,)
        )

        self.assertEqual(record.version, "0.0.128.0")


class ComponentAllocationDiscoveryTest(unittest.TestCase):
    def test_read_version_decodes_semver_safe_chrome_identity(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            operations = GitComponentReleaseOperations(
                Path(tmp), "browseros-ai/BrowserOS"
            )
            with mock.patch.object(
                operations, "_git", return_value='{"version":"0.0.126+7"}'
            ):
                version = operations.read_version("agent", "origin/main")

        self.assertEqual(version, "0.0.126.7")

    def test_r2_retry_uses_requested_source_with_older_release_history(self) -> None:
        key = "artifacts/server/0.0.130/browseros-server-resources-linux-x64.zip"
        client = mock.MagicMock()
        client.list_objects_v2.return_value = {
            "Contents": [{"Key": key}],
            "IsTruncated": False,
        }
        client.head_object.return_value = {
            "Metadata": {
                "component": "artifacts/server",
                "release-sha": SOURCE_SHA,
                "sha256": "a" * 64,
                "target": "linux-x64",
                "version": "0.0.130",
            }
        }
        releases = [
            {
                "tagName": "agent-server/v0.0.130",
                "isDraft": True,
                "targetCommitish": SOURCE_SHA,
            },
            {
                "tagName": "agent-server/v0.0.129",
                "isDraft": False,
                "targetCommitish": "2" * 40,
            },
        ]
        with tempfile.TemporaryDirectory() as tmp:
            operations = GitComponentReleaseOperations(
                Path(tmp),
                "browseros-ai/BrowserOS",
                r2_client=client,
                r2_bucket="browseros",
            )
            with (
                mock.patch.object(operations, "_git", return_value=""),
                mock.patch(
                    "bos_build.release.component_release.subprocess.run",
                    return_value=subprocess.CompletedProcess(
                        args=[], returncode=1, stdout="", stderr=""
                    ),
                ),
                mock.patch(
                    "bos_build.release.component_release.list_pull_requests",
                    return_value=[],
                ),
                mock.patch(
                    "bos_build.release.component_release.list_github_releases",
                    return_value=releases,
                ),
            ):
                operations.allocations("server")
                resource = operations.resource_allocation(
                    "server", "0.0.130", SOURCE_SHA
                )

        self.assertIsNotNone(resource)
        assert resource is not None
        self.assertTrue(resource.reusable)
        self.assertEqual(resource.source_sha, SOURCE_SHA)
        client.head_object.assert_called_once_with(Bucket="browseros", Key=key)

    def test_open_browser_candidate_is_discovered_as_a_reservation(self) -> None:
        candidate = CandidateRecord(
            product="browseros",
            parent_sha="2" * 40,
            candidate_sha="3" * 40,
            default_branch="main",
            branch=f"bot/release-browseros-{'2' * 12}",
            browser_version="0.31.0",
            component_versions={
                "server": "0.0.128",
                "agent": "0.0.101.0",
                "app-onboard": "0.0.12",
            },
            pull_request_number=42,
            pull_request_url="https://github.com/browseros-ai/BrowserOS/pull/42",
        )
        body = (
            f"<!-- browseros-release-candidate-v1\n{candidate.to_json().strip()}\n-->"
        )
        with tempfile.TemporaryDirectory() as tmp:
            operations = GitComponentReleaseOperations(
                Path(tmp), "browseros-ai/BrowserOS"
            )
            with (
                mock.patch.object(operations, "_git", return_value=""),
                mock.patch(
                    "bos_build.release.component_release.subprocess.run",
                    return_value=subprocess.CompletedProcess(
                        args=[], returncode=0, stdout="[]", stderr=""
                    ),
                ),
                mock.patch(
                    "bos_build.release.component_release.list_pull_requests",
                    return_value=[
                        {
                            "body": body,
                            "baseRefName": "main",
                            "headRefName": candidate.branch,
                            "headRefOid": candidate.candidate_sha,
                            "headRepository": {
                                "nameWithOwner": "browseros-ai/BrowserOS"
                            },
                            "isCrossRepository": False,
                            "state": "OPEN",
                            "mergedAt": None,
                        }
                    ],
                ),
                mock.patch(
                    "bos_build.release.component_release.list_github_releases",
                    return_value=[],
                ),
                mock.patch(
                    "bos_build.release.component_release.GitHubCandidateBackend.validate_candidate"
                ) as validate_candidate,
            ):
                allocations = operations.allocations("server")

        self.assertEqual(len(allocations), 1)
        self.assertEqual(allocations[0].kind, "candidate")
        self.assertEqual(allocations[0].version, "0.0.128")
        self.assertFalse(allocations[0].public)
        validate_candidate.assert_called_once_with(candidate)

    def test_open_family_suite_is_discovered_as_a_reservation(self) -> None:
        suite = suite_record(state_sha=STATE_SHA)
        pull_request = {
            "body": suite.pull_request_body(),
            "baseRefName": suite.default_branch,
            "headRefName": suite.branch,
            "headRefOid": suite.state_sha,
            "headRepository": {"nameWithOwner": "browseros-ai/BrowserOS"},
            "isCrossRepository": False,
            "number": suite.pull_request_number,
            "url": suite.pull_request_url,
            "state": "OPEN",
            "mergedAt": None,
            "mergeCommit": None,
        }
        with tempfile.TemporaryDirectory() as tmp:
            operations = GitComponentReleaseOperations(
                Path(tmp), "browseros-ai/BrowserOS"
            )
            operations.default_branch = "main"
            with (
                mock.patch.object(operations, "_git", return_value=""),
                mock.patch(
                    "bos_build.release.component_release.subprocess.run",
                    return_value=subprocess.CompletedProcess(
                        args=[], returncode=0, stdout="[]", stderr=""
                    ),
                ),
                mock.patch(
                    "bos_build.release.component_release.list_pull_requests",
                    return_value=[pull_request],
                ),
                mock.patch(
                    "bos_build.release.component_release.list_github_releases",
                    return_value=[],
                ),
                mock.patch(
                    "bos_build.release.suite.GitHubSuiteBackend.discover_branch_reservations",
                    return_value=(
                        replace(suite, pull_request_number=0, pull_request_url=""),
                    ),
                ),
            ):
                allocations = operations.allocations("server")

        self.assertEqual(len(allocations), 1)
        self.assertEqual(allocations[0].kind, "candidate")
        self.assertEqual(allocations[0].version, suite.component_versions["server"])
        self.assertEqual(allocations[0].candidate_id, suite.branch)
        self.assertEqual(allocations[0].source_sha, suite.source_sha)
        self.assertEqual(allocations[0].reference, "agent-server/v0.0.147")
        self.assertTrue(allocations[0].reusable)

        release_operations = FakeOperations()
        release_operations.records = allocations
        release = resolve_standalone_release(
            StandaloneReleaseRequest(
                component="server",
                event_name="workflow_call",
                default_branch="main",
                requested_version=suite.component_versions["server"],
                release_ref=suite.source_sha,
            ),
            release_operations,
        )
        self.assertEqual(release.version, suite.component_versions["server"])

    def test_closed_family_suite_without_branch_blocks_standalone_reuse(self) -> None:
        suite = suite_record(state="closed", state_sha=STATE_SHA)
        pull_request = {
            "body": suite.pull_request_body(),
            "baseRefName": suite.default_branch,
            "headRefName": suite.branch,
            "headRefOid": suite.state_sha,
            "headRepository": {"nameWithOwner": "browseros-ai/BrowserOS"},
            "isCrossRepository": False,
            "number": suite.pull_request_number,
            "url": suite.pull_request_url,
            "state": "CLOSED",
            "mergedAt": None,
            "mergeCommit": None,
        }
        with tempfile.TemporaryDirectory() as tmp:
            operations = GitComponentReleaseOperations(
                Path(tmp), "browseros-ai/BrowserOS"
            )
            operations.default_branch = "main"
            with (
                mock.patch.object(operations, "_git", return_value=""),
                mock.patch(
                    "bos_build.release.component_release.subprocess.run",
                    return_value=subprocess.CompletedProcess(
                        args=[], returncode=0, stdout="[]", stderr=""
                    ),
                ),
                mock.patch(
                    "bos_build.release.component_release.list_pull_requests",
                    return_value=[pull_request],
                ) as list_prs,
                mock.patch(
                    "bos_build.release.component_release.list_github_releases",
                    return_value=[],
                ),
                mock.patch(
                    "bos_build.release.suite.GitHubSuiteBackend.discover_branch_reservations",
                    return_value=(),
                ),
            ):
                allocations = operations.allocations("server")

        list_prs.assert_called_once_with("browseros-ai/BrowserOS", state="all")
        self.assertEqual(len(allocations), 1)
        self.assertFalse(allocations[0].reusable)
        self.assertTrue(allocations[0].reuse_forbidden)
        release_operations = FakeOperations()
        release_operations.records = allocations
        with self.assertRaisesRegex(ValueError, "already allocated"):
            resolve_standalone_release(
                StandaloneReleaseRequest(
                    component="server",
                    event_name="workflow_call",
                    default_branch="main",
                    requested_version=suite.component_versions["server"],
                    release_ref=suite.source_sha,
                ),
                release_operations,
            )

    def test_closed_suite_vetoes_reuse_of_its_existing_component_draft(self) -> None:
        suite = suite_record(state="closed", state_sha=STATE_SHA)
        release_operations = FakeOperations()
        release_operations.records = (
            AllocationRecord(
                component="server",
                version=suite.component_versions["server"],
                kind="release",
                source_sha=suite.source_sha,
                reference="agent-server/v0.0.147",
                reusable=True,
            ),
            AllocationRecord(
                component="server",
                version=suite.component_versions["server"],
                kind="candidate",
                source_sha=suite.source_sha,
                reference="agent-server/v0.0.147",
                reuse_forbidden=True,
            ),
        )

        with self.assertRaisesRegex(ValueError, "already allocated"):
            resolve_standalone_release(
                StandaloneReleaseRequest(
                    component="server",
                    event_name="workflow_call",
                    default_branch="main",
                    requested_version=suite.component_versions["server"],
                    release_ref=suite.source_sha,
                ),
                release_operations,
            )


if __name__ == "__main__":
    unittest.main()
