#!/usr/bin/env python3
"""Tests for the git setup module's gclient handling against a mock checkout."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from .git import GitSetupModule, BROWSEROS_BRANCH
from ..source.provision import ensure_gclient_config
from ...core.context import Context
from ...core.step import ValidationError
from ...lib.testing import MockBrowserOSRoot, MockChromium, make_context


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
        with mock.patch("bos_build.steps.setup.git.ensure") as prepare:
            GitSetupModule().execute(self.ctx)
        prepare.assert_called_once_with(
            self.ctx.chromium_src.parent,
            self.ctx.chromium_version,
            strategy="full",
            branch=BROWSEROS_BRANCH,
        )

    def test_missing_tag_stops_before_checkout(self):
        with mock.patch(
            "bos_build.steps.setup.git.ensure", side_effect=ValidationError("missing")
        ):
            with self.assertRaises(ValidationError):
                GitSetupModule().execute(self.ctx)


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

    def test_missing_gclient_is_bootstrapped(self):
        self.gclient.unlink()

        ensure_gclient_config(self.ctx.chromium_src.parent, ("x64", "arm64"))

        self.assertTrue(self.gclient.exists())

    def test_appends_target_cpus_when_absent(self):
        ensure_gclient_config(self.ctx.chromium_src.parent, ("x64", "arm64"))

        content = self.gclient.read_text()
        self.assertIn("target_cpus = ['x64', 'arm64']", content)
        self.assertIn("solutions = [", content)

    def test_merges_missing_archs_into_existing_list(self):
        self.gclient.write_text(self.gclient.read_text() + "\ntarget_cpus = ['x64']\n")

        ensure_gclient_config(self.ctx.chromium_src.parent, ("x64", "arm64"))

        content = self.gclient.read_text()
        self.assertIn("target_cpus = ['arm64', 'x64']", content)
        self.assertNotIn("target_cpus = ['x64']\n", content)

    def test_complete_list_leaves_file_unchanged(self):
        self.gclient.write_text(
            self.gclient.read_text() + "\ntarget_cpus = ['arm64', 'x64']\n"
        )
        before = self.gclient.read_text()

        ensure_gclient_config(self.ctx.chromium_src.parent, ("x64", "arm64"))

        self.assertEqual(self.gclient.read_text(), before)


if __name__ == "__main__":
    unittest.main()
