#!/usr/bin/env python3
"""Tests for branding string replacements against a mock checkout."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast

from .string_replaces import StringReplacesModule, apply_string_replacements_impl
from ...common.context import Context
from ...common.module import ValidationError
from ...common.testing import MockBrowserOSRoot, MockChromium, make_context


class ApplyStringReplacementsTest(unittest.TestCase):
    def setUp(self):
        self._chromium_tmp = tempfile.TemporaryDirectory()
        self._root_tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._chromium_tmp.cleanup)
        self.addCleanup(self._root_tmp.cleanup)
        self.chromium = MockChromium(Path(self._chromium_tmp.name))
        self.root = MockBrowserOSRoot(Path(self._root_tmp.name))
        self.ctx = make_context(self.chromium, self.root)

    def test_rebrands_target_files(self):
        self.chromium.with_branding_files()

        self.assertTrue(apply_string_replacements_impl(self.ctx))

        content = (
            self.chromium.src / "chrome" / "app" / "chromium_strings.grd"
        ).read_text()
        self.assertNotIn("Google Chrome", content)
        self.assertNotIn("Chromium", content)
        self.assertIn("BrowserOS", content)
        self.assertIn("The BrowserOS Authors. All rights reserved.", content)

    def test_google_play_is_preserved(self):
        self.chromium.with_branding_files()

        self.assertTrue(apply_string_replacements_impl(self.ctx))

        content = (
            self.chromium.src / "chrome" / "app" / "chromium_strings.grd"
        ).read_text()
        self.assertIn("Google Play", content)
        self.assertNotIn("BrowserOS Play", content)

    def test_both_target_files_are_processed(self):
        self.chromium.with_branding_files()

        self.assertTrue(apply_string_replacements_impl(self.ctx))

        grdp = (
            self.chromium.src / "chrome" / "app" / "settings_chromium_strings.grdp"
        ).read_text()
        self.assertNotIn("Google Chrome", grdp)
        self.assertIn("BrowserOS", grdp)

    def test_missing_target_file_is_tolerated(self):
        self.chromium.add_file(
            "chrome/app/chromium_strings.grd", "<grit>Google Chrome</grit>\n"
        )
        # settings_chromium_strings.grdp intentionally absent

        self.assertTrue(apply_string_replacements_impl(self.ctx))

        content = (
            self.chromium.src / "chrome" / "app" / "chromium_strings.grd"
        ).read_text()
        self.assertIn("BrowserOS", content)

    def test_all_targets_missing_still_succeeds(self):
        self.assertTrue(apply_string_replacements_impl(self.ctx))

    def test_non_target_files_untouched(self):
        self.chromium.with_branding_files()
        other = self.chromium.add_file(
            "chrome/app/google_chrome_strings.grd", "<grit>Chromium</grit>\n"
        )

        self.assertTrue(apply_string_replacements_impl(self.ctx))

        self.assertEqual(other.read_text(), "<grit>Chromium</grit>\n")

    def test_browserclaw_context_rebrands_to_browserclaw(self):
        self.chromium.with_branding_files()
        ctx = make_context(self.chromium, self.root, product="browserclaw")

        self.assertTrue(apply_string_replacements_impl(ctx))

        content = (
            self.chromium.src / "chrome" / "app" / "chromium_strings.grd"
        ).read_text()
        self.assertIn("BrowserClaw", content)
        self.assertNotIn("BrowserOS", content)


class StringReplacesModuleValidateTest(unittest.TestCase):
    def test_missing_chromium_src_raises_validation_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            ctx = cast(
                Context, SimpleNamespace(chromium_src=Path(tmp) / "missing")
            )
            with self.assertRaises(ValidationError):
                StringReplacesModule().validate(ctx)


if __name__ == "__main__":
    unittest.main()
