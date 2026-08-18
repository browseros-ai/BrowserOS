#!/usr/bin/env python3
"""Immutable browser candidate lifecycle tests."""

import subprocess
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from bos_build.release.candidate import (
    CandidateRecord,
    CandidateRequest,
    GitHubCandidateBackend,
    PullRequestState,
    candidate_record_from_pull_request,
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
        self.merge_commit_matches = True

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

    def merge_pull_request(self, number: int, expected_head_sha: str) -> str:
        self.merged.append((number, expected_head_sha))
        return "3" * 40

    def default_branch_contains_versions(self, record: CandidateRecord) -> bool:
        return False

    def merge_commit_matches_candidate(
        self, record: CandidateRecord, merge_sha: str
    ) -> bool:
        return self.merge_commit_matches


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

    def test_accepts_only_same_repository_canonical_candidate_pull_requests(self) -> None:
        record = candidate_record()
        body = (
            "<!-- browseros-release-candidate-v1\n"
            f"{record.to_json().strip()}\n"
            "-->"
        )
        pull_request = {
            "body": body,
            "baseRefName": "main",
            "headRefName": record.branch,
            "headRefOid": record.candidate_sha,
            "headRepository": {"nameWithOwner": "browseros-ai/BrowserOS"},
            "isCrossRepository": False,
        }

        self.assertEqual(
            candidate_record_from_pull_request(
                pull_request, "browseros-ai/BrowserOS"
            ),
            record,
        )
        self.assertIsNone(
            candidate_record_from_pull_request(
                {
                    **pull_request,
                    "headRepository": {"nameWithOwner": "attacker/BrowserOS"},
                    "isCrossRepository": True,
                },
                "browseros-ai/BrowserOS",
            )
        )
        self.assertIsNone(
            candidate_record_from_pull_request(
                {**pull_request, "headRefName": "attacker-branch"},
                "browseros-ai/BrowserOS",
            )
        )


class CandidateBranchRecoveryTest(unittest.TestCase):
    def _git(self, root: Path, *args: str) -> str:
        result = subprocess.run(
            ["git", *args],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout.strip()

    def _repo(self, root: Path) -> tuple[Path, str]:
        remote = root / "remote.git"
        repo = root / "repo"
        self._git(root, "init", "--bare", str(remote))
        self._git(root, "init", str(repo))
        self._git(repo, "config", "user.name", "BrowserOS CI")
        self._git(repo, "config", "user.email", "ci@browseros.com")
        self._git(repo, "remote", "add", "origin", str(remote))
        (repo / "version.txt").write_text("parent\n", encoding="utf-8")
        self._git(repo, "add", "version.txt")
        self._git(repo, "commit", "-m", "parent")
        return repo, self._git(repo, "rev-parse", "HEAD")

    def _commit(self, repo: Path, parent: str, value: str, message: str) -> str:
        self._git(repo, "reset", "--hard", parent)
        (repo / "version.txt").write_text(value, encoding="utf-8")
        self._git(repo, "add", "version.txt")
        self._git(repo, "commit", "-m", message)
        return self._git(repo, "rev-parse", "HEAD")

    def test_recovers_matching_remote_branch_after_interrupted_creation(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo, parent = self._repo(Path(temp_dir))
            remote_candidate = self._commit(repo, parent, "candidate\n", "remote")
            branch = "bot/release-browseros-retry"
            self._git(repo, "push", "origin", f"HEAD:refs/heads/{branch}")
            local_candidate = self._commit(repo, parent, "candidate\n", "local")
            backend = GitHubCandidateBackend(repo, "owner/repo", "main")

            recovered = backend._publish_candidate_commit(
                branch,
                local_candidate,
                parent,
                cwd=repo,
            )

            self.assertNotEqual(local_candidate, remote_candidate)
            self.assertEqual(recovered, remote_candidate)

    def test_rejects_remote_branch_with_different_candidate_content(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo, parent = self._repo(Path(temp_dir))
            self._commit(repo, parent, "unexpected\n", "remote")
            branch = "bot/release-browseros-conflict"
            self._git(repo, "push", "origin", f"HEAD:refs/heads/{branch}")
            local_candidate = self._commit(repo, parent, "candidate\n", "local")
            backend = GitHubCandidateBackend(repo, "owner/repo", "main")

            with self.assertRaisesRegex(ValueError, "unexpected candidate content"):
                backend._publish_candidate_commit(
                    branch,
                    local_candidate,
                    parent,
                    cwd=repo,
                )


class CandidateBackendVersionGuardTest(unittest.TestCase):
    def setUp(self) -> None:
        self.backend = GitHubCandidateBackend(Path("/repo"), "owner/repo", "main")
        self.record = candidate_record()

    def test_independent_onboarding_release_does_not_supersede_candidate(self) -> None:
        observed = []

        def version_at_ref(component: str, ref: str) -> str:
            observed.append(component)
            if component == "claw-onboard" and ref == "origin/main":
                return "0.0.13"
            return self.record.component_versions[component]

        with (
            patch.object(self.backend, "_git"),
            patch.object(
                self.backend,
                "_version_at_ref",
                side_effect=version_at_ref,
            ),
        ):
            self.assertFalse(self.backend.default_branch_contains_versions(self.record))

        self.assertNotIn("claw-onboard", observed)

    def test_merged_candidate_retry_ignores_independent_onboarding_release(
        self,
    ) -> None:
        observed = []

        def version_at_ref(component: str, ref: str) -> str:
            observed.append(component)
            if component == "claw-onboard":
                return "0.0.13"
            return self.record.component_versions[component]

        with (
            patch.object(self.backend, "_git"),
            patch.object(
                self.backend,
                "_version_at_ref",
                side_effect=version_at_ref,
            ),
            patch(
                "bos_build.release.candidate.subprocess.run",
                return_value=subprocess.CompletedProcess([], 0),
            ),
        ):
            self.assertTrue(
                self.backend.merge_commit_matches_candidate(
                    self.record,
                    "3" * 40,
                )
            )

        self.assertNotIn("claw-onboard", observed)

    def test_merged_candidate_retry_decodes_chrome_package_version(self) -> None:
        for package_version, release_version in (
            ("0.0.101", "0.0.101.0"),
            ("0.0.101+7", "0.0.101.7"),
        ):
            with self.subTest(package_version=package_version):
                record = replace(
                    self.record,
                    component_versions={
                        **self.record.component_versions,
                        "agent": release_version,
                    },
                )

                def version_at_ref(component: str, ref: str) -> str:
                    if component == "agent":
                        return package_version
                    return record.component_versions[component]

                with (
                    patch.object(self.backend, "_git"),
                    patch.object(
                        self.backend,
                        "_version_at_ref",
                        side_effect=version_at_ref,
                    ),
                    patch(
                        "bos_build.release.candidate.subprocess.run",
                        return_value=subprocess.CompletedProcess([], 0),
                    ),
                ):
                    self.assertTrue(
                        self.backend.merge_commit_matches_candidate(
                            record,
                            "3" * 40,
                        )
                    )


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
            "artifacts": {
                "BrowserOS.dmg": {
                    "filename": "BrowserOS.dmg",
                    "size": 1,
                    "sha256": "9" * 64,
                    "url": "https://cdn.browseros.com/BrowserOS.dmg",
                    "sparkle_signature": "signature",
                }
            },
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
        self.assertEqual(self.backend.merged, [(42, CANDIDATE_SHA)])

    def test_recovers_only_an_exact_already_merged_candidate(self) -> None:
        self.backend.pr = PullRequestState(
            number=42,
            url=self.record.pull_request_url,
            state="merged",
            head_sha=CANDIDATE_SHA,
            head_branch=self.record.branch,
            base_branch="main",
            mergeable=False,
            merge_sha="3" * 40,
        )

        merged = merge_candidate(self.record, self.gate, self.backend)

        self.assertEqual(merged.state, "merged")
        self.assertEqual(merged.merge_sha, "3" * 40)
        self.backend.pr = replace(self.backend.pr, head_sha="9" * 40)
        with self.assertRaisesRegex(ValueError, "head"):
            merge_candidate(self.record, self.gate, self.backend)
        self.backend.pr = replace(self.backend.pr, head_sha=CANDIDATE_SHA)
        self.backend.merge_commit_matches = False
        with self.assertRaisesRegex(ValueError, "commit"):
            merge_candidate(self.record, self.gate, self.backend)


if __name__ == "__main__":
    unittest.main()
