#!/usr/bin/env python3
"""Family release suite CLI tests."""

import json
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from typer.testing import CliRunner

from bos_build.browseros import app
from bos_build.release.suite import SuiteRecord
from bos_build.release.suite_test import SOURCE_SHA, STATE_SHA, suite_record


runner = CliRunner()


class SuiteCliTest(unittest.TestCase):
    @patch("bos_build.cli.release_suite.GitHubRollingReleaseBackend")
    @patch("bos_build.cli.release_suite.reconcile_rolling_release")
    def test_rolling_release_command_selects_one_signed_dmg(
        self, reconcile, backend
    ) -> None:
        reconcile.return_value = "reused"
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            artifact_root = root / "artifact"
            artifact_root.mkdir()
            dmg = artifact_root / "BrowserOS_v1.2.3_arm64.dmg"
            dmg.write_bytes(b"signed")
            result = runner.invoke(
                app,
                [
                    "release",
                    "suite",
                    "reconcile-rolling-release",
                    "--tag",
                    "nightly-browseros",
                    "--title",
                    "BrowserOS Nightly",
                    "--source-sha",
                    SOURCE_SHA,
                    "--browser-version",
                    "1.2.3",
                    "--artifact-root",
                    str(artifact_root),
                    "--repo",
                    "browseros-ai/BrowserOS",
                    "--repo-root",
                    str(root),
                ],
            )

        self.assertEqual(result.exit_code, 0, result.output)
        self.assertEqual(result.output, "reused\n")
        request = reconcile.call_args.args[0]
        self.assertEqual(request.asset, dmg)
        self.assertEqual(request.source_sha, SOURCE_SHA)
        backend.assert_called_once_with("browseros-ai/BrowserOS", root.resolve())

    @patch("bos_build.cli.release_suite.GitHubSuiteBackend")
    @patch("bos_build.cli.release_suite.reconcile_transaction")
    def test_reconcile_writes_record_summary_and_distinct_identities(
        self, reconcile, backend
    ) -> None:
        reconcile.return_value = suite_record(state_sha=STATE_SHA)
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            record_path = root / "suite.json"
            github_output = root / "github-output"
            github_summary = root / "github-summary"
            result = runner.invoke(
                app,
                [
                    "release",
                    "suite",
                    "reconcile",
                    "--mode",
                    "nightly",
                    "--source-sha",
                    SOURCE_SHA,
                    "--default-branch",
                    "main",
                    "--dispatch-ref",
                    "main",
                    "--repo-root",
                    str(root),
                    "--repo",
                    "browseros-ai/BrowserOS",
                    "--output",
                    str(record_path),
                    "--github-output",
                    str(github_output),
                    "--github-summary",
                    str(github_summary),
                ],
            )

            self.assertEqual(result.exit_code, 0, result.output)
            self.assertEqual(SuiteRecord.from_path(record_path), reconcile.return_value)
            outputs = dict(
                line.split("=", 1) for line in github_output.read_text().splitlines()
            )
            self.assertEqual(outputs["source_sha"], SOURCE_SHA)
            self.assertEqual(outputs["reservation_sha"], suite_record().reservation_sha)
            self.assertEqual(outputs["state_sha"], STATE_SHA)
            self.assertEqual(outputs["state_ref"], suite_record().branch)
            self.assertEqual(outputs["server_version"], "0.0.147")
            self.assertEqual(outputs["agent_version"], "0.0.121.0")
            self.assertEqual(outputs["claw_server_version"], "0.0.46")
            self.assertEqual(outputs["browserclaw_version"], "0.0.83.0")
            self.assertEqual(outputs["onboarding_version"], "0.0.15")
            self.assertEqual(outputs["app_onboarding_version"], "0.0.0")
            self.assertIn("family release transaction", github_summary.read_text())
            reconcile.assert_called_once()
            backend.assert_called_once()

    @patch("bos_build.cli.release_suite.GitHubSuiteBackend")
    @patch("bos_build.cli.release_suite.inspect_transaction")
    def test_inspect_is_exposed_as_a_read_only_command(self, inspect, backend) -> None:
        inspect.return_value = suite_record(state_sha=STATE_SHA)
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            result = runner.invoke(
                app,
                [
                    "release",
                    "suite",
                    "inspect",
                    "--mode",
                    "nightly",
                    "--source-sha",
                    SOURCE_SHA,
                    "--default-branch",
                    "main",
                    "--dispatch-ref",
                    "main",
                    "--repo-root",
                    str(root),
                    "--repo",
                    "browseros-ai/BrowserOS",
                ],
            )

        self.assertEqual(result.exit_code, 0, result.output)
        self.assertEqual(json.loads(result.output)["state_sha"], STATE_SHA)
        inspect.assert_called_once()
        backend.assert_called_once()

    @patch("bos_build.cli.release_suite.GitHubSuiteBackend")
    @patch("bos_build.cli.release_suite.merge_transaction")
    def test_merge_updates_the_record_and_emits_merge_sha(self, merge, backend) -> None:
        merged = replace(
            suite_record(state_sha=STATE_SHA), state="merged", merge_sha="4" * 40
        )
        merge.return_value = merged
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            record_path = root / "suite.json"
            gate_path = root / "gate.json"
            github_output = root / "github-output"
            record_path.write_text(suite_record(state_sha=STATE_SHA).to_json())
            gate_path.write_text(
                json.dumps({"schema": "browseros-release-suite-gate-v1"})
            )

            result = runner.invoke(
                app,
                [
                    "release",
                    "suite",
                    "merge",
                    "--record",
                    str(record_path),
                    "--gate",
                    str(gate_path),
                    "--repo-root",
                    str(root),
                    "--repo",
                    "browseros-ai/BrowserOS",
                    "--github-output",
                    str(github_output),
                ],
            )

            self.assertEqual(result.exit_code, 0, result.output)
            self.assertEqual(SuiteRecord.from_path(record_path), merged)
            outputs = dict(
                line.split("=", 1) for line in github_output.read_text().splitlines()
            )
            self.assertEqual(outputs["merge_sha"], merged.merge_sha)
            self.assertEqual(outputs["state_sha"], STATE_SHA)
            self.assertEqual(outputs["state_ref"], "refs/pull/77/head")


if __name__ == "__main__":
    unittest.main()
