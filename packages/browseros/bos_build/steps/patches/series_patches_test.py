"""Series execution validates its complete input list before applying patches."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from .series_patches import apply_series_patches_impl


class SeriesInputTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.series = self.root / "series_patches"
        self.series.mkdir()
        self.ctx = SimpleNamespace(
            get_series_patches_dir=lambda: self.series,
            chromium_src=self.root / "src",
        )
        (self.series / "first.patch").write_text("first")
        (self.series / "second.patch").write_text("second")
        (self.series / "series").write_text("# common\nfirst.patch # inline note\n")

    def test_common_and_platform_patch_order_is_preserved(self):
        (self.series / "series.macos").write_text("\nsecond.patch\n")
        with (
            mock.patch(
                "bos_build.steps.patches.series_patches.get_platform",
                return_value="macos",
            ),
            mock.patch(
                "bos_build.steps.patches.series_patches.apply_single_patch",
                return_value=(True, ""),
            ) as apply,
        ):
            applied, failed = apply_series_patches_impl(self.ctx)
        self.assertEqual(
            applied, [self.series / "first.patch", self.series / "second.patch"]
        )
        self.assertEqual(failed, [])
        self.assertEqual([call.args[0] for call in apply.call_args_list], applied)

    def test_outside_platform_entry_stops_before_valid_common_patch(self):
        (self.root / "outside.patch").write_text("outside")
        (self.series / "series.macos").write_text("../outside.patch\n")
        with (
            mock.patch(
                "bos_build.steps.patches.series_patches.get_platform",
                return_value="macos",
            ),
            mock.patch(
                "bos_build.steps.patches.series_patches.apply_single_patch"
            ) as apply,
        ):
            with self.assertRaisesRegex(
                ValueError, "Series patch path must stay within"
            ):
                apply_series_patches_impl(self.ctx)
            apply.assert_not_called()

    def test_link_outside_series_directory_is_rejected_at_execution(self):
        outside = self.root / "outside.patch"
        outside.write_text("outside")
        (self.series / "link.patch").symlink_to(outside)
        (self.series / "series").write_text("first.patch\nlink.patch\n")
        with mock.patch(
            "bos_build.steps.patches.series_patches.apply_single_patch"
        ) as apply:
            with self.assertRaisesRegex(
                ValueError, "Series patch path must stay within"
            ):
                apply_series_patches_impl(self.ctx)
            apply.assert_not_called()


if __name__ == "__main__":
    unittest.main()
