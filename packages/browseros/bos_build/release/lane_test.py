#!/usr/bin/env python3
"""Tests for browser lane attestations and the release gate."""

import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from bos_build.core.context import ArtifactRegistry
from bos_build.release.lane import (
    ArtifactAttestation,
    LaneManifest,
    LaneOutcome,
    build_lane_manifest,
    gate_lane_manifests,
)
from bos_build.release.prepared_resources import PreparedResourcesManifest


CANDIDATE_SHA = "2" * 40
PARENT_SHA = "1" * 40
COMMON_DIGEST = "3" * 64


def _artifact(name: str, signed: bool = True) -> ArtifactAttestation:
    return ArtifactAttestation(
        filename=name,
        size=10,
        sha256="4" * 64,
        url=f"https://cdn.browseros.com/{name}",
        sparkle_signature="signature" if signed else "",
    )


def _manifest(
    lane_id: str,
    outcomes: tuple[LaneOutcome, ...],
    servers: dict[str, str],
    *,
    result: str = "success",
) -> LaneManifest:
    artifacts = {
        filename: _artifact(filename, outcome.signed)
        for outcome in outcomes
        for filename in outcome.artifacts
    }
    return LaneManifest(
        lane_id=lane_id,
        product="browseros",
        parent_sha=PARENT_SHA,
        candidate_sha=CANDIDATE_SHA,
        browser_version="0.31.0",
        component_versions={
            "server": "0.0.128",
            "agent": "0.0.116.0",
            "app-onboard": "0.0.12",
        },
        common_manifest_digest=COMMON_DIGEST,
        server_checksums=servers,
        artifacts=artifacts,
        outcomes={outcome.id: outcome for outcome in outcomes},
        toolchain={"runner": lane_id},
        result=result,
    )


def _complete_lanes() -> list[LaneManifest]:
    linux = LaneOutcome(
        id="linux-x64",
        artifacts=("BrowserOS_v0.31.0_x64.AppImage", "BrowserOS_v0.31.0_amd64.deb"),
        signed=False,
    )
    windows = LaneOutcome(
        id="windows-x64",
        artifacts=(
            "BrowserOS_v0.31.0_x64_installer.exe",
            "BrowserOS_v0.31.0_x64_installer.zip",
        ),
        signed=True,
    )
    mac_outcomes = tuple(
        LaneOutcome(
            id=f"macos-{architecture}",
            artifacts=(f"BrowserOS_v0.31.0_{architecture}.dmg",),
            signed=True,
        )
        for architecture in ("arm64", "x64", "universal")
    )
    return [
        _manifest("linux-x64", (linux,), {"linux-x64": "5" * 64}),
        _manifest("windows-x64", (windows,), {"windows-x64": "6" * 64}),
        _manifest(
            "macos-universal",
            mac_outcomes,
            {"darwin-arm64": "7" * 64, "darwin-x64": "8" * 64},
        ),
    ]


class LaneManifestTest(unittest.TestCase):
    def test_linux_lane_attests_the_exact_registered_pair(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            common = PreparedResourcesManifest(
                product="browseros",
                parent_sha=PARENT_SHA,
                source_sha=CANDIDATE_SHA,
                browser_version="0.31.0",
                component_versions={"server": "0.0.128"},
                files={},
            )
            (root / "prepared-resources.json").write_text(common.to_json())
            appimage = root / "BrowserOS_v0.31.0_x64.AppImage"
            deb = root / "BrowserOS_v0.31.0_amd64.deb"
            appimage.write_bytes(b"appimage")
            appimage.chmod(0o755)
            deb.write_bytes(b"deb")
            (root / "BrowserOS_v0.31.0_old.AppImage").write_bytes(b"stale")
            registry = ArtifactRegistry()
            registry.add("appimage", appimage)
            registry.add("deb", deb)
            registry.add(
                "server_resources",
                {"linux-x64": SimpleNamespace(manifest_sha256="5" * 64)},
            )
            context = SimpleNamespace(
                resource_mode="source",
                prepared_resources=root,
                artifact_registry=registry,
                architecture="x64",
                get_dist_dir=lambda: root,
                get_artifact_name=lambda kind: (
                    appimage.name if kind == "appimage" else deb.name
                ),
            )
            with (
                mock.patch("bos_build.release.lane.get_platform", return_value="linux"),
                mock.patch(
                    "bos_build.steps.storage.upload.IS_MACOS",
                    return_value=False,
                ),
                mock.patch(
                    "bos_build.steps.storage.upload.IS_WINDOWS",
                    return_value=False,
                ),
            ):
                lane = build_lane_manifest([context], {"runner": "warp"})

        self.assertEqual(lane.lane_id, "linux-x64")
        self.assertEqual(
            lane.outcomes["linux-x64"].artifacts,
            (appimage.name, deb.name),
        )
        self.assertEqual(set(lane.artifacts), {appimage.name, deb.name})

    def test_completed_macos_build_emits_packages_signatures_and_toolchain(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            common = PreparedResourcesManifest(
                product="browseros",
                parent_sha=PARENT_SHA,
                source_sha=CANDIDATE_SHA,
                browser_version="0.31.0",
                component_versions={"server": "0.0.128"},
                files={},
            )
            (root / "prepared-resources.json").write_text(common.to_json())
            paths = []
            signatures = {}
            for architecture in ("arm64", "x64", "universal"):
                path = root / f"BrowserOS_v0.31.0_{architecture}.dmg"
                path.write_bytes(architecture.encode())
                paths.append(path)
                signatures[path.name] = (f"signature-{architecture}", path.stat().st_size)
            registry = ArtifactRegistry()
            registry.add("sparkle_signatures", signatures)
            registry.add(
                "server_resources",
                {
                    "darwin-arm64": SimpleNamespace(manifest_sha256="7" * 64),
                    "darwin-x64": SimpleNamespace(manifest_sha256="8" * 64),
                },
            )
            context = SimpleNamespace(
                resource_mode="source",
                prepared_resources=root,
                artifact_registry=registry,
            )
            with (
                mock.patch("bos_build.release.lane.get_platform", return_value="macos"),
                mock.patch("bos_build.release.lane.detect_artifacts", return_value=paths),
            ):
                lane = build_lane_manifest([context], {"runner": "warp"})

        self.assertEqual(lane.lane_id, "macos-universal")
        self.assertEqual(lane.toolchain["runner"], "warp")
        self.assertEqual(set(lane.artifacts), {path.name for path in paths})
        self.assertTrue(all(outcome.signed for outcome in lane.outcomes.values()))
        self.assertTrue(
            all(artifact.sparkle_signature for artifact in lane.artifacts.values())
        )

    def test_round_trip_preserves_explicit_provenance(self) -> None:
        manifest = _complete_lanes()[2]
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "lane.json"
            path.write_text(manifest.to_json())
            loaded = LaneManifest.from_path(path)

        self.assertEqual(loaded, manifest)
        self.assertEqual(loaded.parent_sha, PARENT_SHA)
        self.assertEqual(loaded.candidate_sha, CANDIDATE_SHA)
        self.assertEqual(set(loaded.server_checksums), {"darwin-arm64", "darwin-x64"})
        self.assertEqual(set(loaded.outcomes), {"macos-arm64", "macos-x64", "macos-universal"})

    def test_gate_accepts_complete_consistent_matrix(self) -> None:
        gate = gate_lane_manifests(_complete_lanes())

        self.assertTrue(gate.passed)
        self.assertEqual(gate.candidate_sha, CANDIDATE_SHA)
        self.assertEqual(
            set(gate.outcomes),
            {
                "linux-x64",
                "windows-x64",
                "macos-arm64",
                "macos-x64",
                "macos-universal",
            },
        )
        self.assertEqual(type(gate).from_dict(gate.to_dict()), gate)

    def test_gate_rejects_missing_duplicate_and_failed_lanes(self) -> None:
        with self.assertRaisesRegex(ValueError, "lane set"):
            gate_lane_manifests(_complete_lanes()[:-1])
        with self.assertRaisesRegex(ValueError, "duplicate lane"):
            gate_lane_manifests([*_complete_lanes(), _complete_lanes()[0]])
        failed = [*_complete_lanes()]
        failed[0] = replace(failed[0], result="failed")
        with self.assertRaisesRegex(ValueError, "failed"):
            gate_lane_manifests(failed)

    def test_gate_rejects_swapped_lane_evidence(self) -> None:
        lanes = _complete_lanes()
        linux, windows = lanes[:2]
        lanes[0] = replace(windows, lane_id="linux-x64")
        lanes[1] = replace(linux, lane_id="windows-x64")

        with self.assertRaisesRegex(ValueError, "invalid outcomes"):
            gate_lane_manifests(lanes)

    def test_gate_rejects_unsigned_required_outcome(self) -> None:
        lanes = _complete_lanes()
        windows = lanes[1]
        outcome = replace(windows.outcomes["windows-x64"], signed=False)
        lanes[1] = replace(
            windows,
            outcomes={"windows-x64": outcome},
        )

        with self.assertRaisesRegex(ValueError, "signed"):
            gate_lane_manifests(lanes)

    def test_gate_rejects_candidate_version_and_common_digest_skew(self) -> None:
        mutations = (
            ("candidate_sha", "9" * 40, "candidate"),
            ("browser_version", "0.32.0", "browser version"),
            ("common_manifest_digest", "9" * 64, "common"),
        )
        for field, value, message in mutations:
            with self.subTest(field=field):
                lanes = _complete_lanes()
                lanes[1] = replace(lanes[1], **{field: value})
                with self.assertRaisesRegex(ValueError, message):
                    gate_lane_manifests(lanes)

    def test_macos_lane_requires_both_server_targets_and_all_packages(self) -> None:
        lanes = _complete_lanes()
        lanes[2] = replace(
            lanes[2], server_checksums={"darwin-arm64": "7" * 64}
        )
        with self.assertRaisesRegex(ValueError, "darwin-x64"):
            gate_lane_manifests(lanes)

        lanes = _complete_lanes()
        outcomes = dict(lanes[2].outcomes)
        outcomes.pop("macos-universal")
        lanes[2] = replace(lanes[2], outcomes=outcomes)
        with self.assertRaisesRegex(ValueError, "macos-universal"):
            gate_lane_manifests(lanes)


if __name__ == "__main__":
    unittest.main()
