#!/usr/bin/env python3
"""Tests for chromium file replacement against a mock checkout."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast

from .chromium_replace import ChromiumReplaceModule, replace_chromium_files_impl
from ...core.context import Context
from ...core.step import ValidationError
from ...lib.testing import MockBrowserOSRoot, MockChromium, make_context


class ReplaceChromiumFilesTest(unittest.TestCase):
    def _make(self, build_type: str):
        chromium_tmp = tempfile.TemporaryDirectory()
        root_tmp = tempfile.TemporaryDirectory()
        self.addCleanup(chromium_tmp.cleanup)
        self.addCleanup(root_tmp.cleanup)
        chromium = MockChromium(Path(chromium_tmp.name))
        root = MockBrowserOSRoot(Path(root_tmp.name))
        ctx = make_context(chromium, root, build_type=build_type)
        return chromium, root, ctx

    def test_missing_chromium_files_dir_is_noop(self):
        _, _, ctx = self._make("release")
        self.assertTrue(replace_chromium_files_impl(ctx))

    def test_generic_file_replaces_existing_destination(self):
        chromium, root, ctx = self._make("release")
        chromium.add_file("chrome/common/branding.h", "// original\n")
        root.add_replacement_file("chrome/common/branding.h", "// custom\n")

        self.assertTrue(replace_chromium_files_impl(ctx))

        self.assertEqual(
            (chromium.src / "chrome" / "common" / "branding.h").read_text(),
            "// custom\n",
        )

    def test_missing_destination_raises(self):
        _, root, ctx = self._make("release")
        root.add_replacement_file("chrome/common/not_in_chromium.h", "// custom\n")

        with self.assertRaises(FileNotFoundError):
            replace_chromium_files_impl(ctx)

    def test_build_type_variant_replaces_when_matching(self):
        chromium, root, ctx = self._make("release")
        chromium.add_file("chrome/common/flags.cc", "// original\n")
        root.add_replacement_file("chrome/common/flags.cc.release", "// release\n")

        self.assertTrue(replace_chromium_files_impl(ctx))

        self.assertEqual(
            (chromium.src / "chrome" / "common" / "flags.cc").read_text(),
            "// release\n",
        )

    def test_build_type_variant_skipped_when_not_matching(self):
        chromium, root, ctx = self._make("debug")
        chromium.add_file("chrome/common/flags.cc", "// original\n")
        root.add_replacement_file("chrome/common/flags.cc.release", "// release\n")

        self.assertTrue(replace_chromium_files_impl(ctx))

        self.assertEqual(
            (chromium.src / "chrome" / "common" / "flags.cc").read_text(),
            "// original\n",
        )

    def test_variant_wins_over_generic_for_matching_build_type(self):
        chromium, root, ctx = self._make("release")
        chromium.add_file("chrome/common/mixed.cc", "// original\n")
        root.add_replacement_file("chrome/common/mixed.cc", "// generic\n")
        root.add_replacement_file("chrome/common/mixed.cc.release", "// release\n")

        self.assertTrue(replace_chromium_files_impl(ctx))

        self.assertEqual(
            (chromium.src / "chrome" / "common" / "mixed.cc").read_text(),
            "// release\n",
        )

    def test_common_overlay_applies_before_product_overlay(self):
        chromium, root, ctx = self._make("release")
        chromium.add_file("chrome/common/branding.h", "// original\n")
        root.add_replacement_file(
            "chrome/common/branding.h", "// common\n", product="common"
        )
        root.add_replacement_file("chrome/common/branding.h", "// product\n")

        self.assertTrue(replace_chromium_files_impl(ctx))

        self.assertEqual(
            (chromium.src / "chrome" / "common" / "branding.h").read_text(),
            "// product\n",
        )


class ChromiumReplaceModuleValidateTest(unittest.TestCase):
    def test_missing_chromium_src_raises_validation_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            ctx = cast(
                Context, SimpleNamespace(chromium_src=Path(tmp) / "missing")
            )
            with self.assertRaises(ValidationError):
                ChromiumReplaceModule().validate(ctx)


if __name__ == "__main__":
    unittest.main()
