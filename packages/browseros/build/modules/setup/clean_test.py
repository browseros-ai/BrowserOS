#!/usr/bin/env python3
"""Tests for the clean module against a mock checkout."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import clean
from ...common.context import Context
from ...common.module import ValidationError
from ...common.testing import MockBrowserOSRoot, MockChromium, make_context


class CleanValidateTest(unittest.TestCase):
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
                clean.CleanModule().validate(ctx)


class CleanExecuteTest(unittest.TestCase):
    def test_removes_out_dir_and_sparkle_and_resets_git(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            chromium = MockChromium(Path(chromium_tmp))
            ctx = make_context(
                chromium, MockBrowserOSRoot(Path(root_tmp)), architecture="x64"
            )
            out_dir = chromium.with_out_dir("x64", args_gn="is_debug = false\n")
            sparkle = chromium.with_sparkle()
            winsparkle = chromium.with_winsparkle()

            with mock.patch.object(clean, "run_command") as run_cmd:
                clean.CleanModule().execute(ctx)

            self.assertFalse(out_dir.exists())
            self.assertFalse(sparkle.exists())
            self.assertFalse(winsparkle.exists())

            git_commands = [call.args[0] for call in run_cmd.call_args_list]
            self.assertEqual(
                git_commands[0], ["git", "reset", "--hard", "HEAD"]
            )
            self.assertTrue(
                all(cmd[0] == "git" for cmd in git_commands),
                f"expected only git commands, got: {git_commands}",
            )
            for call in run_cmd.call_args_list:
                self.assertEqual(call.kwargs["cwd"], ctx.chromium_src)

    def test_missing_out_dir_is_tolerated(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            chromium = MockChromium(Path(chromium_tmp))
            ctx = make_context(chromium, MockBrowserOSRoot(Path(root_tmp)))

            with mock.patch.object(clean, "run_command"):
                clean.CleanModule().execute(ctx)


if __name__ == "__main__":
    unittest.main()
