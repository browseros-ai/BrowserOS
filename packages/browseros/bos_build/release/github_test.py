#!/usr/bin/env python3
"""GitHub command adapter tests."""

import json
import subprocess
import unittest

from bos_build.release.github import (
    create_pull_request,
    edit_pull_request_body,
    list_pull_requests,
    merge_pull_request,
)


class RecordingRunner:
    def __init__(self, stdout: str = "") -> None:
        self.stdout = stdout
        self.calls = []

    def __call__(self, command, **kwargs):
        self.calls.append((command, kwargs))
        return subprocess.CompletedProcess(command, 0, self.stdout, "")


class PullRequestAdapterTest(unittest.TestCase):
    def test_lists_pull_requests_as_json(self) -> None:
        runner = RecordingRunner(stdout=json.dumps([{"number": 42}]))

        records = list_pull_requests(
            "browseros-ai/BrowserOS",
            state="all",
            head="bot/release-browseros",
            runner=runner,
        )

        self.assertEqual(records, [{"number": 42}])
        command = runner.calls[0][0]
        self.assertEqual(command[:3], ["gh", "pr", "list"])
        self.assertIn("--head", command)
        self.assertIn("--json", command)

    def test_creates_and_edits_pull_request_without_changing_git(self) -> None:
        create_runner = RecordingRunner(
            stdout="https://github.com/browseros-ai/BrowserOS/pull/42\n"
        )
        url = create_pull_request(
            repo="browseros-ai/BrowserOS",
            head="bot/release-browseros",
            base="main",
            title="chore(release): candidate",
            body="candidate body",
            runner=create_runner,
        )
        edit_runner = RecordingRunner()
        edit_pull_request_body(
            repo="browseros-ai/BrowserOS",
            number=42,
            body="updated body",
            runner=edit_runner,
        )

        self.assertTrue(url.endswith("/42"))
        self.assertEqual(create_runner.calls[0][0][:3], ["gh", "pr", "create"])
        self.assertEqual(edit_runner.calls[0][0][:3], ["gh", "pr", "edit"])

    def test_merges_pull_request_and_returns_merge_commit(self) -> None:
        runner = RecordingRunner(stdout=json.dumps({"mergeCommit": {"oid": "3" * 40}}))

        sha = merge_pull_request(
            "browseros-ai/BrowserOS",
            42,
            expected_head_sha="2" * 40,
            runner=runner,
        )

        self.assertEqual(sha, "3" * 40)
        command = runner.calls[0][0]
        self.assertEqual(command[:3], ["gh", "pr", "merge"])
        self.assertIn("--squash", command)
        self.assertEqual(command[-2:], ["--match-head-commit", "2" * 40])
        self.assertNotIn("--delete-branch", command)


if __name__ == "__main__":
    unittest.main()
