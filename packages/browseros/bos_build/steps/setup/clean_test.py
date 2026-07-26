#!/usr/bin/env python3
"""Tests for the clean module against a mock checkout."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import clean
from ...core.context import Context
from ...core.step import ValidationError
from ...lib.testing import MockBrowserOSRoot, MockChromium, make_context


class CleanValidateTest(unittest.TestCase):
    def test_missing_chromium_src_raises(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = MockBrowserOSRoot(Path(tmp) / "root")
            ctx = Context(
                root_dir=root.root,
                chromium_src=Path(tmp) / "missing-src",
                architecture="x64",
                build_type="release",
            )
            with self.assertRaises(ValidationError):
                clean.CleanModule().validate(ctx)


class CleanExecuteTest(unittest.TestCase):
    def test_removes_out_dir_and_sparkle_and_resets_git(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            chromium = MockChromium(Path(chromium_tmp))
            ctx = make_context(
                chromium, MockBrowserOSRoot(Path(root_tmp)), architecture="x64"
            )
            out_dir = chromium.with_out_dir("x64", args_gn="is_debug = false\n")
            sparkle = chromium.with_sparkle()
            winsparkle = chromium.with_winsparkle()

            with mock.patch.object(clean, "run_command") as run_cmd:
                clean.CleanModule().execute(ctx)

            self.assertFalse(out_dir.exists())
            self.assertFalse(sparkle.exists())
            self.assertFalse(winsparkle.exists())

            git_commands = [call.args[0] for call in run_cmd.call_args_list]
            self.assertEqual(
                git_commands[0], ["git", "reset", "--hard", "HEAD"]
            )
            self.assertTrue(
                all(cmd[0] == "git" for cmd in git_commands),
                f"expected only git commands, got: {git_commands}",
            )
            for call in run_cmd.call_args_list:
                self.assertEqual(call.kwargs["cwd"], ctx.chromium_src)

    def test_missing_out_dir_is_tolerated(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            chromium = MockChromium(Path(chromium_tmp))
            ctx = make_context(chromium, MockBrowserOSRoot(Path(root_tmp)))

            with mock.patch.object(clean, "run_command"):
                clean.CleanModule().execute(ctx)


class CleanPruneOrphanBinariesTest(unittest.TestCase):
    """Pruning of resources/binaries/<family> dirs absent from the download config."""

    MANAGED_CONFIG = {
        "download_operations": [
            {
                "name": "Server arm64",
                "destination": "resources/binaries/browseros_server/darwin-arm64",
            },
            {
                "name": "Rust claw server arm64",
                "destination": "resources/binaries/browseros_claw_server_rust/darwin-arm64",
            },
            {
                "name": "Onboard",
                "destination": "resources/binaries/browseros_claw_onboard",
            },
        ]
    }

    def _execute(self, ctx):
        with mock.patch.object(clean, "run_command"):
            clean.CleanModule().execute(ctx)

    def test_prunes_orphan_and_keeps_managed_families_and_loose_files(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            root = MockBrowserOSRoot(Path(root_tmp))
            root.write_download_config(self.MANAGED_CONFIG)
            ctx = make_context(MockChromium(Path(chromium_tmp)), root)

            binaries = root.root / "resources" / "binaries"
            orphan = binaries / "browseros_claw_server"
            (orphan / "darwin-arm64").mkdir(parents=True)
            (orphan / "darwin-arm64" / "artifact-metadata.json").write_text("{}")

            managed = binaries / "browseros_server" / "darwin-arm64"
            managed.mkdir(parents=True)
            (managed / "artifact-metadata.json").write_text("{}")

            # Nightly-macos stages local bundles without artifact metadata;
            # membership in the config alone must keep the family.
            staged = binaries / "browseros_claw_server_rust" / "darwin-arm64"
            staged.mkdir(parents=True)
            (staged / "browseros_claw_server_rust").write_text("binary")

            loose_file = binaries / "README.txt"
            loose_file.write_text("not a family dir")

            self._execute(ctx)

            self.assertFalse(orphan.exists())
            self.assertTrue(managed.exists())
            self.assertTrue(staged.exists())
            self.assertTrue(loose_file.exists())

    def test_empty_managed_set_prunes_nothing(self):
        # Fail-safe pin: a missing/unparseable config yields an empty managed
        # set, which must mean "unknown — prune nothing", never "prune all".
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            root = MockBrowserOSRoot(Path(root_tmp))
            ctx = make_context(MockChromium(Path(chromium_tmp)), root)
            self.assertFalse(ctx.get_download_resources_config().exists())

            orphan = root.root / "resources" / "binaries" / "browseros_claw_server"
            orphan.mkdir(parents=True)

            self._execute(ctx)

            self.assertTrue(orphan.exists())

    def test_missing_binaries_dir_is_tolerated(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            root = MockBrowserOSRoot(Path(root_tmp))
            root.write_download_config(self.MANAGED_CONFIG)
            ctx = make_context(MockChromium(Path(chromium_tmp)), root)

            self._execute(ctx)

            self.assertFalse((root.root / "resources" / "binaries").exists())


if __name__ == "__main__":
    unittest.main()
