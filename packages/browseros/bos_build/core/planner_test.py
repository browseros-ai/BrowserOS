#!/usr/bin/env python3
"""Golden tests: plan() must reproduce the module lists the deleted
config/release.*.yaml files encoded (source file named per case)."""

import tempfile
import unittest
from pathlib import Path

from bos_build.core.planner import (
    Profile,
    Switches,
    load_profile,
    plan,
    plan_runs,
    required_env,
    slice_from,
    slice_runs_from,
)

RELEASE = Switches(preset="release")
CI = Switches(preset="release", clean=False, provision="none", sign=False, upload=False)
UNIVERSAL = Switches(preset="release", architectures=("universal",))


class ReleaseGoldenTest(unittest.TestCase):
    def test_macos_arm64_signed(self):
        # release.browseros.macos.arm64.yaml / release.macos.arm64.yaml
        self.assertEqual(
            plan(RELEASE, "arm64", "macos"),
            [
                "clean",
                "git_setup",
                "sparkle_setup",
                "download_resources",
                "resources",
                "bundled_extensions",
                "chromium_replace",
                "string_replaces",
                "series_patches",
                "patches",
                "configure",
                "compile",
                "sign_macos",
                "package_macos",
                "sparkle_sign",
                "upload",
            ],
        )

    def test_windows_signed(self):
        # release.browseros.windows.yaml — note sparkle_sign AFTER package
        self.assertEqual(
            plan(RELEASE, "x64", "windows"),
            [
                "clean",
                "git_setup",
                "winsparkle_setup",
                "download_resources",
                "resources",
                "bundled_extensions",
                "chromium_replace",
                "string_replaces",
                "series_patches",
                "patches",
                "configure",
                "compile",
                "sign_windows",
                "package_windows",
                "sparkle_sign",
                "upload",
            ],
        )

    def test_linux_never_plans_sign(self):
        # release.browseros.linux.yaml
        steps = plan(RELEASE, "x64", "linux")
        self.assertEqual(
            steps,
            [
                "clean",
                "git_setup",
                "download_resources",
                "resources",
                "bundled_extensions",
                "chromium_replace",
                "string_replaces",
                "series_patches",
                "patches",
                "configure",
                "compile",
                "package_linux",
                "upload",
            ],
        )
        self.assertFalse(any(s.startswith("sign") for s in steps))

    def test_macos_universal(self):
        # release.browseros.macos.universal.yaml — three sequential runs on
        # one prepped tree: prep exactly once (repeating clean/git_setup/
        # patches would reset it), resources per arch, then the merge
        self.assertEqual(
            plan_runs(UNIVERSAL, "macos"),
            [
                (
                    "arm64",
                    [
                        "clean",
                        "git_setup",
                        "sparkle_setup",
                        "download_resources",
                        "bundled_extensions",
                        "chromium_replace",
                        "string_replaces",
                        "series_patches",
                        "patches",
                        "resources",
                        "configure",
                        "compile",
                        "sign_macos",
                        "package_macos",
                        "sparkle_sign",
                        "upload",
                    ],
                ),
                (
                    "x64",
                    [
                        "resources",
                        "configure",
                        "compile",
                        "sign_macos",
                        "package_macos",
                        "sparkle_sign",
                        "upload",
                    ],
                ),
                (
                    "universal",
                    [
                        "merge_universal",
                        "sign_macos",
                        "package_macos",
                        "sparkle_sign",
                        "upload",
                    ],
                ),
            ],
        )

    def test_universal_rejected_off_macos(self):
        with self.assertRaisesRegex(ValueError, "only supported on macos"):
            plan_runs(UNIVERSAL, "linux")

    def test_noupload_variant(self):
        # release.macos.arm64.noupload.yaml == release minus upload;
        # signed artifacts are still Sparkle-signed like Windows.
        steps = plan(Switches(preset="release", upload=False), "arm64", "macos")
        self.assertEqual(steps[-2:], ["package_macos", "sparkle_sign"])
        self.assertNotIn("upload", steps)


class CiGoldenTest(unittest.TestCase):
    def test_macos_ci_keeps_sparkle_setup_unsigned(self):
        # release.macos.arm64.ci.yaml
        self.assertEqual(
            plan(CI, "arm64", "macos"),
            [
                "sparkle_setup",
                "download_resources",
                "resources",
                "bundled_extensions",
                "chromium_replace",
                "string_replaces",
                "series_patches",
                "patches",
                "configure",
                "compile",
                "package_macos",
            ],
        )

    def test_windows_ci_swaps_sign_for_mini_installer(self):
        # release.windows.ci.yaml — no sparkle_sign; winsparkle_setup stays
        # (the release GN config links WinSparkle.dll even unsigned)
        self.assertEqual(
            plan(CI, "x64", "windows"),
            [
                "winsparkle_setup",
                "download_resources",
                "resources",
                "bundled_extensions",
                "chromium_replace",
                "string_replaces",
                "series_patches",
                "patches",
                "configure",
                "compile",
                "mini_installer",
                "package_windows",
            ],
        )

    def test_linux_ci(self):
        # release.linux.ci.yaml
        self.assertEqual(
            plan(CI, "x64", "linux"),
            [
                "download_resources",
                "resources",
                "bundled_extensions",
                "chromium_replace",
                "string_replaces",
                "series_patches",
                "patches",
                "configure",
                "compile",
                "package_linux",
            ],
        )


class DebugGoldenTest(unittest.TestCase):
    def test_debug_macos(self):
        # config/debug.yaml — no clean, no series_patches, no sparkle_setup,
        # no sign, no upload
        self.assertEqual(
            plan(Switches(preset="debug"), "arm64", "macos"),
            [
                "git_setup",
                "download_resources",
                "resources",
                "bundled_extensions",
                "chromium_replace",
                "string_replaces",
                "patches",
                "configure",
                "compile",
                "package_macos",
            ],
        )

    def test_debug_rejects_universal(self):
        with self.assertRaisesRegex(ValueError, "not supported for debug"):
            plan_runs(Switches(preset="debug", architectures=("universal",)), "macos")

    def test_debug_windows_builds_installer_before_packaging(self):
        self.assertEqual(
            plan(Switches(preset="debug"), "x64", "windows"),
            [
                "git_setup",
                "winsparkle_setup",
                "download_resources",
                "resources",
                "bundled_extensions",
                "chromium_replace",
                "string_replaces",
                "patches",
                "configure",
                "compile",
                "mini_installer",
                "package_windows",
            ],
        )

    def test_debug_linux(self):
        self.assertEqual(
            plan(Switches(preset="debug"), "x64", "linux"),
            [
                "git_setup",
                "download_resources",
                "resources",
                "bundled_extensions",
                "chromium_replace",
                "string_replaces",
                "patches",
                "configure",
                "compile",
                "package_linux",
            ],
        )

    def test_debug_rejection_wins_over_platform(self):
        with self.assertRaisesRegex(ValueError, "not supported for debug"):
            plan_runs(Switches(preset="debug", architectures=("universal",)), "linux")


class SwitchesTest(unittest.TestCase):
    def test_unknown_preset_rejected(self):
        with self.assertRaisesRegex(ValueError, "Unknown preset"):
            Switches(preset="nightly").resolved()

    def test_invalid_arch_rejected(self):
        with self.assertRaisesRegex(ValueError, "Invalid architecture"):
            Switches(architectures=("mips",)).resolved()

    def test_invalid_provision_rejected(self):
        with self.assertRaisesRegex(ValueError, "Invalid provision"):
            Switches(provision="warp").resolved()

    def test_multi_arch_plans_per_arch(self):
        sw = Switches(preset="release", architectures=("x64", "arm64")).resolved()
        plans = [plan(sw, arch, "linux") for arch in sw.architectures]
        self.assertEqual(len(plans), 2)
        self.assertEqual(plans[0], plans[1])

    def test_build_type_follows_preset(self):
        self.assertEqual(Switches(preset="release").build_type, "release")
        self.assertEqual(Switches(preset="debug").build_type, "debug")

    def test_bundle_local_extensions_defaults_off(self):
        self.assertFalse(Switches().resolved().bundle_local_extensions)

    def test_bundle_local_extensions_does_not_change_step_order(self):
        local = Switches(preset="release", bundle_local_extensions=True)
        self.assertEqual(plan(local, "x64", "linux"), plan(RELEASE, "x64", "linux"))


class SkipTest(unittest.TestCase):
    def test_skip_subtracts_from_composed_plan(self):
        steps = plan(
            Switches(preset="release", skip=("upload", "series_patches")),
            "arm64",
            "macos",
        )
        self.assertEqual(
            steps,
            [
                s
                for s in plan(RELEASE, "arm64", "macos")
                if s not in ("upload", "series_patches")
            ],
        )

    def test_skip_is_subtraction_not_switch_flip(self):
        # Skipping sign_windows must not resurrect mini_installer or drop
        # the other signing-conditional steps — composition already ran.
        steps = plan(
            Switches(preset="release", skip=("sign_windows",)), "x64", "windows"
        )
        self.assertNotIn("sign_windows", steps)
        self.assertNotIn("mini_installer", steps)
        self.assertIn("winsparkle_setup", steps)
        self.assertIn("sparkle_sign", steps)

    def test_skip_applies_to_universal_runs(self):
        runs = plan_runs(
            Switches(
                preset="release",
                architectures=("universal",),
                skip=("series_patches",),
            ),
            "macos",
        )
        for arch, steps in runs:
            self.assertNotIn("series_patches", steps, arch)
        self.assertIn("merge_universal", runs[2][1])

    def test_skip_absent_step_is_noop(self):
        # mini_installer never appears in a signed windows plan; upload is
        # already absent when upload=False. Both subtract to nothing.
        self.assertEqual(
            plan(
                Switches(preset="release", skip=("mini_installer",)), "x64", "windows"
            ),
            plan(RELEASE, "x64", "windows"),
        )
        steps = plan(
            Switches(preset="release", upload=False, skip=("upload",)),
            "arm64",
            "macos",
        )
        self.assertNotIn("upload", steps)

    def test_skip_unknown_step_rejected_listing_valid(self):
        with self.assertRaises(ValueError) as err:
            plan(Switches(preset="release", skip=("uplod",)), "arm64", "macos")
        message = str(err.exception)
        self.assertIn("uplod", message)
        self.assertIn("compile", message)

    def test_skip_non_string_entry_rejected_not_typeerror(self):
        # yaml `skip: [upload: true]` parses to a dict entry
        with self.assertRaisesRegex(ValueError, "Unknown step"):
            plan(
                Switches(preset="release", skip=({"upload": True},)),
                "arm64",
                "macos",
            )


class SliceFromTest(unittest.TestCase):
    def test_slices_composed_plan_from_step(self):
        self.assertEqual(
            slice_from(plan(RELEASE, "arm64", "macos"), "sign_macos"),
            ["sign_macos", "package_macos", "sparkle_sign", "upload"],
        )

    def test_slice_from_first_step_is_identity(self):
        full = plan(RELEASE, "arm64", "macos")
        self.assertEqual(slice_from(full, full[0]), full)

    def test_unknown_step_rejected_listing_valid(self):
        with self.assertRaises(ValueError) as err:
            slice_from(plan(RELEASE, "arm64", "macos"), "sing_macos")
        message = str(err.exception)
        self.assertIn("sing_macos", message)
        self.assertIn("compile", message)

    def test_step_absent_from_plan_rejected_showing_plan(self):
        with self.assertRaises(ValueError) as err:
            slice_from(plan(RELEASE, "arm64", "macos"), "mini_installer")
        message = str(err.exception)
        self.assertIn("mini_installer", message)
        self.assertIn("sign_macos", message)

    def test_from_a_skipped_step_rejected(self):
        steps = plan(Switches(preset="release", skip=("sign_macos",)), "arm64", "macos")
        with self.assertRaisesRegex(ValueError, "not in the composed plan"):
            slice_from(steps, "sign_macos")


class SliceRunsFromTest(unittest.TestCase):
    def test_single_arch_run_sliced(self):
        runs = plan_runs(Switches(preset="release", architectures=("arm64",)), "macos")
        self.assertEqual(
            slice_runs_from(runs, "sign_macos"),
            [("arm64", ["sign_macos", "package_macos", "sparkle_sign", "upload"])],
        )

    def test_universal_merge_failure_resumes_without_recompiling(self):
        runs = slice_runs_from(plan_runs(UNIVERSAL, "macos"), "merge_universal")
        self.assertEqual(
            runs,
            [
                (
                    "universal",
                    [
                        "merge_universal",
                        "sign_macos",
                        "package_macos",
                        "sparkle_sign",
                        "upload",
                    ],
                )
            ],
        )

    def test_first_run_containing_step_wins_later_runs_stay_whole(self):
        full = plan_runs(UNIVERSAL, "macos")
        runs = slice_runs_from(full, "resources")
        self.assertEqual(runs[0][0], "arm64")
        self.assertEqual(
            runs[0][1],
            [
                "resources",
                "configure",
                "compile",
                "sign_macos",
                "package_macos",
                "sparkle_sign",
                "upload",
            ],
        )
        self.assertEqual(runs[1:], full[1:])

    def test_multi_arch_timeline(self):
        runs = plan_runs(
            Switches(preset="release", architectures=("x64", "arm64")), "linux"
        )
        sliced = slice_runs_from(runs, "compile")
        self.assertEqual(sliced[0][0], "x64")
        self.assertEqual(sliced[0][1][0], "compile")
        self.assertEqual(sliced[1], runs[1])

    def test_unknown_step_rejected_listing_valid(self):
        with self.assertRaisesRegex(ValueError, "Unknown step"):
            slice_runs_from(plan_runs(UNIVERSAL, "macos"), "sing_macos")

    def test_step_absent_from_all_runs_rejected(self):
        with self.assertRaisesRegex(ValueError, "not in the composed plan"):
            slice_runs_from(plan_runs(UNIVERSAL, "macos"), "mini_installer")


class RequiredEnvTest(unittest.TestCase):
    def test_signed_macos_requires_cert_notarization_and_sparkle_key(self):
        env = required_env(plan(RELEASE, "arm64", "macos"))
        self.assertEqual(
            env,
            [
                "MACOS_CERTIFICATE_NAME",
                "PROD_MACOS_NOTARIZATION_APPLE_ID",
                "PROD_MACOS_NOTARIZATION_TEAM_ID",
                "PROD_MACOS_NOTARIZATION_PWD",
                "SPARKLE_PRIVATE_KEY",
            ],
        )

    def test_signed_windows_requires_esigner_and_sparkle_key(self):
        # parity with release.*.windows.yaml required_envs
        env = required_env(plan(RELEASE, "x64", "windows"))
        self.assertEqual(
            env,
            [
                "CODE_SIGN_TOOL_PATH",
                "ESIGNER_USERNAME",
                "ESIGNER_PASSWORD",
                "ESIGNER_TOTP_SECRET",
                "SPARKLE_PRIVATE_KEY",
            ],
        )

    def test_unsigned_ci_requires_nothing(self):
        self.assertEqual(required_env(plan(CI, "x64", "windows")), [])
        self.assertEqual(required_env(plan(CI, "arm64", "macos")), [])


class ProfileTest(unittest.TestCase):
    def _load(self, text: str) -> Profile:
        with tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False) as f:
            f.write(text)
            path = Path(f.name)
        self.addCleanup(path.unlink)
        return load_profile(path)

    def test_nightly_ci_profile_maps_to_switches(self):
        prof = self._load(
            "preset: release\nclean: false\nprovision: none\nsign: false\nupload: false\n"
        )
        self.assertEqual(
            plan(prof.switches, "arm64", "macos"), plan(CI, "arm64", "macos")
        )

    def test_nightly_macos_profile_keeps_signed_defaults_without_downloads(self):
        profile_path = (
            Path(__file__).resolve().parents[1] / "profiles" / "nightly-macos.yaml"
        )
        switches = load_profile(profile_path).switches.resolved()

        self.assertTrue(switches.clean)
        self.assertEqual("full", switches.provision)
        self.assertFalse(switches.download)
        self.assertTrue(switches.sign)
        self.assertTrue(switches.upload)
        self.assertTrue(switches.bundle_local_extensions)
        self.assertNotIn("download_resources", plan(switches, "arm64", "macos"))

    def test_arch_list(self):
        prof = self._load("preset: release\narch: [x64, arm64]\n")
        self.assertEqual(prof.switches.architectures, ("x64", "arm64"))

    def test_unknown_key_rejected(self):
        with self.assertRaisesRegex(ValueError, "Unknown profile keys"):
            self._load("preset: release\nbogus: 1\n")

    def test_skip_key_parses_list_and_scalar(self):
        prof = self._load("preset: release\nskip: [upload, series_patches]\n")
        self.assertEqual(prof.switches.skip, ("upload", "series_patches"))
        self.assertEqual(self._load("skip: upload\n").switches.skip, ("upload",))

    def test_bundle_local_extensions_profile_key(self):
        prof = self._load("preset: release\nbundle_local_extensions: true\n")
        self.assertTrue(prof.switches.bundle_local_extensions)

    def test_flat_profile_has_no_modules(self):
        self.assertIsNone(self._load("preset: release\n").modules)

    def test_modules_profile_parses(self):
        prof = self._load(
            "modules: [clean, compile]\n"
            "product: browserclaw\n"
            "arch: arm64\n"
            "build_type: release\n"
        )
        self.assertEqual(prof.modules, ("clean", "compile"))
        self.assertEqual(prof.build_type, "release")
        self.assertEqual(prof.switches.product, "browserclaw")
        self.assertEqual(prof.switches.architectures, ("arm64",))

    def test_modules_profile_defaults(self):
        prof = self._load("modules: [compile]\n")
        self.assertEqual(prof.modules, ("compile",))
        self.assertIsNone(prof.build_type)
        self.assertEqual(prof.switches.architectures, ())

    def test_modules_rejects_planner_keys(self):
        for key, value in (
            ("preset", "release"),
            ("clean", "false"),
            ("provision", "none"),
            ("download", "false"),
            ("sign", "false"),
            ("upload", "false"),
            ("bundle_local_extensions", "true"),
            ("skip", "[upload]"),
        ):
            with self.assertRaisesRegex(ValueError, "do not combine", msg=key):
                self._load(f"modules: [compile]\n{key}: {value}\n")

    def test_build_type_requires_modules(self):
        with self.assertRaisesRegex(ValueError, "requires 'modules:'"):
            self._load("preset: release\nbuild_type: release\n")

    def test_invalid_build_type_rejected(self):
        with self.assertRaisesRegex(ValueError, "build_type"):
            self._load("modules: [compile]\nbuild_type: fast\n")

    def test_empty_modules_rejected(self):
        with self.assertRaisesRegex(ValueError, "non-empty"):
            self._load("modules: []\n")

    def test_non_string_modules_entry_rejected(self):
        # yaml `modules: [clean: true]` parses to a dict entry
        with self.assertRaisesRegex(ValueError, "list of step names"):
            self._load("modules: [{clean: true}]\n")

    def test_modules_arch_list_rejected(self):
        with self.assertRaisesRegex(ValueError, "single-arch"):
            self._load("modules: [compile]\narch: [x64, arm64]\n")


class PreflightTest(unittest.TestCase):
    def test_lists_all_missing_env_vars_not_first_only(self):
        import os
        from unittest import mock

        from bos_build.core.planner import preflight

        clean = {
            k: v
            for k, v in os.environ.items()
            if not k.startswith(("MACOS_", "PROD_MACOS_"))
        }
        with mock.patch.dict(os.environ, clean, clear=True):
            with self.assertRaises(ValueError) as err:
                preflight(plan(RELEASE, "arm64", "macos"), platform="macos")

        message = str(err.exception)
        self.assertIn("MACOS_CERTIFICATE_NAME", message)
        self.assertIn("PROD_MACOS_NOTARIZATION_APPLE_ID", message)
        self.assertIn("PROD_MACOS_NOTARIZATION_TEAM_ID", message)
        self.assertIn("PROD_MACOS_NOTARIZATION_PWD", message)

    def test_platform_mismatch_rejected(self):
        from bos_build.core.planner import preflight

        with self.assertRaisesRegex(ValueError, "does not apply to platform 'linux'"):
            preflight(["clean", "sign_macos"], platform="linux")

    def test_unknown_step_rejected(self):
        from bos_build.core.planner import preflight

        with self.assertRaisesRegex(ValueError, "unknown step: nonsense"):
            preflight(["nonsense"], platform="linux")

    def test_unsigned_ci_pipeline_preflights_clean(self):
        from bos_build.core.planner import preflight

        preflight(plan(CI, "x64", "linux"), platform="linux")

    def test_step_preflight_hook_failures_reported(self):
        from types import SimpleNamespace
        from unittest import mock

        from bos_build.core import step as step_mod
        from bos_build.core.planner import preflight
        from bos_build.core.step import Step, ValidationError

        class NeedsTool(Step):
            def preflight(self, context):
                raise ValidationError("xcode 26 required")

            def validate(self, context):
                pass

            def execute(self, context):
                pass

        with mock.patch.dict(step_mod._REGISTRY, {"needs_tool": NeedsTool}):
            with self.assertRaisesRegex(ValueError, "xcode 26 required"):
                preflight(["needs_tool"], platform="linux", ctx=SimpleNamespace())


class DownloadSwitchTest(unittest.TestCase):
    def test_no_download_drops_resource_download_only(self):
        # The signed macOS nightly profile stages server resources locally
        # and disables only download_resources.
        with_dl = plan(RELEASE, "arm64", "macos")
        without = plan(Switches(preset="release", download=False), "arm64", "macos")
        self.assertEqual([s for s in with_dl if s != "download_resources"], without)

    def test_shipped_nightly_ci_profile_matches_ci_switches(self):
        shipped = Path(__file__).resolve().parents[1] / "profiles" / "nightly-ci.yaml"
        prof = load_profile(shipped)
        # Shipped profiles stay switch-based; modules: is a local-only opt-in.
        self.assertIsNone(prof.modules)
        self.assertFalse(prof.switches.bundle_local_extensions)
        for platform, arch in (
            ("macos", "arm64"),
            ("windows", "x64"),
            ("linux", "x64"),
        ):
            self.assertEqual(
                plan(prof.switches, arch, platform),
                plan(CI, arch, platform),
                f"profile drift on {platform}/{arch}",
            )


class UniversalRunsTest(unittest.TestCase):
    def test_flat_plan_rejects_universal(self):
        with self.assertRaisesRegex(ValueError, "plan_runs"):
            plan(RELEASE, "universal", "macos")

    def test_universal_requires_sign(self):
        with self.assertRaisesRegex(ValueError, "always signed"):
            plan_runs(
                Switches(preset="release", architectures=("universal",), sign=False),
                "macos",
            )

    def test_universal_rejected_in_multi_arch_list(self):
        with self.assertRaisesRegex(ValueError, "cannot be combined"):
            plan_runs(
                Switches(preset="release", architectures=("x64", "universal")),
                "macos",
            )

    def test_noupload_drops_upload_from_every_run(self):
        runs = plan_runs(
            Switches(preset="release", architectures=("universal",), upload=False),
            "macos",
        )
        self.assertEqual([arch for arch, _ in runs], ["arm64", "x64", "universal"])
        for arch, steps in runs:
            self.assertNotIn("upload", steps, arch)
            self.assertEqual(steps[-2:], ["package_macos", "sparkle_sign"], arch)

    def test_non_universal_runs_match_flat_plan_per_arch(self):
        sw = Switches(preset="release", architectures=("x64", "arm64"))
        self.assertEqual(
            plan_runs(sw, "linux"),
            [(arch, plan(sw, arch, "linux")) for arch in ("x64", "arm64")],
        )


class UniversalEnvTest(unittest.TestCase):
    def test_universal_runs_require_signing_and_sparkle_env_upfront(self):
        for arch, steps in plan_runs(UNIVERSAL, "macos"):
            self.assertEqual(
                required_env(steps),
                [
                    "MACOS_CERTIFICATE_NAME",
                    "PROD_MACOS_NOTARIZATION_APPLE_ID",
                    "PROD_MACOS_NOTARIZATION_TEAM_ID",
                    "PROD_MACOS_NOTARIZATION_PWD",
                    "SPARKLE_PRIVATE_KEY",
                ],
                arch,
            )


if __name__ == "__main__":
    unittest.main()
