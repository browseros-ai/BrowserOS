#!/usr/bin/env python3
"""Tests for browser-only release finalization."""

import hashlib
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest import mock

from bos_build.release.browser_finalize import (
    DraftState,
    GitHubDraftBackend,
    finalize_browser_release,
)
from bos_build.release.candidate import CandidateRecord
from bos_build.release.lane import ArtifactAttestation, LaneGate


PARENT_SHA = "1" * 40
CANDIDATE_SHA = "2" * 40
MERGE_SHA = "3" * 40
COMPONENTS = {
    "server": "0.0.128",
    "agent": "0.0.101.0",
    "app-onboard": "0.0.12",
}
ARTIFACTS = {
    "macos": {
        "arm64": "BrowserOS_v0.31.0_arm64.dmg",
        "x64": "BrowserOS_v0.31.0_x64.dmg",
        "universal": "BrowserOS_v0.31.0_universal.dmg",
    },
    "win": {
        "x64_installer": "BrowserOS_v0.31.0_x64_installer.exe",
        "x64_zip": "BrowserOS_v0.31.0_x64_installer.zip",
    },
    "linux": {
        "x64_appimage": "BrowserOS_v0.31.0_x64.AppImage",
        "x64_deb": "BrowserOS_v0.31.0_amd64.deb",
    },
}


def _checksum(filename: str) -> str:
    return hashlib.sha256(filename.encode()).hexdigest()


def _candidate(state: str = "merged") -> CandidateRecord:
    return CandidateRecord(
        product="browseros",
        parent_sha=PARENT_SHA,
        candidate_sha=CANDIDATE_SHA,
        default_branch="main",
        branch=f"bot/release-browseros-{PARENT_SHA[:12]}",
        browser_version="0.31.0",
        component_versions=COMPONENTS,
        pull_request_number=42,
        pull_request_url="https://github.com/browseros-ai/BrowserOS/pull/42",
        state=state,
        merge_sha=MERGE_SHA if state == "merged" else "",
    )


def _gate() -> LaneGate:
    return LaneGate(
        product="browseros",
        parent_sha=PARENT_SHA,
        candidate_sha=CANDIDATE_SHA,
        browser_version="0.31.0",
        component_versions=COMPONENTS,
        common_manifest_digest="4" * 64,
        lanes=("linux-x64", "macos-universal", "windows-x64"),
        outcomes=(
            "linux-x64",
            "macos-arm64",
            "macos-universal",
            "macos-x64",
            "windows-x64",
        ),
        server_checksums={
            "darwin-arm64": "5" * 64,
            "darwin-x64": "6" * 64,
            "linux-x64": "7" * 64,
            "windows-x64": "8" * 64,
        },
        artifacts={
            filename: ArtifactAttestation(
                filename=filename,
                size=len(filename),
                sha256=_checksum(filename),
                url=f"https://cdn.browseros.com/{filename}",
                sparkle_signature="signature" if platform in {"macos", "win"} else "",
            )
            for platform, platform_artifacts in ARTIFACTS.items()
            for filename in platform_artifacts.values()
        },
    )


def _metadata() -> dict[str, dict]:
    result = {}
    for platform, artifacts in ARTIFACTS.items():
        result[platform] = {
            "product": "browseros",
            "platform": platform,
            "version": "0.31.0",
            "source_sha": CANDIDATE_SHA,
            "parent_sha": PARENT_SHA,
            "component_versions": COMPONENTS,
            "common_manifest_digest": "4" * 64,
            "chromium_version": "136.0.0.0",
            "sparkle_version": "10000.0.31.0",
            "build_date": "2026-08-05T12:00:00+00:00",
            "artifacts": {
                key: {
                    "filename": filename,
                    "url": f"https://cdn.browseros.com/{filename}",
                    "size": len(filename),
                    "sha256": _checksum(filename),
                    "sparkle_signature": (
                        "signature" if platform in {"macos", "win"} else ""
                    ),
                    "sparkle_length": len(filename),
                }
                for key, filename in artifacts.items()
            },
        }
    return result


class FakeDraftBackend:
    def __init__(self) -> None:
        self.calls = []

    def ensure_draft(self, candidate, metadata):
        self.calls.append((candidate, metadata))
        return DraftState(
            tag="v0.31.0",
            url="https://github.com/browseros-ai/BrowserOS/releases/tag/v0.31.0",
            target_sha=candidate.candidate_sha,
            action="created",
            assets=tuple(
                sorted(
                    artifact["filename"]
                    for release in metadata.values()
                    for artifact in release["artifacts"].values()
                )
            ),
        )


class BrowserFinalizationTest(unittest.TestCase):
    def test_finalizes_browser_draft_and_writes_local_appcast_previews(self) -> None:
        backend = FakeDraftBackend()
        with tempfile.TemporaryDirectory() as tmp:
            record = finalize_browser_release(
                _candidate(), _gate(), _metadata(), Path(tmp), backend
            )
            previews = {path.name for path in Path(tmp).glob("*.xml")}

        self.assertEqual(
            previews,
            {"appcast.xml", "appcast-x86_64.xml", "appcast-win.xml"},
        )
        self.assertEqual(record.draft.target_sha, CANDIDATE_SHA)
        self.assertEqual(record.merge_sha, MERGE_SHA)
        self.assertEqual(len(backend.calls), 1)
        self.assertIn("publish separately", record.summary())

    def test_requires_merged_candidate_and_matching_gate_identity(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, "merged"):
                finalize_browser_release(
                    _candidate("open"), _gate(), _metadata(), Path(tmp), FakeDraftBackend()
                )
            with self.assertRaisesRegex(ValueError, "browser_version"):
                finalize_browser_release(
                    _candidate(),
                    replace(_gate(), browser_version="0.32.0"),
                    _metadata(),
                    Path(tmp),
                    FakeDraftBackend(),
                )

    def test_rejects_release_metadata_checksum_or_candidate_skew(self) -> None:
        mutations = (
            ("sha256", "9" * 64, "evidence"),
            ("sparkle_signature", "substituted", "evidence"),
            ("source_sha", "9" * 40, "source_sha"),
        )
        for field, value, message in mutations:
            with self.subTest(field=field), tempfile.TemporaryDirectory() as tmp:
                metadata = _metadata()
                if field in {"sha256", "sparkle_signature"}:
                    metadata["linux"]["artifacts"]["x64_deb"][field] = value
                else:
                    metadata["linux"][field] = value
                with self.assertRaisesRegex(RuntimeError, message):
                    finalize_browser_release(
                        _candidate(),
                        _gate(),
                        metadata,
                        Path(tmp),
                        FakeDraftBackend(),
                    )


class GitHubDraftBackendTest(unittest.TestCase):
    def test_exact_existing_draft_is_reused_without_asset_transfer(self) -> None:
        expected = sorted(
            filename
            for platform in ARTIFACTS.values()
            for filename in platform.values()
        )
        release = {
            "isDraft": True,
            "assets": expected,
            "asset_metadata": {
                filename: {
                    "sha256": _checksum(filename),
                    "size": len(filename),
                }
                for filename in expected
            },
            "targetCommitish": CANDIDATE_SHA,
        }
        with (
            mock.patch(
                "bos_build.release.browser_finalize.create_github_release",
                return_value=(False, "Release v0.31.0 already exists"),
            ),
            mock.patch(
                "bos_build.release.browser_finalize.inspect_github_release",
                return_value=release,
            ),
            mock.patch(
                "bos_build.release.browser_finalize.verify_github_release_target"
            ),
            mock.patch("bos_build.release.browser_finalize.edit_github_release"),
            mock.patch(
                "bos_build.release.browser_finalize.download_file"
            ) as download,
            mock.patch(
                "bos_build.release.browser_finalize.upload_to_github_release"
            ) as upload,
        ):
            state = GitHubDraftBackend(
                "browseros-ai/BrowserOS"
            ).ensure_draft(_candidate(), _metadata())

        self.assertEqual(state.action, "reused")
        self.assertEqual(list(state.assets), expected)
        download.assert_not_called()
        upload.assert_not_called()

    def test_same_named_assets_with_wrong_digest_are_replaced(self) -> None:
        expected = sorted(
            filename
            for platform in ARTIFACTS.values()
            for filename in platform.values()
        )
        stale = {
            "isDraft": True,
            "assets": expected,
            "asset_metadata": {
                filename: {"sha256": "0" * 64, "size": len(filename)}
                for filename in expected
            },
            "targetCommitish": CANDIDATE_SHA,
        }
        refreshed = {
            **stale,
            "asset_metadata": {
                filename: {
                    "sha256": _checksum(filename),
                    "size": len(filename),
                }
                for filename in expected
            },
        }

        def download(url, path):
            path.write_bytes(path.name.encode())
            return True

        with (
            mock.patch(
                "bos_build.release.browser_finalize.create_github_release",
                return_value=(False, "Release v0.31.0 already exists"),
            ),
            mock.patch(
                "bos_build.release.browser_finalize.inspect_github_release",
                side_effect=[stale, refreshed],
            ),
            mock.patch(
                "bos_build.release.browser_finalize.verify_github_release_target"
            ),
            mock.patch("bos_build.release.browser_finalize.edit_github_release"),
            mock.patch(
                "bos_build.release.browser_finalize.download_file",
                side_effect=download,
            ),
            mock.patch(
                "bos_build.release.browser_finalize.delete_github_release_asset"
            ) as delete,
            mock.patch(
                "bos_build.release.browser_finalize.upload_to_github_release",
                return_value=True,
            ) as upload,
        ):
            state = GitHubDraftBackend(
                "browseros-ai/BrowserOS"
            ).ensure_draft(_candidate(), _metadata())

        self.assertEqual(state.action, "refreshed")
        self.assertEqual(delete.call_count, len(expected))
        self.assertEqual(upload.call_count, len(expected))

    def test_download_failure_does_not_delete_existing_draft_assets(self) -> None:
        release = {
            "isDraft": True,
            "assets": ["BrowserOS_v0.31.0_arm64.dmg"],
            "targetCommitish": CANDIDATE_SHA,
        }
        with (
            mock.patch(
                "bos_build.release.browser_finalize.create_github_release",
                return_value=(False, "Release v0.31.0 already exists"),
            ),
            mock.patch(
                "bos_build.release.browser_finalize.inspect_github_release",
                return_value=release,
            ),
            mock.patch(
                "bos_build.release.browser_finalize.verify_github_release_target"
            ),
            mock.patch("bos_build.release.browser_finalize.edit_github_release"),
            mock.patch(
                "bos_build.release.browser_finalize.download_file",
                return_value=False,
            ),
            mock.patch(
                "bos_build.release.browser_finalize.delete_github_release_asset"
            ) as delete,
        ):
            with self.assertRaisesRegex(RuntimeError, "download"):
                GitHubDraftBackend("browseros-ai/BrowserOS").ensure_draft(
                    _candidate(), _metadata()
                )

        delete.assert_not_called()


if __name__ == "__main__":
    unittest.main()
