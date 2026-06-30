#!/usr/bin/env python3
"""Tests for the GN configure module against a mock checkout."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import configure
from ...common.context import Context
from ...common.module import ValidationError
from ...common.testing import MockBrowserOSRoot, MockChromium, make_context
from ...common.utils import get_platform


class ConfigureValidateTest(unittest.TestCase):
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
                configure.ConfigureModule().validate(ctx)

    def test_missing_gn_flags_file_raises(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            ctx = make_context(
                MockChromium(Path(chromium_tmp)),
                MockBrowserOSRoot(Path(root_tmp)),
                build_type="release",
            )
            with self.assertRaises(ValidationError):
                configure.ConfigureModule().validate(ctx)

    def test_passes_with_existing_flags_file(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            root = MockBrowserOSRoot(Path(root_tmp))
            root.write_gn_flags(get_platform(), "release", "is_debug = false\n")
            ctx = make_context(
                MockChromium(Path(chromium_tmp)), root, build_type="release"
            )
            configure.ConfigureModule().validate(ctx)


class ConfigureExecuteTest(unittest.TestCase):
    def _execute(self, build_type: str, architecture: str = "x64"):
        chromium_tmp = tempfile.TemporaryDirectory()
        root_tmp = tempfile.TemporaryDirectory()
        self.addCleanup(chromium_tmp.cleanup)
        self.addCleanup(root_tmp.cleanup)

        chromium = MockChromium(Path(chromium_tmp.name))
        root = MockBrowserOSRoot(Path(root_tmp.name))
        root.write_gn_flags(get_platform(), build_type, "is_official_build = true\n")
        ctx = make_context(
            chromium, root, architecture=architecture, build_type=build_type
        )

        with (
            mock.patch.object(configure, "run_command") as run_cmd,
            mock.patch.object(configure, "IS_LINUX", return_value=False),
            mock.patch.object(configure, "IS_WINDOWS", return_value=False),
        ):
            configure.ConfigureModule().execute(ctx)

        return ctx, chromium, run_cmd

    def test_writes_args_gn_with_target_cpu(self):
        ctx, chromium, _ = self._execute("release", architecture="arm64")

        args_gn = chromium.src / ctx.out_dir / "args.gn"
        self.assertTrue(args_gn.exists())
        self.assertEqual(
            args_gn.read_text(),
            'is_official_build = true\n\ntarget_cpu = "arm64"\n'
            'browseros_product = "browseros"\n'
            "browseros_allow_runtime_product_override = false\n"
            "browseros_package_all_server_resources = false\n",
        )

    def test_release_build_fails_on_unused_args(self):
        ctx, _, run_cmd = self._execute("release")

        run_cmd.assert_called_once()
        self.assertEqual(
            run_cmd.call_args.args[0],
            ["gn", "gen", ctx.out_dir, "--fail-on-unused-args"],
        )
        self.assertEqual(run_cmd.call_args.kwargs["cwd"], ctx.chromium_src)

    def test_debug_build_omits_fail_on_unused_args(self):
        ctx, _, run_cmd = self._execute("debug")

        self.assertEqual(run_cmd.call_args.args[0], ["gn", "gen", ctx.out_dir])


if __name__ == "__main__":
    unittest.main()
