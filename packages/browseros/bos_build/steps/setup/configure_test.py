#!/usr/bin/env python3
"""Tests for the GN configure module against a mock checkout."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import configure
from ...core.context import Context
from ...core.step import ValidationError
from ...lib.testing import MockBrowserOSRoot, MockChromium, make_context
from ...lib.utils import get_platform


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
    def _execute(
        self,
        build_type: str,
        architecture: str = "x64",
        flags: str = "is_official_build = true\n",
        extra_gn_args: tuple = (),
    ):
        chromium_tmp = tempfile.TemporaryDirectory()
        root_tmp = tempfile.TemporaryDirectory()
        self.addCleanup(chromium_tmp.cleanup)
        self.addCleanup(root_tmp.cleanup)

        chromium = MockChromium(Path(chromium_tmp.name))
        root = MockBrowserOSRoot(Path(root_tmp.name))
        root.write_gn_flags(get_platform(), build_type, flags)
        ctx = make_context(
            chromium, root, architecture=architecture, build_type=build_type
        )
        ctx.extra_gn_args = extra_gn_args

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

    def test_debug_args_append_product_args_verbatim(self):
        ctx, chromium, _ = self._execute(
            "debug", architecture="arm64", flags="is_debug = true\n"
        )

        args_gn = (chromium.src / ctx.out_dir / "args.gn").read_text()
        self.assertEqual(
            args_gn,
            'is_debug = true\n\ntarget_cpu = "arm64"\n'
            + "\n".join(ctx.get_product_gn_args())
            + "\n",
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

    def test_extra_gn_args_appended_last(self):
        ctx, chromium, _ = self._execute(
            "release",
            architecture="arm64",
            extra_gn_args=("symbol_level=2", "dcheck_always_on=true"),
        )

        args_gn = (chromium.src / ctx.out_dir / "args.gn").read_text()
        self.assertEqual(
            args_gn,
            'is_official_build = true\n\ntarget_cpu = "arm64"\n'
            'browseros_product = "browseros"\n'
            "browseros_allow_runtime_product_override = false\n"
            "browseros_package_all_server_resources = false\n"
            "\n# --gn-arg overrides\n"
            "symbol_level=2\n"
            "dcheck_always_on=true\n",
        )

    def test_extra_gn_arg_overriding_flags_key_lands_after_it(self):
        ctx, chromium, _ = self._execute(
            "release",
            flags="is_official_build = true\nsymbol_level = 1\n",
            extra_gn_args=("symbol_level=2",),
        )

        args_gn = (chromium.src / ctx.out_dir / "args.gn").read_text()
        self.assertLess(
            args_gn.index("symbol_level = 1"), args_gn.index("symbol_level=2")
        )

    def test_debug_warns_when_server_resources_not_staged(self):
        with mock.patch.object(configure, "log_warning") as log_warning:
            self._execute("debug")

        messages = [call.args[0] for call in log_warning.call_args_list]
        self.assertTrue(any("server resources" in m for m in messages), messages)

    def test_release_does_not_warn_about_server_resources(self):
        with mock.patch.object(configure, "log_warning") as log_warning:
            self._execute("release")

        log_warning.assert_not_called()

    def test_extra_gn_args_logged(self):
        with mock.patch.object(configure, "log_info") as log_info:
            self._execute("debug", extra_gn_args=("symbol_level=2",))

        messages = [call.args[0] for call in log_info.call_args_list]
        self.assertTrue(
            any("Applying 1 gn-arg override(s): symbol_level=2" in m for m in messages),
            messages,
        )


if __name__ == "__main__":
    unittest.main()
