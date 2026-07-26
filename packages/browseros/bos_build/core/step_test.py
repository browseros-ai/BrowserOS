#!/usr/bin/env python3
"""Tests for the step registry."""

import unittest

from bos_build.core import step as step_mod
from bos_build.core.step import (
    Step,
    all_steps,
    phase_steps,
    step,
)


class RegistryContentTest(unittest.TestCase):
    def test_all_pipeline_steps_registered(self):
        names = set(all_steps())
        self.assertEqual(
            names,
            {
                "source_checkout",
                "source_sync",
                "clean",
                "git_setup",
                "sparkle_setup",
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
                "merge_universal",
                "sign_macos",
                "sign_windows",
                "sign_linux",
                "sparkle_sign",
                "mini_installer",
                "package_windows",
                "package_macos",
                "package_linux",
                "upload",
            },
        )

    def test_phase_order_matches_legacy_execution_order_macos(self):
        self.assertEqual(
            phase_steps("setup", "macos"), ["clean", "git_setup", "sparkle_setup"]
        )
        self.assertEqual(
            phase_steps("prep", "macos"),
            [
                "download_resources",
                "resources",
                "bundled_extensions",
                "chromium_replace",
                "string_replaces",
                "patches",
                "configure",
            ],
        )
        self.assertEqual(phase_steps("build", "macos"), ["compile"])
        self.assertEqual(phase_steps("sign", "macos"), ["sign_macos"])
        self.assertEqual(phase_steps("package", "macos"), ["package_macos"])
        self.assertEqual(phase_steps("upload", "macos"), ["upload"])

    def test_platform_filtering_windows_and_linux(self):
        self.assertEqual(
            phase_steps("setup", "windows"),
            ["clean", "git_setup", "winsparkle_setup"],
        )
        self.assertEqual(phase_steps("setup", "linux"), ["clean", "git_setup"])
        self.assertEqual(phase_steps("sign", "windows"), ["sign_windows"])
        self.assertEqual(phase_steps("sign", "linux"), ["sign_linux"])
        self.assertEqual(phase_steps("package", "linux"), ["package_linux"])

    def test_optional_steps_excluded_unless_requested(self):
        self.assertNotIn("series_patches", phase_steps("prep", "macos"))
        self.assertIn(
            "series_patches", phase_steps("prep", "macos", include_optional=True)
        )
        self.assertNotIn("merge_universal", phase_steps("build", "macos"))
        self.assertNotIn("mini_installer", phase_steps("sign", "windows"))

class RegistrationRulesTest(unittest.TestCase):
    def test_duplicate_name_raises(self):
        with self.assertRaisesRegex(ValueError, "Duplicate step name 'clean'"):

            @step("clean", phase="setup")
            class DuplicateClean(Step):
                pass

    def test_unknown_phase_raises(self):
        with self.assertRaisesRegex(ValueError, "Unknown phase 'deploy'"):
            step("deployer", phase="deploy")

    def test_registration_sets_metadata(self):
        try:

            @step(
                "temp_step_for_test",
                phase="prep",
                platforms=("linux",),
                env=("SOME_VAR",),
                optional=True,
            )
            class TempStep(Step):
                pass

            self.assertEqual(TempStep.name, "temp_step_for_test")
            self.assertEqual(TempStep.phase, "prep")
            self.assertEqual(TempStep.platforms, ("linux",))
            self.assertEqual(TempStep.env, ("SOME_VAR",))
            self.assertTrue(TempStep.optional)
            self.assertTrue(TempStep().applies_to("linux"))
            self.assertFalse(TempStep().applies_to("macos"))
        finally:
            step_mod._REGISTRY.pop("temp_step_for_test", None)


if __name__ == "__main__":
    unittest.main()
