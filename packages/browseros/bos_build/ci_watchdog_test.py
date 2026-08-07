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

    def load_workflow(self, workflow_name: str) -> dict[str, object]:
        workflow_path = REPO_ROOT / ".github" / "workflows" / workflow_name
        return yaml.safe_load(workflow_path.read_text(encoding="utf-8"))

    def load_watchdog_step(self, workflow_name: str) -> dict[str, object]:
        workflow = self.load_workflow(workflow_name)
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
                workflow = self.load_workflow(workflow_name)
                step = self.load_watchdog_step(workflow_name)
                env = step["env"]
                self.assertEqual(env["RUNNER_LABEL"], expected_label)
                self.assertEqual(
                    env["RUNNER_LABEL"],
                    workflow["jobs"]["build"]["with"]["runner"],
                )

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

    def test_release_workflow_changes_trigger_these_tests(self):
        test_workflow = self.load_workflow("bos-build-tests.yml")
        # PyYAML's YAML 1.1 resolver treats the GitHub Actions `on` key as a
        # boolean, so accept either representation here.
        triggers = test_workflow.get("on", test_workflow.get(True))
        pull_request_paths = triggers["pull_request"]["paths"]

        for workflow_name in self.WORKFLOWS:
            with self.subTest(workflow=workflow_name):
                self.assertIn(
                    f".github/workflows/{workflow_name}",
                    pull_request_paths,
                )


class WarpBuildRunbookTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        runbook_path = (
            REPO_ROOT
            / "packages"
            / "browseros"
            / "bos_build"
            / "docs"
            / "warpbuild-ci.md"
        )
        cls.runbook = runbook_path.read_text(encoding="utf-8")
        paragraphs = [
            " ".join(paragraph.split())
            for paragraph in cls.runbook.split("\n\n")
        ]
        cls.windows_generation_guidance = next(
            paragraph
            for paragraph in paragraphs
            if paragraph.startswith(
                "WarpBuild's Windows Server 2025 image is "
                "Hypervisor Generation 1"
            )
        )
        troubleshooting = cls.runbook.split(
            "## Troubleshooting: jobs stuck in `queued`",
            maxsplit=1,
        )[1]
        cls.troubleshooting = " ".join(troubleshooting.split())

    def test_windows_runner_table_uses_live_sku(self):
        self.assertIn(
            "| Windows x64 | "
            "`warp-custom-browseros-windows-2025-x64-32x` | "
            "Windows Server 2025 | `Standard_D32as_v5` | P30, 1024 GB |",
            self.runbook,
        )
        self.assertNotIn(
            "Windows Server 2025 | `Standard_D32ls_v5` |",
            self.runbook,
        )
        self.assertNotIn(
            "Windows Server 2025 | `Standard_D32als_v7` |",
            self.runbook,
        )

    def test_windows_image_generation_constraint_is_documented(self):
        self.assertIn(
            "WarpBuild's Windows Server 2025 image is Hypervisor Generation 1",
            self.windows_generation_guidance,
        )
        self.assertRegex(
            self.windows_generation_guidance,
            r"`Standard_D32as_v5` supports both Generation 1\s+and 2",
        )
        self.assertIn(
            "Dalsv7 is Generation 2-only",
            self.windows_generation_guidance,
        )

    def test_windows_capacity_history_is_distinct_from_compatibility(self):
        self.assertRegex(
            self.windows_generation_guidance,
            r"`Standard_D32ls_v5` is Gen1-compatible [^.]* "
            r"East US returned HTTP 409 `SkuNotAvailable`; [^.]* "
            r"regional-capacity failure, not an image-generation mismatch",
        )

    def test_troubleshooting_calls_out_stack_launch_errors(self):
        self.assertIn("BYOC stack's launch errors", self.troubleshooting)
        self.assertIn("LaunchInstances", self.troubleshooting)
        self.assertRegex(
            self.troubleshooting,
            r"An Azure 400 [^.]* image/VM-size generation mismatch [^.]* "
            r"choose a Gen1-compatible size",
        )
        self.assertRegex(
            self.troubleshooting,
            r"An Azure 409 `SkuNotAvailable` [^.]*\. "
            r"Check quota and regional capacity [^.]* "
            r"choose a regionally available Gen1-compatible size",
        )
        self.assertRegex(
            self.troubleshooting,
            r"`Standard_D32as_v4` is the verified-family fallback [^.]* "
            r"confirm its capacity in East US before switching",
        )


if __name__ == "__main__":
    unittest.main()
