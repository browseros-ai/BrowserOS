#!/usr/bin/env python3
"""Tests for the git setup module's gclient handling against a mock checkout."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from .git import GitSetupModule, BROWSEROS_BRANCH
from ...common.context import Context
from ...common.module import ValidationError
from ...common.testing import MockBrowserOSRoot, MockChromium, make_context


class GitSetupValidateTest(unittest.TestCase):
    def test_missing_chromium_src_raises(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = MockBrowserOSRoot(Path(tmp) / "root")
            ctx = Context(
                root_dir=root.root,
                chromium_src=Path(tmp) / "missing-src",
                architecture="x64",
                build_type="release",
            )
            with self.assertRaises(ValidationError):
                GitSetupModule().validate(ctx)

    def test_missing_chromium_version_raises(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            root = MockBrowserOSRoot(Path(root_tmp))
            (root.root / "CHROMIUM_VERSION").unlink()
            ctx = make_context(MockChromium(Path(chromium_tmp)), root)
            self.assertEqual(ctx.chromium_version, "")
            with self.assertRaises(ValidationError):
                GitSetupModule().validate(ctx)

    def test_passes_with_src_and_version(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            ctx = make_context(
                MockChromium(Path(chromium_tmp)),
                MockBrowserOSRoot(Path(root_tmp)),
            )
            GitSetupModule().validate(ctx)


class GitSetupExecuteTest(unittest.TestCase):
    def setUp(self):
        self._chromium_tmp = tempfile.TemporaryDirectory()
        self._root_tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._chromium_tmp.cleanup)
        self.addCleanup(self._root_tmp.cleanup)
        self.ctx = make_context(
            MockChromium(Path(self._chromium_tmp.name)),
            MockBrowserOSRoot(Path(self._root_tmp.name)),
        )

    def test_checks_out_tag_as_browseros_branch(self):
        commands = []

        def fake_run_command(cmd, cwd=None):
            commands.append(cmd)

        with (
            mock.patch("build.modules.setup.git.run_command", fake_run_command),
            mock.patch("build.modules.setup.git.IS_LINUX", return_value=False),
            mock.patch("build.modules.setup.git.IS_WINDOWS", return_value=False),
            mock.patch.object(GitSetupModule, "_verify_tag_exists", return_value=None),
        ):
            GitSetupModule().execute(self.ctx)

        tag_ref = f"tags/{self.ctx.chromium_version}"
        # One checkout creates the branch straight from the tag; the redundant
        # detached-HEAD checkout that #1216 shipped (and was reverted) is gone.
        self.assertEqual(
            commands,
            [
                ["git", "fetch", "--tags", "--force"],
                ["git", "checkout", "-B", BROWSEROS_BRANCH, tag_ref],
                ["gclient", "sync", "-D", "--no-history", "--shallow"],
            ],
        )

    def test_missing_tag_stops_before_checkout(self):
        commands = []

        def fake_run_command(cmd, cwd=None):
            commands.append(cmd)

        with (
            mock.patch("build.modules.setup.git.run_command", fake_run_command),
            mock.patch.object(
                GitSetupModule,
                "_verify_tag_exists",
                side_effect=ValidationError("missing"),
            ),
        ):
            with self.assertRaises(ValidationError):
                GitSetupModule().execute(self.ctx)

        self.assertEqual(commands, [["git", "fetch", "--tags", "--force"]])


class EnsureGclientTargetCpusTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self._root_tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.addCleanup(self._root_tmp.cleanup)
        self.chromium = MockChromium(Path(self._tmp.name))
        self.ctx = make_context(
            self.chromium, MockBrowserOSRoot(Path(self._root_tmp.name))
        )
        self.gclient = self.chromium.root / ".gclient"
        self.module = GitSetupModule()

    def test_missing_gclient_is_tolerated(self):
        self.gclient.unlink()

        self.module._ensure_gclient_target_cpus(self.ctx, ["x64", "arm64"])

        self.assertFalse(self.gclient.exists())

    def test_appends_target_cpus_when_absent(self):
        self.module._ensure_gclient_target_cpus(self.ctx, ["x64", "arm64"])

        content = self.gclient.read_text()
        self.assertIn("target_cpus = ['x64', 'arm64']", content)
        self.assertIn("solutions = [", content)

    def test_merges_missing_archs_into_existing_list(self):
        self.gclient.write_text(self.gclient.read_text() + "\ntarget_cpus = ['x64']\n")

        self.module._ensure_gclient_target_cpus(self.ctx, ["x64", "arm64"])

        content = self.gclient.read_text()
        self.assertIn("target_cpus = ['arm64', 'x64']", content)
        self.assertNotIn("target_cpus = ['x64']\n", content)

    def test_complete_list_leaves_file_unchanged(self):
        self.gclient.write_text(
            self.gclient.read_text() + "\ntarget_cpus = ['arm64', 'x64']\n"
        )
        before = self.gclient.read_text()

        self.module._ensure_gclient_target_cpus(self.ctx, ["x64", "arm64"])

        self.assertEqual(self.gclient.read_text(), before)


if __name__ == "__main__":
    unittest.main()
