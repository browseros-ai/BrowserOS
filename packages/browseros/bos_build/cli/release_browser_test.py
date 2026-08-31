#!/usr/bin/env python3
"""CLI tests for browser release gating and finalization."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from typer.testing import CliRunner

from bos_build.browseros import app
from bos_build.release.candidate import CandidateRecord
from bos_build.release.lane import (
    ArtifactAttestation,
    LaneManifest,
    LaneOutcome,
    gate_lane_manifests,
)


PARENT_SHA = "1" * 40
CANDIDATE_SHA = "2" * 40
COMPONENTS = {
    "server": "0.0.128",
    "agent": "0.0.101.0",
    "app-onboard": "0.0.12",
}
runner = CliRunner()


def _lane(
    lane_id: str,
    outcomes: tuple[tuple[str, tuple[str, ...], bool], ...],
    servers: dict[str, str],
) -> LaneManifest:
    artifacts = {
        filename: ArtifactAttestation(
            filename,
            10,
            "9" * 64,
            f"https://cdn.browseros.com/{filename}",
            "signature" if signed else "",
        )
        for _, filenames, signed in outcomes
        for filename in filenames
    }
    return LaneManifest(
        lane_id=lane_id,
        product="browseros",
        parent_sha=PARENT_SHA,
        candidate_sha=CANDIDATE_SHA,
        browser_version="0.31.0",
        component_versions=COMPONENTS,
        common_manifest_digest="4" * 64,
        server_checksums=servers,
        artifacts=artifacts,
        outcomes={
            outcome_id: LaneOutcome(outcome_id, filenames, signed)
            for outcome_id, filenames, signed in outcomes
        },
        toolchain={"runner": lane_id},
        result="success",
    )


def _lanes() -> list[LaneManifest]:
    return [
        _lane(
            "linux-x64",
            ((
                "linux-x64",
                ("BrowserOS_v0.31.0_x64.AppImage", "BrowserOS_v0.31.0_amd64.deb"),
                False,
            ),),
            {"linux-x64": "5" * 64},
        ),
        _lane(
            "windows-x64",
            ((
                "windows-x64",
                (
                    "BrowserOS_v0.31.0_x64_installer.exe",
                    "BrowserOS_v0.31.0_x64_installer.zip",
                ),
                True,
            ),),
            {"windows-x64": "6" * 64},
        ),
        _lane(
            "macos-universal",
            tuple(
                (
                    f"macos-{architecture}",
                    (f"BrowserOS_v0.31.0_{architecture}.dmg",),
                    True,
                )
                for architecture in ("arm64", "x64", "universal")
            ),
            {"darwin-arm64": "7" * 64, "darwin-x64": "8" * 64},
        ),
    ]


def _candidate() -> CandidateRecord:
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
        state="merged",
        merge_sha="3" * 40,
    )


class BrowserReleaseCliTest(unittest.TestCase):
    def test_gate_writes_record_summary_and_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            lane_paths = []
            for lane in _lanes():
                path = root / f"{lane.lane_id}.json"
                path.write_text(lane.to_json())
                lane_paths.append(path)
            output = root / "gate.json"
            summary = root / "summary.md"
            github_output = root / "output.txt"
            args = ["release", "browser", "gate"]
            for path in lane_paths:
                args.extend(["--lane", str(path)])
            args.extend(
                [
                    "--output",
                    str(output),
                    "--github-summary",
                    str(summary),
                    "--github-output",
                    str(github_output),
                ]
            )
            result = runner.invoke(app, args)

            self.assertEqual(result.exit_code, 0, result.output)
            self.assertEqual(
                output.read_text(), gate_lane_manifests(_lanes()).to_json()
            )
            self.assertIn("Result: passed", summary.read_text())
            self.assertIn(f"candidate_sha={CANDIDATE_SHA}", github_output.read_text())

    def test_finalize_fetches_metadata_and_writes_result(self) -> None:
        final = SimpleNamespace(
            candidate_sha=CANDIDATE_SHA,
            merge_sha="3" * 40,
            draft=SimpleNamespace(tag="v0.31.0", url="https://release"),
            to_json=lambda: '{"schema":"final"}\n',
            summary=lambda: "## Finalized\n",
        )
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            candidate_path = root / "candidate.json"
            gate_path = root / "gate.json"
            candidate_path.write_text(_candidate().to_json())
            gate_path.write_text(gate_lane_manifests(_lanes()).to_json())
            output = root / "final.json"
            preview_dir = root / "previews"
            with (
                mock.patch(
                    "bos_build.cli.release_browser.fetch_all_release_metadata",
                    return_value={"metadata": {}},
                ) as fetch,
                mock.patch(
                    "bos_build.cli.release_browser.GitHubDraftBackend"
                ) as backend,
                mock.patch(
                    "bos_build.cli.release_browser.finalize_browser_release",
                    return_value=final,
                ) as finalize,
            ):
                result = runner.invoke(
                    app,
                    [
                        "release",
                        "browser",
                        "finalize",
                        "--candidate",
                        str(candidate_path),
                        "--gate",
                        str(gate_path),
                        "--repo",
                        "browseros-ai/BrowserOS",
                        "--preview-dir",
                        str(preview_dir),
                        "--output",
                        str(output),
                    ],
                )

            self.assertEqual(result.exit_code, 0, result.output)
            self.assertEqual(output.read_text(), final.to_json())
            fetch.assert_called_once_with("0.31.0", product_id="browseros")
            finalize.assert_called_once()
            self.assertIs(finalize.call_args.args[4], backend.return_value)


if __name__ == "__main__":
    unittest.main()
