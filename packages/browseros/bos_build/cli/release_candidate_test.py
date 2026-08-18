#!/usr/bin/env python3
"""Candidate CLI tests."""

import json
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from typer.testing import CliRunner

from bos_build.browseros import app
from bos_build.release.candidate import CandidateRecord
from bos_build.release.candidate_test import PARENT_SHA, candidate_record


runner = CliRunner()


class CandidateCliTest(unittest.TestCase):
    @patch("bos_build.cli.release_candidate.GitHubCandidateBackend")
    @patch("bos_build.cli.release_candidate.ensure_candidate")
    def test_ensure_writes_record_and_github_outputs(self, ensure, backend) -> None:
        ensure.return_value = candidate_record()
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            record_path = root / "candidate.json"
            github_output = root / "github-output"
            github_summary = root / "github-summary"
            result = runner.invoke(
                app,
                [
                    "release",
                    "candidate",
                    "ensure",
                    "--product",
                    "browseros",
                    "--parent-sha",
                    PARENT_SHA,
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
            self.assertEqual(
                json.loads(record_path.read_text())["candidate_sha"],
                candidate_record().candidate_sha,
            )
            outputs = dict(
                line.split("=", 1) for line in github_output.read_text().splitlines()
            )
            self.assertEqual(outputs["candidate_sha"], candidate_record().candidate_sha)
            self.assertEqual(outputs["branch"], candidate_record().branch)
            self.assertEqual(outputs["pull_request_number"], "42")
            self.assertIn("Browser release candidate", github_summary.read_text())
            backend.assert_called_once()

    @patch("bos_build.cli.release_candidate.GitHubCandidateBackend")
    @patch("bos_build.cli.release_candidate.merge_candidate")
    def test_merge_reads_gate_and_updates_record(self, merge, backend) -> None:
        merged = replace(candidate_record(state="merged"), merge_sha="3" * 40)
        merge.return_value = merged
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            record_path = root / "candidate.json"
            gate_path = root / "gate.json"
            github_output = root / "github-output"
            record_path.write_text(candidate_record().to_json())
            gate_path.write_text(
                json.dumps({"passed": True, "candidate_sha": candidate_record().candidate_sha})
            )

            result = runner.invoke(
                app,
                [
                    "release",
                    "candidate",
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
            self.assertEqual(CandidateRecord.from_path(record_path), merged)
            self.assertIn(
                f"merge_sha={merged.merge_sha}", github_output.read_text()
            )


if __name__ == "__main__":
    unittest.main()
