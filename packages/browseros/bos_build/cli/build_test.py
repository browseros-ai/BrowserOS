#!/usr/bin/env python3
"""CLI tests for --show-plan / --skip / --from and modules profiles.

Every invocation scrubs CHROMIUM_SRC/ARCH from the environment, so a
passing projection test proves the path never needs a chromium checkout.
"""

import multiprocessing
import os
import re
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from typer.testing import CliRunner

from bos_build.browseros import app
from bos_build.cli.build import _resolve_preset
from bos_build.core.checkout_lock import ChromiumCheckoutLock
from bos_build.core.planner import Switches, plan
from bos_build.lib.testing import MockChromium
from bos_build.lib.utils import get_platform, get_platform_arch

runner = CliRunner()
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def invoke(*args: str):
    return runner.invoke(app, ["build", *args])


def combined(result) -> str:
    """stdout + stderr across click versions (8.2 split them)."""
    out = result.output
    try:
        out += result.stderr
    except (ValueError, AttributeError):
        pass
    return out


def plain_output(result) -> str:
    return ANSI_RE.sub("", combined(result))


def scrubbed_env(*extra: str):
    drop = {"CHROMIUM_SRC", "ARCH", *extra}
    clean = {k: v for k, v in os.environ.items() if k not in drop}
    return mock.patch.dict(os.environ, clean, clear=True)


def _hold_checkout_lock(chromium_src: str, ready, release) -> None:
    with ChromiumCheckoutLock(
        Path(chromium_src),
        product="browserclaw",
        command=("test-holder",),
    ):
        ready.set()
        release.wait(15)


def plan_lines(output: str):
    """Parse the numbered step lines back into a step list."""
    steps = []
    for line in output.splitlines():
        head, sep, tail = line.strip().partition(". ")
        if sep and head.isdigit():
            steps.append(tail)
    return steps


class _ProfileMixin(unittest.TestCase):
    def _profile(self, text: str) -> Path:
        with tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False) as f:
            f.write(text)
            path = Path(f.name)
        self.addCleanup(path.unlink)
        return path


class ShowPlanPresetTest(_ProfileMixin):
    def test_exits_zero_without_chromium(self):
        with scrubbed_env():
            result = invoke("--preset", "release", "--show-plan")
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("compile", plan_lines(result.output))
        self.assertIn("Required env", result.output)

    def test_matches_planner_output(self):
        with scrubbed_env():
            result = invoke("--preset", "release", "--arch", "x64", "--show-plan")
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertEqual(
            plan_lines(result.output),
            plan(Switches(preset="release"), "x64", get_platform()),
        )

    def test_skip_reflected(self):
        with scrubbed_env():
            result = invoke("--preset", "release", "--skip", "upload", "--show-plan")
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertNotIn("upload", plan_lines(result.output))

    def test_from_reflected(self):
        with scrubbed_env():
            result = invoke("--preset", "release", "--from", "configure", "--show-plan")
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertEqual(plan_lines(result.output)[0], "configure")

    def test_multi_arch_profile_prints_block_per_arch(self):
        path = self._profile("preset: release\narch: [x64, arm64]\n")
        with scrubbed_env():
            result = invoke("--profile", str(path), "--show-plan")
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("x64 (", result.output)
        self.assertIn("arm64 (", result.output)

    def test_bundle_local_extensions_switch_reflected(self):
        path = self._profile("preset: release\nbundle_local_extensions: true\n")
        with scrubbed_env():
            result = invoke("--profile", str(path), "--show-plan")
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("bundle_local_extensions=True", result.output)

    def test_profile_skip_unions_with_cli_skip(self):
        path = self._profile("preset: release\nskip: [upload]\n")
        with scrubbed_env():
            result = invoke(
                "--profile", str(path), "--skip", "series_patches", "--show-plan"
            )
        self.assertEqual(result.exit_code, 0, combined(result))
        steps = plan_lines(result.output)
        self.assertNotIn("upload", steps)
        self.assertNotIn("series_patches", steps)

    def test_unknown_skip_name_fails_listing_valid(self):
        with scrubbed_env():
            result = invoke("--preset", "release", "--skip", "uplod", "--show-plan")
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("Valid steps", combined(result))

    def test_from_step_not_in_plan_fails(self):
        # debug plans never contain series_patches
        with scrubbed_env():
            result = invoke(
                "--preset", "debug", "--from", "series_patches", "--show-plan"
            )
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("not in the composed plan", combined(result))

    def test_env_markers_never_show_values(self):
        signing_env = (
            "MACOS_CERTIFICATE_NAME",
            "PROD_MACOS_NOTARIZATION_APPLE_ID",
            "PROD_MACOS_NOTARIZATION_TEAM_ID",
            "PROD_MACOS_NOTARIZATION_PWD",
        )
        with scrubbed_env(*signing_env):
            os.environ["MACOS_CERTIFICATE_NAME"] = "super-secret-cert"
            result = invoke("--modules", "sign_macos", "--show-plan")
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("MACOS_CERTIFICATE_NAME  ✓ set", result.output)
        self.assertIn("✗ MISSING", result.output)
        self.assertNotIn("super-secret-cert", result.output)


class EmptyPlanTest(unittest.TestCase):
    def _host_debug_plan(self):
        return plan(Switches(preset="debug"), get_platform_arch(), get_platform())

    def test_all_steps_skipped_fails_before_chromium(self):
        with scrubbed_env():
            result = invoke(
                "--preset", "debug", "--skip", ",".join(self._host_debug_plan())
            )
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("empty", combined(result).lower())

    def test_show_plan_displays_zero_steps(self):
        with scrubbed_env():
            result = invoke(
                "--preset",
                "debug",
                "--skip",
                ",".join(self._host_debug_plan()),
                "--show-plan",
            )
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("(0 steps)", result.output)


class ModeGuardTest(unittest.TestCase):
    def test_skip_requires_preset_mode(self):
        with scrubbed_env():
            result = invoke("--modules", "clean,compile", "--skip", "clean")
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("preset", combined(result).lower())

    def test_from_requires_preset_mode(self):
        with scrubbed_env():
            result = invoke("--build", "--from", "compile")
        self.assertNotEqual(result.exit_code, 0)

    def test_show_plan_without_mode_keeps_mode_error(self):
        with scrubbed_env():
            result = invoke("--show-plan")
        self.assertNotEqual(result.exit_code, 0)

    def test_direct_modules_show_plan(self):
        with scrubbed_env():
            result = invoke("--modules", "clean,compile", "--show-plan")
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertEqual(plan_lines(result.output), ["clean", "compile"])
        self.assertNotIn("DIRECT MODE", result.output)

    def test_direct_unknown_module_fails_show_plan(self):
        with scrubbed_env():
            result = invoke("--modules", "clean,nonsense", "--show-plan")
        self.assertNotEqual(result.exit_code, 0)

    def test_direct_invalid_arch_fails_show_plan(self):
        with scrubbed_env():
            result = invoke("--modules", "clean", "--arch", "bogus", "--show-plan")
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("Invalid architecture", combined(result))


class CheckoutLockCliTest(unittest.TestCase):
    def test_build_fails_fast_when_checkout_is_locked(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            ctx = multiprocessing.get_context("spawn")
            ready = ctx.Event()
            release = ctx.Event()
            proc = ctx.Process(
                target=_hold_checkout_lock,
                args=(str(m.src), ready, release),
            )
            proc.start()
            try:
                self.assertTrue(ready.wait(5), f"lock holder exited: {proc.exitcode}")
                with scrubbed_env():
                    result = invoke("--modules", "clean", "--chromium-src", str(m.src))
                self.assertNotEqual(result.exit_code, 0)
                output = plain_output(result)
                self.assertIn("Chromium checkout is already locked", output)
                self.assertIn("product=browserclaw", output)
                self.assertIn("--lock-wait", output)
            finally:
                release.set()
                proc.join(5)
                if proc.is_alive():
                    proc.kill()
                    proc.join(5)
            self.assertEqual(proc.exitcode, 0)


class ModulesProfileCliTest(_ProfileMixin):
    def test_show_plan_prints_enumerated_list(self):
        path = self._profile("modules: [clean, compile]\nbuild_type: release\n")
        with scrubbed_env():
            result = invoke("--profile", str(path), "--show-plan")
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertEqual(plan_lines(result.output), ["clean", "compile"])
        self.assertIn("you own this list", result.output)

    def test_planner_flags_rejected(self):
        path = self._profile("modules: [clean]\n")
        for flags in (
            ("--preset", "release"),
            ("--sign",),
            ("--no-upload",),
            ("--skip", "clean"),
            ("--from", "clean"),
        ):
            with scrubbed_env():
                result = invoke("--profile", str(path), *flags, "--show-plan")
            self.assertNotEqual(result.exit_code, 0, flags)
            self.assertIn("modules", combined(result))

    def test_build_type_override_allowed(self):
        path = self._profile("modules: [clean]\nbuild_type: release\n")
        with scrubbed_env():
            result = invoke(
                "--profile", str(path), "--build-type", "debug", "--show-plan"
            )
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("build_type=debug", result.output)

    def test_invalid_build_type_override_rejected(self):
        path = self._profile("modules: [clean]\n")
        with scrubbed_env():
            result = invoke(
                "--profile", str(path), "--build-type", "fast", "--show-plan"
            )
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("Invalid build type", combined(result))

    def test_arch_label_honors_env_like_the_run_will(self):
        path = self._profile("modules: [clean]\n")
        with scrubbed_env():
            os.environ["ARCH"] = "x64"
            result = invoke("--profile", str(path), "--show-plan")
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("arch=x64", result.output)
        self.assertIn("x64 (", result.output)

    def test_invalid_profile_arch_fails_show_plan(self):
        path = self._profile("modules: [clean]\narch: bogus\n")
        with scrubbed_env():
            result = invoke("--profile", str(path), "--show-plan")
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("Invalid architecture", combined(result))

    def test_unknown_module_in_profile_fails(self):
        path = self._profile("modules: [clean, nonsense]\n")
        with scrubbed_env():
            result = invoke("--profile", str(path), "--show-plan")
        self.assertNotEqual(result.exit_code, 0)

    def test_build_type_rejected_for_switch_profiles(self):
        path = self._profile("preset: release\n")
        with scrubbed_env():
            result = invoke(
                "--profile", str(path), "--build-type", "release", "--show-plan"
            )
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("owned by the preset", combined(result))


class GnArgOptionTest(_ProfileMixin):
    def test_malformed_gn_arg_rejected(self):
        with scrubbed_env():
            result = invoke("--preset", "debug", "--gn-arg", "bogus", "--show-plan")
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("bogus", combined(result))
        self.assertIn("key=value", combined(result))

    def test_empty_gn_arg_value_rejected(self):
        with scrubbed_env():
            result = invoke(
                "--preset", "debug", "--gn-arg", "symbol_level=", "--show-plan"
            )
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("symbol_level=", combined(result))

    def test_help_documents_repeatable(self):
        result = invoke("--help")
        self.assertEqual(result.exit_code, 0, combined(result))
        help_text = plain_output(result)
        self.assertIn("--gn-arg", help_text)
        self.assertIn("repeatable", help_text)

    def test_preset_show_plan_lists_overrides(self):
        with scrubbed_env():
            result = invoke(
                "--preset",
                "release",
                "--gn-arg",
                "symbol_level=2",
                "--gn-arg",
                "dcheck_always_on=true",
                "--show-plan",
            )
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn(
            "GN arg overrides: symbol_level=2, dcheck_always_on=true", result.output
        )

    def test_direct_show_plan_lists_overrides(self):
        with scrubbed_env():
            result = invoke(
                "--modules",
                "clean,compile",
                "--gn-arg",
                "symbol_level=2",
                "--show-plan",
            )
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("GN arg overrides: symbol_level=2", result.output)

    def test_modules_profile_show_plan_lists_overrides(self):
        path = self._profile("modules: [clean]\n")
        with scrubbed_env():
            result = invoke(
                "--profile", str(path), "--gn-arg", "symbol_level=2", "--show-plan"
            )
        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("GN arg overrides: symbol_level=2", result.output)


class GnArgPlumbingTest(_ProfileMixin):
    """--gn-arg must reach every Context the projections construct."""

    def _preset_kwargs(self, **overrides):
        kwargs = dict(
            preset=None,
            profile=None,
            product=None,
            arch=None,
            clean=None,
            provision=None,
            download=None,
            sign=None,
            upload=None,
            build_type=None,
            skip=None,
            from_=None,
            chromium_src=None,
            extra_gn_args=("symbol_level=2",),
        )
        kwargs.update(overrides)
        return kwargs

    def test_preset_build_runs_carry_extra_gn_args(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            with scrubbed_env():
                projection = _resolve_preset(
                    **self._preset_kwargs(preset="debug", chromium_src=m.src)
                )
                runs = projection.build_runs()
        self.assertTrue(runs)
        for ctx, _steps in runs:
            self.assertEqual(ctx.extra_gn_args, ("symbol_level=2",))

    def test_preset_build_runs_carry_bundle_local_extensions(self):
        profile_path = self._profile("preset: release\nbundle_local_extensions: true\n")
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            with scrubbed_env():
                projection = _resolve_preset(
                    **self._preset_kwargs(profile=profile_path, chromium_src=m.src)
                )
                runs = projection.build_runs()
        self.assertTrue(runs)
        for ctx, _steps in runs:
            self.assertTrue(ctx.bundle_local_extensions)

    def test_modules_profile_build_runs_carry_extra_gn_args(self):
        profile_path = self._profile("modules: [clean]\n")
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            with scrubbed_env():
                projection = _resolve_preset(
                    **self._preset_kwargs(profile=profile_path, chromium_src=m.src)
                )
                runs = projection.build_runs()
        self.assertTrue(runs)
        for ctx, _steps in runs:
            self.assertEqual(ctx.extra_gn_args, ("symbol_level=2",))


if __name__ == "__main__":
    unittest.main()
