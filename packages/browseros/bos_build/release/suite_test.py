#!/usr/bin/env python3
"""Family release transaction lifecycle tests."""

import hashlib
import shutil
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest import mock

from bos_build.release.components import AllocationRecord, increment_component_version
from bos_build.release.suite import (
    BrowserAllocation,
    GitHubSuiteBackend,
    SUITE_COMPONENTS,
    SUITE_ONBOARDING_COMPONENTS,
    SUITE_RELEASE_COMPONENTS,
    SUITE_STATE_PATHS,
    SuitePullRequest,
    SuiteRecord,
    SuiteRequest,
    inspect_transaction,
    merge_suite_component_allocations,
    merge_transaction,
    reconcile_transaction,
    suite_record_from_pull_request,
    transaction_branch,
    transaction_id,
)


SOURCE_SHA = "1" * 40
RESERVATION_SHA = "2" * 40
STATE_SHA = "3" * 40
MERGE_SHA = "4" * 40
BRANCH = f"bot/release-nightly-{SOURCE_SHA[:12]}"


def suite_record(
    *, state: str = "open", state_sha: str = RESERVATION_SHA
) -> SuiteRecord:
    return SuiteRecord(
        transaction_id=f"nightly-{SOURCE_SHA}",
        mode="nightly",
        source_sha=SOURCE_SHA,
        reservation_sha=RESERVATION_SHA,
        state_sha=state_sha,
        default_branch="main",
        branch=BRANCH,
        browser_version="0.50.1",
        build_offset=401,
        component_versions={
            "server": "0.0.147",
            "agent": "0.0.121.0",
            "claw-server-rust": "0.0.46",
            "browserclaw": "0.0.83.0",
            "app-onboard": "0.0.0",
            "claw-onboard": "0.0.15",
        },
        pull_request_number=77,
        pull_request_url="https://github.com/browseros-ai/BrowserOS/pull/77",
        state=state,
        merge_sha=MERGE_SHA if state == "merged" else "",
        state_checksums={
            "updates/extensions/bundled-manifest.xml": "a" * 64,
            "updates/extensions/extensions.alpha.json": "b" * 64,
            "updates/extensions/update-manifest.alpha.xml": "c" * 64,
            "updates/server/appcast-claw-server.alpha.xml": "d" * 64,
            "updates/server/appcast-server.alpha.xml": "e" * 64,
        },
    )


class FakeBackend:
    def __init__(self) -> None:
        self.head = SOURCE_SHA
        self.changed = ()
        self.existing: SuiteRecord | None = None
        self.allocations: tuple[AllocationRecord, ...] = ()
        self.browser_allocations: tuple[BrowserAllocation, ...] = ()
        self.created = []
        self.reconciled = []
        self.merged = []
        self.readied = []
        self.superseded = False
        self.merge_matches = True
        self.pr = SuitePullRequest(
            number=77,
            url="https://github.com/browseros-ai/BrowserOS/pull/77",
            state="open",
            head_sha=RESERVATION_SHA,
            head_branch=BRANCH,
            base_branch="main",
            mergeable=True,
            draft=True,
        )

    def current_sha(self) -> str:
        return self.head

    def changed_paths(self):
        return self.changed

    def find_transaction(self, request: SuiteRequest):
        return self.existing

    def discover_allocations(self):
        return self.allocations

    def discover_browser_allocations(self):
        return self.browser_allocations

    def read_committed_versions(self):
        return {
            "server": "0.0.146",
            "agent": "0.0.120.0",
            "claw-server-rust": "0.0.45",
            "browserclaw": "0.0.82.0",
            "app-onboard": "0.0.0",
            "claw-onboard": "0.0.15",
        }

    def read_browser_version(self):
        return "0.50.0"

    def read_build_offset(self):
        return 400

    def create_transaction(
        self, request, branch, component_versions, browser_version, build_offset
    ):
        self.created.append(
            (request, branch, component_versions, browser_version, build_offset)
        )
        return suite_record()

    def reconcile_state(self, record, state_root):
        self.reconciled.append((record, state_root))
        return replace(record, state_sha=STATE_SHA)

    def inspect_pull_request(self, number):
        return self.pr

    def default_branch_contains_transaction(self, record):
        return self.superseded

    def merge_pull_request(self, number, expected_head_sha):
        self.merged.append((number, expected_head_sha))
        return MERGE_SHA

    def mark_pull_request_ready(self, number):
        self.readied.append(number)

    def merge_commit_matches_transaction(self, record, merge_sha):
        return self.merge_matches


class SuiteIdentityTest(unittest.TestCase):
    def test_summary_prints_browser_version_once(self) -> None:
        self.assertEqual(suite_record().summary().count("- Browser version:"), 1)

    def test_identity_uses_mode_and_source_but_not_run_attempt(self) -> None:
        self.assertEqual(transaction_id("nightly", SOURCE_SHA), f"nightly-{SOURCE_SHA}")
        self.assertEqual(transaction_branch("nightly", SOURCE_SHA), BRANCH)
        self.assertNotIn("attempt", transaction_id("full", SOURCE_SHA))

        for mode in ("other", ""):
            with self.subTest(mode=mode), self.assertRaisesRegex(ValueError, "mode"):
                transaction_id(mode, SOURCE_SHA)
        with self.assertRaisesRegex(ValueError, "SHA"):
            transaction_id("nightly", "short")

    def test_pull_request_parser_accepts_only_canonical_same_repo_record(self) -> None:
        record = suite_record()
        # PR creation writes this body before GitHub has assigned its number and
        # URL. Recovery must hydrate both from the same live PR object and must
        # not depend on a second body-edit request completing.
        provisional = replace(record, pull_request_number=0, pull_request_url="")
        pull_request = {
            "body": provisional.pull_request_body(),
            "baseRefName": "main",
            "headRefName": BRANCH,
            "headRefOid": STATE_SHA,
            "headRepository": {"nameWithOwner": "browseros-ai/BrowserOS"},
            "isCrossRepository": False,
            "number": 77,
            "url": record.pull_request_url,
            "state": "OPEN",
            "mergedAt": None,
            "mergeCommit": None,
        }

        parsed = suite_record_from_pull_request(pull_request, "browseros-ai/BrowserOS")

        self.assertIsNotNone(parsed)
        assert parsed is not None
        self.assertEqual(parsed.pull_request_number, 77)
        self.assertEqual(parsed.pull_request_url, record.pull_request_url)
        self.assertEqual(parsed.state_sha, STATE_SHA)
        self.assertEqual(parsed.reservation_sha, RESERVATION_SHA)
        self.assertIsNone(
            suite_record_from_pull_request(
                {
                    **pull_request,
                    "isCrossRepository": True,
                    "headRepository": {"nameWithOwner": "fork/BrowserOS"},
                },
                "browseros-ai/BrowserOS",
            )
        )
        self.assertIsNone(
            suite_record_from_pull_request(
                {**pull_request, "headRefName": "bot/attacker"},
                "browseros-ai/BrowserOS",
            )
        )

    def test_closed_suite_pr_keeps_browser_version_reserved(self) -> None:
        record = suite_record()
        closed_pull_request = {
            "body": record.pull_request_body(),
            "baseRefName": record.default_branch,
            "headRefName": record.branch,
            "headRefOid": record.state_sha,
            "headRepository": {"nameWithOwner": "browseros-ai/BrowserOS"},
            "isCrossRepository": False,
            "number": record.pull_request_number,
            "url": record.pull_request_url,
            "state": "CLOSED",
            "mergedAt": None,
            "mergeCommit": None,
        }
        with tempfile.TemporaryDirectory() as tmp:
            backend = GitHubSuiteBackend(Path(tmp), "browseros-ai/BrowserOS", "main")
            with (
                mock.patch.object(
                    backend, "discover_branch_reservations", return_value=()
                ),
                mock.patch(
                    "bos_build.release.suite.list_pull_requests",
                    return_value=[closed_pull_request],
                ) as list_prs,
            ):
                allocations = backend.discover_browser_allocations()

        list_prs.assert_called_once_with("browseros-ai/BrowserOS", state="all")
        self.assertEqual(
            allocations,
            (
                BrowserAllocation(
                    transaction_id=record.transaction_id,
                    browser_version=record.browser_version,
                    build_offset=record.build_offset,
                ),
            ),
        )

    def test_malformed_closed_suite_pr_fails_instead_of_releasing_version(self) -> None:
        record = suite_record(state="closed")
        malformed = {
            "body": "closed suite marker was corrupted",
            "baseRefName": record.default_branch,
            "headRefName": record.branch,
            "headRefOid": record.state_sha,
            "headRepository": {"nameWithOwner": "browseros-ai/BrowserOS"},
            "isCrossRepository": False,
            "number": record.pull_request_number,
            "url": record.pull_request_url,
            "state": "CLOSED",
            "mergedAt": None,
            "mergeCommit": None,
        }
        with tempfile.TemporaryDirectory() as tmp:
            backend = GitHubSuiteBackend(Path(tmp), "browseros-ai/BrowserOS", "main")
            with (
                mock.patch.object(
                    backend, "discover_branch_reservations", return_value=()
                ),
                mock.patch(
                    "bos_build.release.suite.list_pull_requests",
                    return_value=[malformed],
                ),
                self.assertRaisesRegex(ValueError, "invalid allocation metadata"),
            ):
                backend.discover_browser_allocations()

    def test_branch_and_pr_component_allocations_must_be_identical(self) -> None:
        record = suite_record()
        allocation = AllocationRecord(
            component="server",
            version=record.component_versions["server"],
            kind="candidate",
            source_sha=record.source_sha,
            candidate_id=record.branch,
            reference="agent-server/v" + record.component_versions["server"],
            reusable=True,
        )

        self.assertEqual(
            merge_suite_component_allocations(
                (allocation,),
                (replace(record, pull_request_number=0, pull_request_url=""),),
                ("server",),
            ),
            (allocation,),
        )
        closed_allocation = replace(allocation, reusable=False)
        self.assertEqual(
            merge_suite_component_allocations(
                (closed_allocation,),
                (
                    replace(
                        record,
                        pull_request_number=0,
                        pull_request_url="",
                        state="open",
                    ),
                ),
                ("server",),
            ),
            (closed_allocation,),
        )
        with self.assertRaisesRegex(ValueError, "conflicting component allocations"):
            merge_suite_component_allocations(
                (replace(allocation, source_sha="9" * 40),),
                (replace(record, pull_request_number=0, pull_request_url=""),),
                ("server",),
            )

    def test_open_and_closed_prs_for_one_branch_cannot_mask_reuse_veto(self) -> None:
        draft = AllocationRecord(
            component="server",
            version="0.0.147",
            kind="release",
            source_sha=SOURCE_SHA,
            reference="agent-server/v0.0.147",
            reusable=True,
        )
        open_pr = AllocationRecord(
            component="server",
            version="0.0.147",
            kind="candidate",
            source_sha=SOURCE_SHA,
            candidate_id=suite_record().branch,
            reference="agent-server/v0.0.147",
            reusable=True,
        )
        closed_pr = replace(open_pr, reusable=False, reuse_forbidden=True)

        with self.assertRaisesRegex(ValueError, "Multiple pull requests"):
            merge_suite_component_allocations(
                (draft, open_pr, closed_pr), (), ("server",)
            )


class SuiteReconcileTest(unittest.TestCase):
    def setUp(self) -> None:
        self.backend = FakeBackend()
        self.request = SuiteRequest(
            mode="nightly",
            source_sha=SOURCE_SHA,
            default_branch="main",
            dispatch_ref="main",
        )

    def test_allocates_the_whole_family_once(self) -> None:
        record = reconcile_transaction(self.request, self.backend)

        self.assertEqual(record, suite_record())
        self.assertEqual(len(self.backend.created), 1)
        _, branch, versions, browser_version, build_offset = self.backend.created[0]
        self.assertEqual(branch, BRANCH)
        self.assertEqual(browser_version, "0.50.1")
        self.assertEqual(build_offset, 401)
        self.assertEqual(versions, suite_record().component_versions)

    def test_skips_other_open_reservations_for_every_family_component(self) -> None:
        self.backend.allocations = tuple(
            AllocationRecord(
                component=component,
                version=version,
                kind="candidate",
                candidate_id="other-transaction",
            )
            for component, version in {
                "server": "0.0.147",
                "agent": "0.0.121.0",
                "claw-server-rust": "0.0.46",
                "browserclaw": "0.0.83.0",
            }.items()
        )
        self.backend.browser_allocations = (
            BrowserAllocation(
                transaction_id=f"nightly-{'9' * 40}",
                browser_version="0.50.1",
                build_offset=401,
            ),
        )

        reconcile_transaction(self.request, self.backend)

        created = self.backend.created[0]
        self.assertEqual(created[3:], ("0.50.2", 402))
        self.assertEqual(
            created[2],
            {
                "server": "0.0.148",
                "agent": "0.0.122.0",
                "claw-server-rust": "0.0.47",
                "browserclaw": "0.0.84.0",
                "app-onboard": "0.0.0",
                "claw-onboard": "0.0.15",
            },
        )

    def test_retry_reuses_record_without_allocating(self) -> None:
        self.backend.existing = suite_record(state_sha=STATE_SHA)

        record = reconcile_transaction(self.request, self.backend)

        self.assertEqual(record, self.backend.existing)
        self.assertEqual(self.backend.created, [])

    def test_closed_transaction_is_allocation_only_and_cannot_restart(self) -> None:
        self.backend.existing = suite_record(state="closed")

        with self.assertRaisesRegex(ValueError, "requires an open pull request"):
            reconcile_transaction(self.request, self.backend)

        self.assertEqual(self.backend.created, [])
        self.assertEqual(self.backend.reconciled, [])

    def test_ready_reservation_cannot_restart_before_final_reconcile(self) -> None:
        self.backend.existing = replace(suite_record(), draft=False)

        with self.assertRaisesRegex(ValueError, "complete final state"):
            reconcile_transaction(self.request, self.backend)

        self.assertEqual(self.backend.created, [])
        self.assertEqual(self.backend.reconciled, [])

    def test_ready_recovery_accepts_only_identical_final_state(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            state_root = Path(temp_dir)
            checksums = {}
            for path in SUITE_STATE_PATHS:
                target = state_root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(f"final {path}\n", encoding="utf-8")
                checksums[path] = hashlib.sha256(target.read_bytes()).hexdigest()
            ready = replace(
                suite_record(state_sha=STATE_SHA),
                draft=False,
                state_checksums=checksums,
            )
            self.backend.existing = ready

            self.assertEqual(
                reconcile_transaction(
                    self.request, self.backend, state_root=state_root
                ),
                ready,
            )
            (state_root / SUITE_STATE_PATHS[0]).write_text(
                "different\n", encoding="utf-8"
            )
            with self.assertRaisesRegex(ValueError, "does not match"):
                reconcile_transaction(self.request, self.backend, state_root=state_root)

        self.assertEqual(self.backend.reconciled, [])

    def test_rejects_wrong_ref_checkout_and_unexpected_dirty_files(self) -> None:
        with self.assertRaisesRegex(ValueError, "default branch"):
            reconcile_transaction(
                replace(self.request, dispatch_ref="feature"), self.backend
            )

        self.backend.head = "9" * 40
        with self.assertRaisesRegex(ValueError, "frozen source"):
            reconcile_transaction(self.request, self.backend)

        self.backend.head = SOURCE_SHA
        self.backend.changed = ("README.md",)
        with self.assertRaisesRegex(ValueError, "unexpected changes"):
            reconcile_transaction(self.request, self.backend)

    def test_final_state_allows_only_transaction_snapshots(self) -> None:
        self.backend.existing = suite_record()
        self.backend.changed = (
            "updates/extensions/bundled-manifest.xml",
            "updates/extensions/extensions.alpha.json",
            "updates/extensions/update-manifest.alpha.xml",
            "updates/server/appcast-claw-server.alpha.xml",
            "updates/server/appcast-server.alpha.xml",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            state_root = Path(temp_dir)
            for path in self.backend.changed:
                target = state_root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(path, encoding="utf-8")
            record = reconcile_transaction(
                self.request, self.backend, state_root=state_root
            )

        self.assertEqual(record.state_sha, STATE_SHA)
        self.assertEqual(len(self.backend.reconciled), 1)

    def test_incomplete_final_state_is_rejected_before_backend_write(self) -> None:
        self.backend.existing = suite_record()
        self.backend.changed = ("updates/extensions/bundled-manifest.xml",)

        with (
            tempfile.TemporaryDirectory() as temp_dir,
            self.assertRaisesRegex(ValueError, "complete snapshot set"),
        ):
            state_root = Path(temp_dir)
            path = state_root / self.backend.changed[0]
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("partial", encoding="utf-8")
            reconcile_transaction(self.request, self.backend, state_root=state_root)
        self.assertEqual(self.backend.reconciled, [])


class SuiteInspectAndMergeTest(unittest.TestCase):
    def setUp(self) -> None:
        self.backend = FakeBackend()
        self.request = SuiteRequest("nightly", SOURCE_SHA, "main", "main")

    def test_inspect_is_read_only_and_requires_existing_transaction(self) -> None:
        with self.assertRaisesRegex(ValueError, "not found"):
            inspect_transaction(self.request, self.backend)

        self.backend.existing = suite_record(state_sha=STATE_SHA)
        self.assertEqual(
            inspect_transaction(self.request, self.backend), self.backend.existing
        )
        self.assertEqual(self.backend.created, [])

    def test_merge_requires_matching_passed_gate_and_exact_head(self) -> None:
        record = suite_record(state_sha=STATE_SHA)
        self.backend.pr = replace(self.backend.pr, head_sha=STATE_SHA)
        gate = {
            "schema": "browseros-release-suite-gate-v1",
            "passed": True,
            "transaction_id": record.transaction_id,
            "source_sha": record.source_sha,
            "state_sha": record.state_sha,
            "browser_version": record.browser_version,
            "component_versions": dict(record.component_versions),
            "state_checksums": dict(record.state_checksums),
            "products": ["browseros", "browserclaw"],
        }

        merged = merge_transaction(record, gate, self.backend)

        self.assertEqual(merged.state, "merged")
        self.assertEqual(merged.merge_sha, MERGE_SHA)
        self.assertEqual(self.backend.readied, [77])
        self.assertEqual(self.backend.merged, [(77, STATE_SHA)])

        with self.assertRaisesRegex(ValueError, "state_sha"):
            merge_transaction(record, {**gate, "state_sha": "9" * 40}, self.backend)

    def test_ready_recovery_requires_complete_exact_final_gate(self) -> None:
        record = suite_record(state_sha=STATE_SHA)
        self.backend.pr = replace(self.backend.pr, head_sha=STATE_SHA, draft=False)
        gate = {
            "schema": "browseros-release-suite-gate-v1",
            "passed": True,
            "transaction_id": record.transaction_id,
            "source_sha": record.source_sha,
            "state_sha": record.state_sha,
            "browser_version": record.browser_version,
            "component_versions": dict(record.component_versions),
            "state_checksums": dict(record.state_checksums),
            "products": ["browseros", "browserclaw"],
        }

        merged = merge_transaction(record, gate, self.backend)

        self.assertEqual(merged.merge_sha, MERGE_SHA)
        self.assertEqual(self.backend.readied, [])
        incomplete = replace(record, state_checksums={})
        with self.assertRaisesRegex(ValueError, "complete final state"):
            merge_transaction(
                incomplete,
                {**gate, "state_checksums": {}},
                self.backend,
            )

    def test_new_merge_rejects_mismatched_squash_tree(self) -> None:
        record = suite_record(state_sha=STATE_SHA)
        self.backend.pr = replace(self.backend.pr, head_sha=STATE_SHA)
        self.backend.merge_matches = False
        gate = {
            "schema": "browseros-release-suite-gate-v1",
            "passed": True,
            "transaction_id": record.transaction_id,
            "source_sha": record.source_sha,
            "state_sha": record.state_sha,
            "browser_version": record.browser_version,
            "component_versions": dict(record.component_versions),
            "state_checksums": dict(record.state_checksums),
            "products": ["browseros", "browserclaw"],
        }

        with self.assertRaisesRegex(ValueError, "merge commit"):
            merge_transaction(record, gate, self.backend)

        # The merge already happened, but publication is stopped until recovery
        # proves that the returned squash tree is the complete transaction.
        self.assertEqual(self.backend.readied, [77])
        self.assertEqual(self.backend.merged, [(77, STATE_SHA)])

    def test_merge_rejects_changed_head_and_superseded_state(self) -> None:
        record = suite_record(state_sha=STATE_SHA)
        gate = {
            "schema": "browseros-release-suite-gate-v1",
            "passed": True,
            "transaction_id": record.transaction_id,
            "source_sha": record.source_sha,
            "state_sha": record.state_sha,
            "browser_version": record.browser_version,
            "component_versions": dict(record.component_versions),
            "state_checksums": dict(record.state_checksums),
            "products": ["browseros", "browserclaw"],
        }
        with self.assertRaisesRegex(ValueError, "head"):
            merge_transaction(record, gate, self.backend)

        self.backend.pr = replace(self.backend.pr, head_sha=STATE_SHA)
        self.backend.superseded = True
        with self.assertRaisesRegex(ValueError, "superseded"):
            merge_transaction(record, gate, self.backend)

    def test_merged_retry_requires_exact_merge_content(self) -> None:
        record = suite_record(state="merged", state_sha=STATE_SHA)
        self.backend.pr = replace(
            self.backend.pr,
            state="merged",
            head_sha=STATE_SHA,
            mergeable=False,
            merge_sha=MERGE_SHA,
        )

        self.assertEqual(
            merge_transaction(
                record,
                {
                    "schema": "browseros-release-suite-gate-v1",
                    "passed": True,
                    "transaction_id": record.transaction_id,
                    "source_sha": record.source_sha,
                    "state_sha": record.state_sha,
                    "browser_version": record.browser_version,
                    "component_versions": dict(record.component_versions),
                    "state_checksums": dict(record.state_checksums),
                    "products": ["browseros", "browserclaw"],
                },
                self.backend,
            ).merge_sha,
            MERGE_SHA,
        )
        self.backend.merge_matches = False
        with self.assertRaisesRegex(ValueError, "merge commit"):
            merge_transaction(
                record,
                {
                    "schema": "browseros-release-suite-gate-v1",
                    "passed": True,
                    "transaction_id": record.transaction_id,
                    "source_sha": record.source_sha,
                    "state_sha": record.state_sha,
                    "browser_version": record.browser_version,
                    "component_versions": dict(record.component_versions),
                    "state_checksums": dict(record.state_checksums),
                    "products": ["browseros", "browserclaw"],
                },
                self.backend,
            )


class GitHubSuiteBackendTest(unittest.TestCase):
    def _git(self, root: Path, *args: str) -> str:
        result = __import__("subprocess").run(
            ["git", *args],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout.strip()

    def _repository(self, root: Path) -> tuple[Path, Path, str]:
        remote = root / "remote.git"
        repo = root / "repo"
        self._git(root, "init", "--bare", str(remote))
        self._git(root, "init", "-b", "main", str(repo))
        self._git(repo, "config", "user.name", "Suite Test")
        self._git(repo, "config", "user.email", "suite@example.com")
        self._git(repo, "remote", "add", "origin", str(remote))

        source_root = Path(__file__).resolve().parents[4]
        paths = {
            "packages/browseros/resources/BROWSEROS_VERSION",
            "packages/browseros/bos_build/config/BROWSEROS_BUILD_OFFSET",
            "packages/browseros-agent/apps/server/package.json",
            "packages/browseros-agent/apps/app/package.json",
            "packages/browseros-agent/apps/claw-server-rust/Cargo.toml",
            "packages/browseros-agent/apps/claw-app/package.json",
            "packages/browseros-agent/apps/app-onboard/package.json",
            "packages/browseros-agent/apps/claw-onboard/package.json",
            "packages/browseros-agent/bun.lock",
            "packages/browseros-agent/Cargo.lock",
            *SUITE_STATE_PATHS,
        }
        for relative in paths:
            target = repo / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        self._git(repo, "add", ".")
        self._git(repo, "commit", "-m", "source")
        self._git(repo, "push", "-u", "origin", "main")
        return repo, remote, self._git(repo, "rev-parse", "HEAD")

    def test_branch_push_before_pr_creation_burns_every_reserved_version(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo, remote, source_sha = self._repository(Path(temp_dir))
            backend = GitHubSuiteBackend(repo, "owner/repo", "main")
            committed = backend.read_committed_versions()
            versions = {
                component: (
                    committed[component]
                    if component in SUITE_ONBOARDING_COMPONENTS
                    else increment_component_version(component, committed[component])
                )
                for component in SUITE_COMPONENTS
            }
            major, minor, build = backend.read_browser_version().split(".")[:3]
            browser_version = f"{major}.{minor}.{int(build) + 1}"
            offset = backend.read_build_offset() + 1
            request = SuiteRequest("nightly", source_sha, "main", "main")

            # The branch push succeeds before the draft-PR API call. Simulate a
            # runner death at exactly that second external write.
            with (
                mock.patch(
                    "bos_build.release.suite.create_pull_request",
                    side_effect=RuntimeError("runner stopped before PR creation"),
                ),
                self.assertRaisesRegex(RuntimeError, "before PR creation"),
            ):
                backend.create_transaction(
                    request,
                    transaction_branch("nightly", source_sha),
                    versions,
                    browser_version,
                    offset,
                )

            branch = transaction_branch("nightly", source_sha)
            self.assertTrue(
                self._git(remote, "show-ref", "--verify", f"refs/heads/{branch}")
            )
            with (
                mock.patch(
                    "bos_build.release.suite.list_pull_requests", return_value=[]
                ),
                mock.patch(
                    "bos_build.release.candidate.list_pull_requests", return_value=[]
                ),
                mock.patch(
                    "bos_build.release.candidate.list_github_releases",
                    return_value=[],
                ),
            ):
                browser_allocations = backend.discover_browser_allocations()
                component_allocations = backend.discover_allocations()

            self.assertEqual(
                browser_allocations,
                (
                    BrowserAllocation(
                        transaction_id=transaction_id("nightly", source_sha),
                        browser_version=browser_version,
                        build_offset=offset,
                    ),
                ),
            )
            self.assertEqual(
                {
                    (item.component, item.version, item.candidate_id)
                    for item in component_allocations
                },
                {
                    (component, versions[component], branch)
                    for component in SUITE_RELEASE_COMPONENTS
                },
            )

            # A later frozen source must allocate strictly beyond the orphan,
            # even though GitHub still has no PR marker for the first branch.
            note = repo / "later-source.txt"
            note.write_text("later source\n", encoding="utf-8")
            self._git(repo, "add", note.name)
            self._git(repo, "commit", "-m", "later source")
            self._git(repo, "push", "origin", "main")
            later_source = self._git(repo, "rev-parse", "HEAD")
            later_request = SuiteRequest("nightly", later_source, "main", "main")
            with (
                mock.patch(
                    "bos_build.release.suite.list_pull_requests", return_value=[]
                ),
                mock.patch(
                    "bos_build.release.candidate.list_pull_requests", return_value=[]
                ),
                mock.patch(
                    "bos_build.release.candidate.list_github_releases",
                    return_value=[],
                ),
                mock.patch(
                    "bos_build.release.suite.create_pull_request",
                    return_value="https://github.com/owner/repo/pull/78",
                ),
            ):
                later = reconcile_transaction(later_request, backend)

            self.assertNotEqual(later.browser_version, browser_version)
            self.assertGreater(later.build_offset, offset)
            for component in SUITE_RELEASE_COMPONENTS:
                self.assertNotEqual(
                    later.component_versions[component], versions[component]
                )

    def test_orphan_reservation_source_must_be_on_default_branch(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo, _, _ = self._repository(Path(temp_dir))
            off_main = repo / "off-main.txt"
            off_main.write_text("not on the remote default branch\n", encoding="utf-8")
            self._git(repo, "add", off_main.name)
            self._git(repo, "commit", "-m", "off-main source")
            source_sha = self._git(repo, "rev-parse", "HEAD")
            backend = GitHubSuiteBackend(repo, "owner/repo", "main")
            committed = backend.read_committed_versions()
            versions = {
                component: (
                    committed[component]
                    if component in SUITE_ONBOARDING_COMPONENTS
                    else increment_component_version(component, committed[component])
                )
                for component in SUITE_COMPONENTS
            }
            major, minor, build = backend.read_browser_version().split(".")[:3]
            with mock.patch(
                "bos_build.release.suite.create_pull_request",
                return_value="https://github.com/owner/repo/pull/79",
            ):
                backend.create_transaction(
                    SuiteRequest("nightly", source_sha, "main", "main"),
                    transaction_branch("nightly", source_sha),
                    versions,
                    f"{major}.{minor}.{int(build) + 1}",
                    backend.read_build_offset() + 1,
                )

            with self.assertRaisesRegex(ValueError, "source is not on main"):
                backend.discover_branch_reservations()

    def test_reservation_branch_and_state_reconcile_recover_exact_effects(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo, remote, source_sha = self._repository(Path(temp_dir))
            backend = GitHubSuiteBackend(repo, "owner/repo", "main")
            committed = backend.read_committed_versions()
            versions = {
                component: (
                    committed[component]
                    if component in SUITE_ONBOARDING_COMPONENTS
                    else increment_component_version(component, committed[component])
                )
                for component in SUITE_COMPONENTS
            }
            current_browser = backend.read_browser_version()
            major, minor, build = current_browser.split(".")[:3]
            browser_version = f"{major}.{minor}.{int(build) + 1}"
            request = SuiteRequest("nightly", source_sha, "main", "main")

            with (
                mock.patch(
                    "bos_build.release.suite.create_pull_request",
                    return_value="https://github.com/owner/repo/pull/77",
                ) as create_pr,
            ):
                record = backend.create_transaction(
                    request,
                    transaction_branch("nightly", source_sha),
                    versions,
                    browser_version,
                    backend.read_build_offset() + 1,
                )

                # This is the interrupted-push window: the same reservation tree
                # is recovered from the deterministic remote branch.
                recovered = backend.create_transaction(
                    request,
                    transaction_branch("nightly", source_sha),
                    versions,
                    browser_version,
                    backend.read_build_offset() + 1,
                )

            self.assertTrue(create_pr.call_args.kwargs["draft"])
            self.assertEqual(recovered.reservation_sha, record.reservation_sha)
            self.assertEqual(
                self._git(remote, "rev-parse", f"refs/heads/{record.branch}"),
                record.reservation_sha,
            )

            state_root = Path(temp_dir) / "state"
            for index, relative in enumerate(SUITE_STATE_PATHS):
                target = state_root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(f"snapshot-{index}\n", encoding="utf-8")

            reconciled = backend.reconcile_state(record, state_root)
            self.assertNotEqual(reconciled.state_sha, record.reservation_sha)
            self.assertEqual(
                backend.reconcile_state(reconciled, state_root), reconciled
            )

            # A later Actions job starts from a fresh main checkout. Inspection
            # must fetch the open transaction branch before reading state bytes.
            fresh = Path(temp_dir) / "fresh"
            self._git(
                Path(temp_dir),
                "clone",
                "--branch",
                "main",
                str(remote),
                str(fresh),
            )
            fresh_backend = GitHubSuiteBackend(fresh, "owner/repo", "main")
            pull_request = {
                "body": reconciled.pull_request_body(),
                "baseRefName": "main",
                "headRefName": reconciled.branch,
                "headRefOid": reconciled.state_sha,
                "headRepository": {"nameWithOwner": "owner/repo"},
                "isCrossRepository": False,
                "number": reconciled.pull_request_number,
                "url": reconciled.pull_request_url,
                "state": "OPEN",
                "mergedAt": None,
                "mergeCommit": None,
            }
            with mock.patch(
                "bos_build.release.suite.list_pull_requests",
                return_value=[pull_request],
            ):
                inspected = fresh_backend.find_transaction(request)
            self.assertIsNotNone(inspected)
            assert inspected is not None
            self.assertEqual(inspected.state_sha, reconciled.state_sha)
            self.assertEqual(inspected.state_checksums, reconciled.state_checksums)

            # A same-repository writer can push to the suite branch. Recovery
            # must reject an arbitrary code overlay before workflow outputs make
            # that head available to either signed browser build.
            attack = Path(temp_dir) / "attack"
            self._git(
                repo, "worktree", "add", "--detach", str(attack), reconciled.state_sha
            )
            try:
                injected = attack / "packages/browseros/INJECTED.txt"
                injected.write_text("not frozen source\n", encoding="utf-8")
                self._git(attack, "add", injected.relative_to(attack).as_posix())
                self._git(attack, "commit", "-m", "inject code")
                attacked_sha = self._git(attack, "rev-parse", "HEAD")
                self._git(attack, "push", "origin", f"HEAD:refs/heads/{record.branch}")
            finally:
                self._git(repo, "worktree", "remove", "--force", str(attack))
            with (
                mock.patch(
                    "bos_build.release.suite.list_pull_requests",
                    return_value=[{**pull_request, "headRefOid": attacked_sha}],
                ),
                self.assertRaisesRegex(ValueError, "unexpected files"),
            ):
                fresh_backend.find_transaction(request)

            # Restore the safe tree with a forward-only revert so this test also
            # obeys the production rule that suite branches are never force-pushed.
            repair = Path(temp_dir) / "repair"
            self._git(repo, "worktree", "add", "--detach", str(repair), attacked_sha)
            try:
                self._git(repair, "revert", "--no-edit", "HEAD")
                repaired_sha = self._git(repair, "rev-parse", "HEAD")
                self._git(repair, "push", "origin", f"HEAD:refs/heads/{record.branch}")
            finally:
                self._git(repo, "worktree", "remove", "--force", str(repair))
            reconciled = replace(reconciled, state_sha=repaired_sha)
            pull_request = {**pull_request, "headRefOid": repaired_sha}

            (state_root / SUITE_STATE_PATHS[0]).write_text(
                "conflicting replay\n", encoding="utf-8"
            )
            with self.assertRaisesRegex(ValueError, "conflicts"):
                backend.reconcile_state(reconciled, state_root)

            # A standalone feed writer can land while this long transaction is
            # building. The suite must detect that foreign snapshot commit
            # before making its draft PR ready, then recover after a true revert.
            foreign_path = repo / SUITE_STATE_PATHS[0]
            foreign_path.write_text("newer standalone snapshot\n", encoding="utf-8")
            self._git(repo, "add", SUITE_STATE_PATHS[0])
            self._git(repo, "commit", "-m", "foreign feed update")
            self._git(repo, "push", "origin", "main")
            self.assertTrue(backend.default_branch_contains_transaction(reconciled))
            self._git(repo, "revert", "--no-edit", "HEAD")
            self._git(repo, "push", "origin", "main")
            self.assertFalse(backend.default_branch_contains_transaction(reconciled))

            # The exact PR-head ref outlives GitHub's deletion of the same-repo
            # source branch. A full workflow retry recovers reservation metadata
            # from the merged PR and still builds the original overlay tree.
            self._git(
                repo,
                "push",
                "origin",
                f"{reconciled.state_sha}:refs/pull/77/head",
            )
            self._git(repo, "checkout", "main")
            self._git(repo, "merge", "--squash", reconciled.state_sha)
            self._git(repo, "commit", "-m", "merge suite state")
            merge_sha = self._git(repo, "rev-parse", "HEAD")
            self._git(repo, "push", "origin", "main")
            self._git(repo, "push", "origin", "--delete", reconciled.branch)
            merged_pull_request = {
                **pull_request,
                "isDraft": False,
                "state": "MERGED",
                "mergedAt": "2026-08-29T00:00:00Z",
                "mergeCommit": {"oid": merge_sha},
            }
            with mock.patch(
                "bos_build.release.suite.list_pull_requests",
                return_value=[merged_pull_request],
            ):
                merged = fresh_backend.find_transaction(request)
            self.assertIsNotNone(merged)
            assert merged is not None
            self.assertEqual(merged.state_sha, reconciled.state_sha)
            self.assertEqual(merged.merge_sha, merge_sha)
            self.assertEqual(merged.state_ref(), "refs/pull/77/head")
            self.assertEqual(
                self._git(
                    repo,
                    "ls-remote",
                    "--heads",
                    "origin",
                    f"refs/heads/{reconciled.branch}",
                ),
                "",
            )
            self._git(
                fresh,
                "fetch",
                "origin",
                f"{merged.state_ref()}:refs/remotes/origin/release-state",
            )
            self.assertEqual(
                self._git(fresh, "rev-parse", "refs/remotes/origin/release-state"),
                reconciled.state_sha,
            )


if __name__ == "__main__":
    unittest.main()
