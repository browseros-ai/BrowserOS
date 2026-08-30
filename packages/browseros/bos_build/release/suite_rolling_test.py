#!/usr/bin/env python3
"""Behavior tests for crash-safe rolling family release reconciliation."""

import hashlib
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path

from bos_build.release.suite_rolling import (
    RollingAsset,
    RollingRelease,
    RollingReleaseRequest,
    reconcile_rolling_release,
    rolling_release_notes,
)


OLDER_SHA = "1" * 40
SOURCE_SHA = "2" * 40
NEWER_SHA = "3" * 40


class FakeRollingBackend:
    def __init__(self, request: RollingReleaseRequest):
        self.request = request
        self.release: RollingRelease | None = None
        self.target: str | None = None
        self.tag_target_results: list[str | None] = []
        self.ancestors: set[tuple[str, str]] = set()
        self.calls: list[str] = []
        self.leave_tag_after_release_delete = False
        self.corrupt_published_digest = False

    def find_release(self, tag: str) -> RollingRelease | None:
        self.calls.append("find-release")
        return self.release

    def tag_target(self, tag: str) -> str | None:
        self.calls.append("tag-target")
        if self.tag_target_results:
            return self.tag_target_results.pop(0)
        return self.target

    def is_ancestor(self, older_sha: str, newer_sha: str) -> bool:
        self.calls.append(f"ancestor:{older_sha[0]}:{newer_sha[0]}")
        return (older_sha, newer_sha) in self.ancestors

    def delete_release(self, tag: str) -> None:
        self.calls.append("delete-release")
        self.release = None
        if not self.leave_tag_after_release_delete:
            self.target = None

    def delete_tag(self, tag: str) -> None:
        self.calls.append("delete-tag")
        self.target = None

    def create_draft(self, request: RollingReleaseRequest, notes: str) -> None:
        self.calls.append("create-draft")
        self.release = RollingRelease(
            tag=request.tag,
            title=request.title,
            target_sha=request.source_sha,
            body=notes,
            draft=True,
            prerelease=True,
            assets=(),
        )

    def upload_asset(self, tag: str, asset: Path) -> None:
        self.calls.append("upload-asset")
        digest = RollingAsset(
            asset.name,
            "sha256:" + hashlib.sha256(asset.read_bytes()).hexdigest(),
        )
        assert self.release is not None
        self.release = replace(self.release, assets=(digest,))

    def publish_draft(self, tag: str) -> None:
        self.calls.append("publish-draft")
        assert self.release is not None
        assets = self.release.assets
        if self.corrupt_published_digest:
            assets = (replace(assets[0], digest="sha256:bad"),)
        self.release = replace(self.release, draft=False, assets=assets)
        self.target = self.release.target_sha


class RollingReleaseReconcileTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        asset = Path(self.temp_dir.name) / "BrowserOS_v1.2.3_arm64.dmg"
        asset.write_bytes(b"signed-dmg")
        self.request = RollingReleaseRequest(
            tag="nightly-browseros",
            title="BrowserOS Nightly (signed macOS arm64)",
            source_sha=SOURCE_SHA,
            browser_version="1.2.3",
            asset=asset,
        )
        self.asset = RollingAsset(
            asset.name,
            "sha256:" + hashlib.sha256(asset.read_bytes()).hexdigest(),
        )

    def release(
        self,
        *,
        source: str = SOURCE_SHA,
        version: str = "1.2.3",
        draft: bool = False,
        assets: tuple[RollingAsset, ...] | None = None,
    ) -> RollingRelease:
        request = replace(self.request, source_sha=source, browser_version=version)
        return RollingRelease(
            tag=request.tag,
            title=request.title,
            target_sha=source,
            body=rolling_release_notes(request),
            draft=draft,
            prerelease=True,
            assets=(self.asset,) if assets is None else assets,
        )

    def test_identical_published_release_is_success_without_writes(self) -> None:
        backend = FakeRollingBackend(self.request)
        backend.release = self.release()
        backend.target = SOURCE_SHA

        self.assertEqual(reconcile_rolling_release(self.request, backend), "reused")
        self.assertFalse(
            {"delete-release", "delete-tag", "create-draft", "upload-asset"}
            & set(backend.calls)
        )

    def test_published_release_with_deleted_tag_is_safely_recreated(self) -> None:
        backend = FakeRollingBackend(self.request)
        backend.release = self.release()

        self.assertEqual(reconcile_rolling_release(self.request, backend), "created")
        self.assertIn("delete-release", backend.calls)
        self.assertIn("create-draft", backend.calls)
        self.assertEqual(backend.target, SOURCE_SHA)
        self.assertEqual(backend.release, self.release())

    def test_release_and_tag_with_different_targets_fail_before_classification(
        self,
    ) -> None:
        backend = FakeRollingBackend(self.request)
        backend.release = self.release()
        backend.target = NEWER_SHA
        backend.ancestors.add((SOURCE_SHA, NEWER_SHA))

        with self.assertRaisesRegex(ValueError, "different sources"):
            reconcile_rolling_release(self.request, backend)
        self.assertNotIn("delete-release", backend.calls)
        self.assertNotIn("delete-tag", backend.calls)

    def test_stale_retry_never_deletes_a_newer_release_or_orphan_tag(self) -> None:
        for with_release in (True, False):
            with self.subTest(with_release=with_release):
                backend = FakeRollingBackend(self.request)
                backend.target = NEWER_SHA
                backend.ancestors.add((SOURCE_SHA, NEWER_SHA))
                if with_release:
                    backend.release = self.release(source=NEWER_SHA, version="1.2.4")

                self.assertEqual(
                    reconcile_rolling_release(self.request, backend), "superseded"
                )
                self.assertNotIn("delete-release", backend.calls)
                self.assertNotIn("delete-tag", backend.calls)
                self.assertNotIn("create-draft", backend.calls)

    def test_release_delete_crash_leaving_older_tag_is_recovered(self) -> None:
        backend = FakeRollingBackend(self.request)
        backend.release = self.release(source=OLDER_SHA, version="1.2.2")
        backend.target = OLDER_SHA
        backend.ancestors.add((OLDER_SHA, SOURCE_SHA))
        backend.leave_tag_after_release_delete = True

        self.assertEqual(reconcile_rolling_release(self.request, backend), "created")
        self.assertLess(
            backend.calls.index("delete-release"), backend.calls.index("delete-tag")
        )
        self.assertLess(
            backend.calls.index("delete-tag"), backend.calls.index("create-draft")
        )
        self.assertEqual(backend.target, SOURCE_SHA)
        self.assertEqual(backend.release, self.release())

    def test_partial_draft_reuses_verified_asset_and_finishes_publication(self) -> None:
        for assets, uploads in (((), 1), ((self.asset,), 0)):
            with self.subTest(assets=assets):
                backend = FakeRollingBackend(self.request)
                backend.release = self.release(draft=True, assets=assets)

                self.assertEqual(
                    reconcile_rolling_release(self.request, backend), "created"
                )
                self.assertEqual(backend.calls.count("upload-asset"), uploads)
                self.assertIn("publish-draft", backend.calls)
                self.assertEqual(backend.release, self.release())
                self.assertEqual(backend.target, SOURCE_SHA)

    def test_same_source_different_digest_fails_closed(self) -> None:
        backend = FakeRollingBackend(self.request)
        backend.release = self.release(
            draft=True,
            assets=(replace(self.asset, digest="sha256:conflict"),),
        )

        with self.assertRaisesRegex(ValueError, "conflicting assets"):
            reconcile_rolling_release(self.request, backend)
        self.assertNotIn("upload-asset", backend.calls)
        self.assertNotIn("publish-draft", backend.calls)

    def test_partial_draft_with_foreign_live_tag_fails_before_upload(self) -> None:
        backend = FakeRollingBackend(self.request)
        backend.release = self.release(draft=True, assets=())
        # The tag appears after the outer identity read but before the draft
        # upload, modeling an external/manual write outside the workflow lock.
        backend.tag_target_results = [None, NEWER_SHA]

        with self.assertRaisesRegex(ValueError, "different sources"):
            reconcile_rolling_release(self.request, backend)
        self.assertNotIn("upload-asset", backend.calls)
        self.assertNotIn("publish-draft", backend.calls)

    def test_publication_rereads_and_rejects_wrong_final_digest(self) -> None:
        backend = FakeRollingBackend(self.request)
        backend.corrupt_published_digest = True

        with self.assertRaisesRegex(ValueError, "conflicting assets"):
            reconcile_rolling_release(self.request, backend)
        self.assertGreaterEqual(backend.calls.count("find-release"), 3)
        self.assertIn("tag-target", backend.calls)


if __name__ == "__main__":
    unittest.main()
