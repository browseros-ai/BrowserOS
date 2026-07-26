#!/usr/bin/env python3
"""Tests for memory-aware ninja parallelism in the compile module."""

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
from typing import cast
from unittest import mock

from . import standard
from ...core.context import ArtifactRegistry, Context


class ComputeNinjaJobsTest(unittest.TestCase):
    def test_env_override_wins_on_any_platform(self):
        with mock.patch.object(standard, "IS_WINDOWS", return_value=False):
            jobs = standard.compute_ninja_jobs({"BROWSEROS_NINJA_JOBS": "24"})
        self.assertEqual(jobs, 24)

    def test_invalid_override_is_ignored(self):
        for bad in ("abc", "0", "-3", ""):
            with mock.patch.object(standard, "IS_WINDOWS", return_value=False):
                jobs = standard.compute_ninja_jobs({"BROWSEROS_NINJA_JOBS": bad})
            self.assertIsNone(jobs, f"override {bad!r} should be ignored")

    def test_non_windows_without_override_keeps_default(self):
        with mock.patch.object(standard, "IS_WINDOWS", return_value=False):
            self.assertIsNone(standard.compute_ninja_jobs({}))

    def test_valid_override_on_windows_skips_memory_probe(self):
        with (
            mock.patch.object(standard, "IS_WINDOWS", return_value=True),
            mock.patch.object(standard, "_windows_total_memory_gb") as probe,
        ):
            jobs = standard.compute_ninja_jobs({"BROWSEROS_NINJA_JOBS": "24"})
        self.assertEqual(jobs, 24)
        probe.assert_not_called()

    def test_invalid_override_on_windows_falls_through_to_memory_cap(self):
        with (
            mock.patch.object(standard, "IS_WINDOWS", return_value=True),
            mock.patch.object(standard, "_windows_total_memory_gb", return_value=64.0),
            mock.patch("os.cpu_count", return_value=32),
        ):
            jobs = standard.compute_ninja_jobs({"BROWSEROS_NINJA_JOBS": "abc"})
        self.assertEqual(jobs, 16)

    def test_windows_caps_jobs_by_physical_memory(self):
        with (
            mock.patch.object(standard, "IS_WINDOWS", return_value=True),
            mock.patch.object(standard, "_windows_total_memory_gb", return_value=64.0),
            mock.patch("os.cpu_count", return_value=32),
        ):
            self.assertEqual(standard.compute_ninja_jobs({}), 16)

    def test_windows_clamps_to_cpu_count(self):
        with (
            mock.patch.object(standard, "IS_WINDOWS", return_value=True),
            mock.patch.object(standard, "_windows_total_memory_gb", return_value=256.0),
            mock.patch("os.cpu_count", return_value=16),
        ):
            self.assertEqual(standard.compute_ninja_jobs({}), 16)

    def test_windows_never_returns_less_than_one_job(self):
        with (
            mock.patch.object(standard, "IS_WINDOWS", return_value=True),
            mock.patch.object(standard, "_windows_total_memory_gb", return_value=2.0),
            mock.patch("os.cpu_count", return_value=8),
        ):
            self.assertEqual(standard.compute_ninja_jobs({}), 1)

    def test_windows_memory_probe_failure_falls_back_to_default(self):
        with (
            mock.patch.object(standard, "IS_WINDOWS", return_value=True),
            mock.patch.object(standard, "_windows_total_memory_gb", return_value=None),
            mock.patch("os.cpu_count", return_value=32),
        ):
            self.assertIsNone(standard.compute_ninja_jobs({}))

    def test_windows_unknown_cpu_count_uses_memory_value(self):
        with (
            mock.patch.object(standard, "IS_WINDOWS", return_value=True),
            mock.patch.object(standard, "_windows_total_memory_gb", return_value=64.0),
            mock.patch("os.cpu_count", return_value=None),
        ):
            self.assertEqual(standard.compute_ninja_jobs({}), 16)


class AutoninjaCommandTest(unittest.TestCase):
    def test_override_inserts_jobs_flag_before_targets(self):
        with mock.patch.object(standard, "IS_WINDOWS", return_value=False):
            cmd = standard.autoninja_command(
                "out/Default_x64",
                ["chrome", "chromedriver"],
                {"BROWSEROS_NINJA_JOBS": "24"},
            )
        self.assertEqual(
            cmd,
            [
                "autoninja",
                "-C",
                "out/Default_x64",
                "-j",
                "24",
                "chrome",
                "chromedriver",
            ],
        )

    def test_default_parallelism_has_no_jobs_flag(self):
        with mock.patch.object(standard, "IS_WINDOWS", return_value=False):
            cmd = standard.autoninja_command("out/Default_arm64", ["chrome"], {})
        self.assertEqual(cmd, ["autoninja", "-C", "out/Default_arm64", "chrome"])

    def test_windows_uses_bat_and_memory_capped_jobs(self):
        with (
            mock.patch.object(standard, "IS_WINDOWS", return_value=True),
            mock.patch.object(standard, "_windows_total_memory_gb", return_value=64.0),
            mock.patch("os.cpu_count", return_value=32),
        ):
            cmd = standard.autoninja_command("out/Default_x64", ["chrome"], {})
        self.assertEqual(
            cmd,
            ["autoninja.bat", "-C", "out/Default_x64", "-j", "16", "chrome"],
        )


class CompileModuleExecuteTest(unittest.TestCase):
    def test_execute_builds_chrome_via_shared_argv_builder(self):
        ctx = mock.Mock()
        ctx.out_dir = "out/Default_x64"
        ctx.chromium_src = Path("/tmp/chromium-src")
        with (
            mock.patch.object(standard, "run_command") as run_cmd,
            mock.patch.object(standard, "IS_WINDOWS", return_value=False),
            mock.patch.object(standard.CompileModule, "_create_version_file"),
            mock.patch.dict("os.environ", {}, clear=True),
        ):
            standard.CompileModule().execute(ctx)
        run_cmd.assert_called_once()
        self.assertEqual(
            run_cmd.call_args.args[0],
            ["autoninja", "-C", "out/Default_x64", "chrome", "chromedriver"],
        )
        self.assertEqual(run_cmd.call_args.kwargs["cwd"], ctx.chromium_src)

    def test_windows_keeps_and_registers_chrome_exe(self):
        with TemporaryDirectory() as tmp:
            build_output_dir = Path(tmp) / "out" / "Default"
            build_output_dir.mkdir(parents=True)
            chrome_path = build_output_dir / "chrome.exe"
            product_path = build_output_dir / "BrowserOS.exe"
            chrome_path.write_bytes(b"chrome")
            registry = ArtifactRegistry()
            ctx = cast(
                Context,
                SimpleNamespace(
                    out_dir="out/Default",
                    chromium_src=Path(tmp),
                    artifact_registry=registry,
                    get_chromium_app_path=lambda: chrome_path,
                    get_app_path=lambda: product_path,
                ),
            )

            with (
                mock.patch.object(standard, "IS_WINDOWS", return_value=True),
                mock.patch.object(standard, "run_command"),
                mock.patch.object(standard, "autoninja_command", return_value=[]),
                mock.patch.object(standard.CompileModule, "_create_version_file"),
            ):
                standard.CompileModule().execute(ctx)

            self.assertTrue(chrome_path.exists())
            self.assertFalse(product_path.exists())
            self.assertEqual(registry.get("built_app"), chrome_path)

    def test_non_windows_keeps_product_rename(self):
        with TemporaryDirectory() as tmp:
            build_output_dir = Path(tmp) / "out" / "Default"
            build_output_dir.mkdir(parents=True)
            chrome_path = build_output_dir / "chrome"
            product_path = build_output_dir / "browseros"
            chrome_path.write_bytes(b"chrome")
            registry = ArtifactRegistry()
            ctx = cast(
                Context,
                SimpleNamespace(
                    out_dir="out/Default",
                    chromium_src=Path(tmp),
                    artifact_registry=registry,
                    get_chromium_app_path=lambda: chrome_path,
                    get_app_path=lambda: product_path,
                ),
            )

            with (
                mock.patch.object(standard, "IS_WINDOWS", return_value=False),
                mock.patch.object(standard, "run_command"),
                mock.patch.object(standard, "autoninja_command", return_value=[]),
                mock.patch.object(standard.CompileModule, "_create_version_file"),
            ):
                standard.CompileModule().execute(ctx)

            self.assertFalse(chrome_path.exists())
            self.assertEqual(product_path.read_bytes(), b"chrome")
            self.assertEqual(registry.get("built_app"), product_path)


class BuildTargetTest(unittest.TestCase):
    def test_build_target_uses_shared_argv_builder(self):
        ctx = cast(
            Context,
            SimpleNamespace(
                out_dir="out/Default_x64", chromium_src=Path("/tmp/chromium-src")
            ),
        )
        with (
            mock.patch.object(standard, "run_command") as run_cmd,
            mock.patch.object(standard, "IS_WINDOWS", return_value=False),
            mock.patch.dict("os.environ", {}, clear=True),
        ):
            standard.build_target(ctx, "mini_installer")
        run_cmd.assert_called_once()
        self.assertEqual(
            run_cmd.call_args.args[0],
            ["autoninja", "-C", "out/Default_x64", "mini_installer"],
        )
        self.assertEqual(run_cmd.call_args.kwargs["cwd"], ctx.chromium_src)

    def test_build_target_respects_jobs_override(self):
        ctx = cast(
            Context,
            SimpleNamespace(
                out_dir="out/Default_x64", chromium_src=Path("/tmp/chromium-src")
            ),
        )
        with (
            mock.patch.object(standard, "run_command") as run_cmd,
            mock.patch.object(standard, "IS_WINDOWS", return_value=False),
            mock.patch.dict("os.environ", {"BROWSEROS_NINJA_JOBS": "8"}, clear=True),
        ):
            standard.build_target(ctx, "mini_installer")
        self.assertEqual(
            run_cmd.call_args.args[0],
            ["autoninja", "-C", "out/Default_x64", "-j", "8", "mini_installer"],
        )


if __name__ == "__main__":
    unittest.main()
