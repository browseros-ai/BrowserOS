#!/usr/bin/env python3
"""Tests for extract command default base commit handling."""

import tempfile
import unittest
from contextlib import nullcontext
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from .common import resolve_base_commit
from .extract_commit import extract_single_commit
from .extract_patch import extract_single_file_patch
from .extract_range import extract_commits_individually
from .utils import FileOperation, FilePatch


def make_context(root_dir: Path) -> SimpleNamespace:
    return SimpleNamespace(
        root_dir=root_dir,
        chromium_src=Path("/tmp/chromium"),
        get_patch_path_for_file=lambda rel: root_dir / "chromium_patches" / rel,
    )


class ExtractBaseDefaultTest(unittest.TestCase):
    def test_resolve_base_commit_reads_base_commit_when_base_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "BASE_COMMIT").write_text("base123\n", encoding="utf-8")

            self.assertEqual(resolve_base_commit(make_context(root), None), "base123")

    def test_resolve_base_commit_preserves_explicit_base(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "BASE_COMMIT").write_text("base123\n", encoding="utf-8")

            self.assertEqual(
                resolve_base_commit(make_context(root), "explicit456"),
                "explicit456",
            )

    def test_extract_single_commit_uses_base_commit_by_default(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "BASE_COMMIT").write_text("base123\n", encoding="utf-8")
            ctx = make_context(root)

            with (
                patch(
                    "bos_build.patchkit.extract.extract_commit.validate_commit_exists",
                    return_value=True,
                ),
                patch(
                    "bos_build.patchkit.extract.extract_commit.get_commit_info",
                    return_value=None,
                ),
                patch(
                    "bos_build.patchkit.extract.extract_commit.extract_with_base",
                    return_value=(1, ["chrome/foo.cc"]),
                ) as extract_with_base_mock,
            ):
                result = extract_single_commit(ctx, "HEAD", force=True)

            self.assertEqual(result, (1, ["chrome/foo.cc"]))
            extract_with_base_mock.assert_called_once_with(
                ctx, "HEAD", "base123", False, True, False
            )

    def test_extract_single_file_patch_uses_base_commit_by_default(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "BASE_COMMIT").write_text("base123\n", encoding="utf-8")
            ctx = make_context(root)
            diff_result = SimpleNamespace(returncode=0, stdout="diff", stderr="")
            patch_file = FilePatch(
                file_path="chrome/foo.cc",
                operation=FileOperation.MODIFY,
                patch_content="diff",
                is_binary=False,
            )

            with (
                patch(
                    "bos_build.patchkit.extract.extract_patch.validate_commit_exists",
                    return_value=True,
                ) as validate_mock,
                patch(
                    "bos_build.patchkit.extract.extract_patch.run_git_command",
                    return_value=diff_result,
                ) as git_mock,
                patch(
                    "bos_build.patchkit.extract.extract_patch.parse_diff_output",
                    return_value={"chrome/foo.cc": patch_file},
                ),
                patch(
                    "bos_build.patchkit.extract.extract_patch.write_patch_file",
                    return_value=True,
                ),
            ):
                success, error = extract_single_file_patch(
                    ctx, "chrome/foo.cc", None, force=True
                )

            self.assertTrue(success)
            self.assertIsNone(error)
            validate_mock.assert_called_once_with("base123", ctx.chromium_src)
            git_mock.assert_called_once_with(
                ["git", "diff", "base123", "--", "chrome/foo.cc"],
                cwd=ctx.chromium_src,
            )

    def test_extract_commits_individually_uses_base_commit_by_default(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "BASE_COMMIT").write_text("base123\n", encoding="utf-8")
            ctx = make_context(root)
            rev_list = SimpleNamespace(returncode=0, stdout="commit1\n", stderr="")

            with (
                patch(
                    "bos_build.patchkit.extract.extract_range.validate_commit_exists",
                    return_value=True,
                ),
                patch(
                    "bos_build.patchkit.extract.extract_range.run_git_command",
                    return_value=rev_list,
                ),
                patch(
                    "bos_build.patchkit.extract.extract_range.extract_with_base",
                    return_value=(1, ["chrome/foo.cc"]),
                ) as extract_with_base_mock,
                patch(
                    "click.progressbar",
                    side_effect=lambda items, **_: nullcontext(items),
                ),
            ):
                result = extract_commits_individually(ctx, "START", "END", force=True)

            self.assertEqual(result, (1, ["chrome/foo.cc"]))
            extract_with_base_mock.assert_called_once_with(
                ctx,
                "commit1",
                "base123",
                verbose=False,
                force=True,
                include_binary=False,
            )


if __name__ == "__main__":
    unittest.main()
