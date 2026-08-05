#!/usr/bin/env python3
"""Immutable browser candidate lifecycle tests."""

import tempfile
import unittest
from pathlib import Path

from bos_build.release.candidate import (
    CandidateRecord,
    CandidateRequest,
    GitHubCandidateBackend,
    PullRequestState,
    ensure_candidate,
    merge_candidate,
)
from bos_build.release.components import AllocationRecord


PARENT_SHA = "1" * 40
CANDIDATE_SHA = "2" * 40


def candidate_record(state: str = "open") -> CandidateRecord:
    return CandidateRecord(
        product="browseros",
        parent_sha=PARENT_SHA,
        candidate_sha=CANDIDATE_SHA,
        default_branch="main",
        branch=f"bot/release-browseros-{PARENT_SHA[:12]}",
        browser_version="0.31.0",
        component_versions={
            "server": "0.0.128",
            "agent": "0.0.101.0",
            "claw-onboard": "0.0.12",
        },
        pull_request_number=42,
        pull_request_url="https://github.com/browseros-ai/BrowserOS/pull/42",
        state=state,
    )


class FakeBackend:
    def __init__(self) -> None:
        self.head = PARENT_SHA
        self.clean = True
        self.existing = None
        self.allocations = ()
        self.created = []
        self.pr = PullRequestState(
            number=42,
            url="https://github.com/browseros-ai/BrowserOS/pull/42",
            state="open",
            head_sha=CANDIDATE_SHA,
            head_branch=f"bot/release-browseros-{PARENT_SHA[:12]}",
            base_branch="main",
            mergeable=True,
        )
        self.merged = []

    def current_sha(self) -> str:
        return self.head

    def is_clean(self) -> bool:
        return self.clean

    def find_candidate(self, product: str, parent_sha: str):
        return self.existing

    def discover_allocations(self, product: str):
        return self.allocations

    def read_committed_versions(self, product: str):
        return {
            "server": "0.0.127",
            "agent": "0.0.100",
            "claw-onboard": "0.0.12",
        }

    def read_browser_version(self) -> str:
        return "0.31.0"

    def create_candidate(self, request, branch, versions, browser_version):
        self.created.append((request, branch, versions, browser_version))
        return candidate_record()

    def inspect_pull_request(self, number: int) -> PullRequestState:
        return self.pr

    def merge_pull_request(self, number: int) -> str:
        self.merged.append(number)
        return "3" * 40

    def default_branch_contains_versions(self, record: CandidateRecord) -> bool:
        return False


class CandidateEnsureTest(unittest.TestCase):
    def setUp(self) -> None:
        self.backend = FakeBackend()
        self.request = CandidateRequest(
            product="browseros",
            parent_sha=PARENT_SHA,
            default_branch="main",
            dispatch_ref="main",
        )

    def test_rejects_non_default_dispatch_wrong_checkout_and_dirty_tree(self) -> None:
        with self.assertRaisesRegex(ValueError, "default branch"):
            ensure_candidate(
                CandidateRequest(
                    product="browseros",
                    parent_sha=PARENT_SHA,
                    default_branch="main",
                    dispatch_ref="feature",
                ),
                self.backend,
            )

        self.backend.head = "9" * 40
        with self.assertRaisesRegex(ValueError, "frozen parent"):
            ensure_candidate(self.request, self.backend)

        self.backend.head = PARENT_SHA
        self.backend.clean = False
        with self.assertRaisesRegex(ValueError, "clean checkout"):
            ensure_candidate(self.request, self.backend)

    def test_creates_one_candidate_with_advanced_component_versions(self) -> None:
        record = ensure_candidate(self.request, self.backend)

        self.assertEqual(record, candidate_record())
        self.assertEqual(len(self.backend.created), 1)
        _, branch, versions, browser_version = self.backend.created[0]
        self.assertEqual(branch, f"bot/release-browseros-{PARENT_SHA[:12]}")
        self.assertEqual(
            versions,
            {
                "server": "0.0.128",
                "agent": "0.0.101.0",
                "claw-onboard": "0.0.12",
            },
        )
        self.assertEqual(browser_version, "0.31.0")

    def test_recovers_existing_candidate_without_allocating_or_mutating(self) -> None:
        self.backend.existing = candidate_record()

        record = ensure_candidate(self.request, self.backend)

        self.assertEqual(record, candidate_record())
        self.assertEqual(self.backend.created, [])

    def test_new_candidate_skips_open_reservations(self) -> None:
        self.backend.allocations = (
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="candidate",
                candidate_id="other",
            ),
            AllocationRecord(
                component="agent",
                version="0.0.101.0",
                kind="candidate",
                candidate_id="other",
            ),
        )

        ensure_candidate(self.request, self.backend)

        self.assertEqual(
            self.backend.created[0][2],
            {
                "server": "0.0.129",
                "agent": "0.0.102.0",
                "claw-onboard": "0.0.12",
            },
        )

    def test_github_backend_reads_semantic_browser_version(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            version_file = root / "packages/browseros/resources/BROWSEROS_VERSION"
            version_file.parent.mkdir(parents=True)
            version_file.write_text(
                "BROWSEROS_MAJOR=0\n"
                "BROWSEROS_MINOR=49\n"
                "BROWSEROS_BUILD=2\n"
                "BROWSEROS_PATCH=0\n"
            )
            backend = GitHubCandidateBackend(root, "owner/repo", "main")

            self.assertEqual(backend.read_browser_version(), "0.49.2")


class CandidateMergeTest(unittest.TestCase):
    def setUp(self) -> None:
        self.backend = FakeBackend()
        self.record = candidate_record()
        self.gate = {
            "schema": "browseros-release-gate-v1",
            "passed": True,
            "product": self.record.product,
            "parent_sha": self.record.parent_sha,
            "candidate_sha": self.record.candidate_sha,
            "browser_version": self.record.browser_version,
            "component_versions": dict(self.record.component_versions),
            "common_manifest_digest": "4" * 64,
            "lanes": ["linux-x64", "macos-universal", "windows-x64"],
            "outcomes": [
                "linux-x64",
                "macos-arm64",
                "macos-universal",
                "macos-x64",
                "windows-x64",
            ],
            "server_checksums": {
                "darwin-arm64": "5" * 64,
                "darwin-x64": "6" * 64,
                "linux-x64": "7" * 64,
                "windows-x64": "8" * 64,
            },
            "artifact_checksums": {"BrowserOS.dmg": "9" * 64},
        }

    def test_rejects_missing_gate_and_changed_pull_request(self) -> None:
        with self.assertRaisesRegex(ValueError, "gate"):
            merge_candidate(self.record, {}, self.backend)

        self.backend.pr = PullRequestState(
            number=42,
            url=self.record.pull_request_url,
            state="open",
            head_sha="9" * 40,
            head_branch=self.record.branch,
            base_branch="main",
            mergeable=True,
        )
        with self.assertRaisesRegex(ValueError, "head"):
            merge_candidate(self.record, self.gate, self.backend)

    def test_rejects_gate_identity_skew(self) -> None:
        for field, value in (
            ("product", "browserclaw"),
            ("parent_sha", "9" * 40),
            ("browser_version", "0.32.0"),
            ("component_versions", {"server": "0.0.999"}),
        ):
            with self.subTest(field=field), self.assertRaisesRegex(
                ValueError, field
            ):
                merge_candidate(
                    self.record, {**self.gate, field: value}, self.backend
                )

    def test_rejects_unmergeable_or_superseded_candidate(self) -> None:
        self.backend.pr = PullRequestState(
            number=42,
            url=self.record.pull_request_url,
            state="open",
            head_sha=CANDIDATE_SHA,
            head_branch=self.record.branch,
            base_branch="main",
            mergeable=False,
        )
        with self.assertRaisesRegex(ValueError, "mergeable"):
            merge_candidate(self.record, self.gate, self.backend)

        self.backend.pr = FakeBackend().pr
        self.backend.default_branch_contains_versions = lambda record: True
        with self.assertRaisesRegex(ValueError, "superseded"):
            merge_candidate(self.record, self.gate, self.backend)

    def test_merges_unchanged_candidate_and_preserves_candidate_sha(self) -> None:
        merged = merge_candidate(self.record, self.gate, self.backend)

        self.assertEqual(merged.state, "merged")
        self.assertEqual(merged.candidate_sha, CANDIDATE_SHA)
        self.assertEqual(merged.merge_sha, "3" * 40)
        self.assertEqual(self.backend.merged, [42])


if __name__ == "__main__":
    unittest.main()
