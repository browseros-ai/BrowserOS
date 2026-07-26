#!/usr/bin/env python3
"""Tests for the patches build module."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock

from . import patches
from ...patchkit import batch_apply
from ...core.context import Context
from ...core.step import ValidationError
from ...lib.testing import MockBrowserOSRoot, MockChromium, make_context


class PatchesModuleValidateTest(unittest.TestCase):
    def test_missing_git_raises_validation_error(self):
        ctx = cast(Context, SimpleNamespace())
        with mock.patch.object(patches.shutil, "which", return_value=None):
            with self.assertRaises(ValidationError):
                patches.PatchesModule().validate(ctx)

    def test_missing_patches_dir_raises_validation_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            ctx = cast(
                Context,
                SimpleNamespace(get_patches_dir=lambda: Path(tmp) / "missing"),
            )
            with self.assertRaises(ValidationError):
                patches.PatchesModule().validate(ctx)

    def test_existing_patches_dir_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            ctx = cast(
                Context, SimpleNamespace(get_patches_dir=lambda: Path(tmp))
            )
            patches.PatchesModule().validate(ctx)


class ApplyPatchesImplTest(unittest.TestCase):
    def _ctx(self) -> Context:
        chromium_tmp = tempfile.TemporaryDirectory()
        root_tmp = tempfile.TemporaryDirectory()
        self.addCleanup(chromium_tmp.cleanup)
        self.addCleanup(root_tmp.cleanup)
        return make_context(
            MockChromium(Path(chromium_tmp.name)),
            MockBrowserOSRoot(Path(root_tmp.name)),
        )

    def test_failures_raise_runtime_error_in_non_interactive_mode(self):
        ctx = self._ctx()
        with mock.patch.object(
            batch_apply, "apply_all_patches", return_value=(1, ["broken.patch"])
        ):
            with self.assertRaises(RuntimeError) as err:
                patches.apply_patches_impl(ctx, interactive=False)
        self.assertIn("1 patches", str(err.exception))

    def test_success_returns_true(self):
        ctx = self._ctx()
        with mock.patch.object(
            batch_apply, "apply_all_patches", return_value=(3, [])
        ) as apply_mock:
            self.assertTrue(patches.apply_patches_impl(ctx, interactive=False))
        apply_mock.assert_called_once_with(
            build_ctx=ctx, dry_run=False, interactive=False
        )


if __name__ == "__main__":
    unittest.main()
