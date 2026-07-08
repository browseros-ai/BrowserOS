#!/usr/bin/env python3
"""Tests for non-interactive batch patch application."""

import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast

from bos_build.core.context import Context
from bos_build.patchkit.batch_apply import (
    apply_all_patches,
    check_patch_applies,
    find_patch_files,
)


def _git(cwd: Path, *args: str) -> None:
    subprocess.run(
        ["git", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
        env={
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@t",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@t",
            "PATH": "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
            "HOME": str(cwd),
        },
    )


def _make_repo(root: Path) -> Path:
    repo = root / "src"
    repo.mkdir()
    _git(repo, "init", "-q")
    (repo / "chrome").mkdir()
    (repo / "chrome" / "a.txt").write_text("line one\nline two\n")
    _git(repo, "add", "-A")
    _git(repo, "commit", "-qm", "base")
    return repo


def _make_patch(repo: Path, patches_dir: Path) -> None:
    """Produce a real git diff patch for chrome/a.txt into patches_dir."""
    (repo / "chrome" / "a.txt").write_text("line one\nline two changed\n")
    diff = subprocess.run(
        ["git", "diff"],
        cwd=repo,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    _git(repo, "checkout", "--", "chrome/a.txt")
    patch_path = patches_dir / "chrome" / "a.txt"
    patch_path.parent.mkdir(parents=True)
    patch_path.write_text(diff)


class BatchApplyTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.repo = _make_repo(self.root)
        self.patches_dir = self.root / "chromium_patches"
        self.patches_dir.mkdir()

    def _ctx(self) -> Context:
        return cast(
            Context,
            SimpleNamespace(
                chromium_src=self.repo,
                get_patches_dir=lambda: self.patches_dir,
            ),
        )

    def test_applies_valid_patch_set(self):
        _make_patch(self.repo, self.patches_dir)

        applied, failed = apply_all_patches(self._ctx())

        self.assertEqual((applied, failed), (1, []))
        self.assertIn(
            "line two changed", (self.repo / "chrome" / "a.txt").read_text()
        )

    def test_corrupt_patch_reports_failure(self):
        bad = self.patches_dir / "chrome" / "a.txt"
        bad.parent.mkdir(parents=True)
        bad.write_text("this is not a patch\n")

        applied, failed = apply_all_patches(self._ctx())

        self.assertEqual(applied, 0)
        self.assertEqual(len(failed), 1)

    def test_missing_patches_dir_is_a_noop(self):
        ctx = cast(
            Context,
            SimpleNamespace(
                chromium_src=self.repo,
                get_patches_dir=lambda: self.root / "nope",
            ),
        )
        self.assertEqual(apply_all_patches(ctx), (0, []))

    def test_find_patch_files_skips_markers_and_hidden(self):
        (self.patches_dir / "a.patch").write_text("x")
        (self.patches_dir / "b.deleted").write_text("x")
        (self.patches_dir / "c.binary").write_text("x")
        (self.patches_dir / "d.rename").write_text("x")
        (self.patches_dir / ".hidden").write_text("x")

        names = [p.name for p in find_patch_files(self.patches_dir)]
        self.assertEqual(names, ["a.patch"])

    def test_find_patch_files_skips_root_metadata_but_not_nested_yaml(self):
        # bpatch-v2 metadata at the tree root is not a patch; a chromium
        # yaml file patched deeper in the tree still is.
        (self.patches_dir / "features.yaml").write_text("x")
        (self.patches_dir / "store.yaml").write_text("x")
        nested = self.patches_dir / "chrome" / "app"
        nested.mkdir(parents=True)
        (nested / "features.yaml").write_text("x")

        names = [
            str(p.relative_to(self.patches_dir))
            for p in find_patch_files(self.patches_dir)
        ]
        self.assertEqual(names, ["chrome/app/features.yaml"])

    def test_dry_run_reports_without_modifying_tree(self):
        _make_patch(self.repo, self.patches_dir)

        applied, failed = apply_all_patches(self._ctx(), dry_run=True)

        self.assertEqual((applied, failed), (1, []))
        self.assertNotIn(
            "line two changed", (self.repo / "chrome" / "a.txt").read_text()
        )


class CheckPatchAppliesTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.repo = _make_repo(self.root)
        self.patches_dir = self.root / "chromium_patches"
        self.patches_dir.mkdir()

    def test_clean_patch_passes_and_tree_untouched(self):
        _make_patch(self.repo, self.patches_dir)

        ok, error = check_patch_applies(
            self.patches_dir / "chrome" / "a.txt", self.repo
        )

        self.assertTrue(ok)
        self.assertIsNone(error)
        self.assertNotIn(
            "line two changed", (self.repo / "chrome" / "a.txt").read_text()
        )

    def test_failing_patch_returns_stderr(self):
        bad = self.patches_dir / "chrome" / "a.txt"
        bad.parent.mkdir(parents=True)
        bad.write_text(
            "--- a/chrome/a.txt\n+++ b/chrome/a.txt\n"
            "@@ -1,1 +1,1 @@\n-no such line\n+replacement\n"
        )

        ok, error = check_patch_applies(bad, self.repo)

        self.assertFalse(ok)
        self.assertTrue(error)


if __name__ == "__main__":
    unittest.main()
