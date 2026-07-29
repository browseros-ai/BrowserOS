#!/usr/bin/env python3
"""Regression tests for the WarpBuild release queue-watchdogs."""

import json
import subprocess
import unittest
from pathlib import Path

import yaml


REPO_ROOT = Path(__file__).resolve().parents[3]


class WatchdogFilterTest(unittest.TestCase):
    WORKFLOWS = {
        "release-linux.yml": "warp-custom-browseros-ubuntu-2204-x64-32x",
        "release-windows.yml": "warp-custom-browseros-windows-2025-x64-32x",
    }

    JOBS_RESPONSE = {
        "jobs": [
            {
                "name": "Linux browser builds / build (browserclaw linux-x64)",
                "status": "queued",
                "labels": ["warp-custom-browseros-ubuntu-2204-x64-32x"],
            },
            {
                "name": "Linux browser builds / build (browseros linux-x64)",
                "status": "completed",
                "labels": ["warp-custom-browseros-ubuntu-2204-x64-32x"],
            },
            {
                "name": "Windows browser builds / build (browserclaw windows-x64)",
                "status": "in_progress",
                "labels": ["warp-custom-browseros-windows-2025-x64-32x"],
            },
            {
                "name": "Linux browser builds / queue-watchdog",
                "status": "in_progress",
                "labels": ["ubuntu-latest"],
            },
            {
                "name": "Windows browser builds / queue-watchdog",
                "status": "in_progress",
                "labels": ["ubuntu-latest"],
            },
            {
                "name": "macOS browser builds / build",
                "status": "in_progress",
                "labels": ["self-hosted", "browseros-builder"],
            },
            {
                "name": "Caller bookkeeping",
                "status": "completed",
            },
        ]
    }

    def load_watchdog_step(self, workflow_name: str) -> dict[str, object]:
        workflow_path = REPO_ROOT / ".github" / "workflows" / workflow_name
        workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
        steps = workflow["jobs"]["queue-watchdog"]["steps"]
        return next(
            step
            for step in steps
            if step["name"] == "Fail fast when no runner picks up the builds"
        )

    def apply_filter(self, job_filter: str, runner_label: str) -> list[dict[str, str]]:
        result = subprocess.run(
            [
                "jq",
                "-c",
                "--arg",
                "runner_label",
                runner_label,
                job_filter,
            ],
            input=json.dumps(self.JOBS_RESPONSE),
            capture_output=True,
            check=True,
            text=True,
        )
        return json.loads(result.stdout)

    def test_each_filter_selects_only_its_runner_jobs(self):
        expected_names = {
            "release-linux.yml": [
                "Linux browser builds / build (browserclaw linux-x64)",
                "Linux browser builds / build (browseros linux-x64)",
            ],
            "release-windows.yml": [
                "Windows browser builds / build (browserclaw windows-x64)",
            ],
        }

        for workflow_name, expected_label in self.WORKFLOWS.items():
            with self.subTest(workflow=workflow_name):
                step = self.load_watchdog_step(workflow_name)
                env = step["env"]
                self.assertEqual(env["RUNNER_LABEL"], expected_label)

                jobs = self.apply_filter(env["JOB_FILTER"], expected_label)

                self.assertEqual(
                    [job["name"] for job in jobs],
                    expected_names[workflow_name],
                )

    def test_filter_keeps_completed_matching_jobs(self):
        step = self.load_watchdog_step("release-linux.yml")
        env = step["env"]

        jobs = self.apply_filter(env["JOB_FILTER"], env["RUNNER_LABEL"])

        self.assertIn(
            {
                "name": "Linux browser builds / build (browseros linux-x64)",
                "status": "completed",
            },
            jobs,
        )

    def test_api_and_filter_failures_share_the_retry_guard(self):
        for workflow_name in self.WORKFLOWS:
            with self.subTest(workflow=workflow_name):
                run = self.load_watchdog_step(workflow_name)["run"]
                self.assertIn('if response="$(gh api ', run)
                self.assertIn(
                    '&& jobs="$(jq -c --arg runner_label "$RUNNER_LABEL" '
                    '"$JOB_FILTER" <<<"$response")"; then',
                    run,
                )
                self.assertIn('if [ "$failures" -ge 3 ]; then', run)
                self.assertNotIn("--jq", run)


if __name__ == "__main__":
    unittest.main()
