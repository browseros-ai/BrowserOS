#!/usr/bin/env python3
"""Tests for chromium source provisioning."""

import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bos_build.core.planner import Switches, plan
from bos_build.steps.source import provision


class PinnedVersionTest(unittest.TestCase):
    def test_parses_major_minor_build_patch(self):
        with tempfile.NamedTemporaryFile("w", suffix="VERSION", delete=False) as f:
            f.write("MAJOR=148\nMINOR=0\nBUILD=7402\nPATCH=57\n")
            path = Path(f.name)
        self.addCleanup(path.unlink)
        self.assertEqual(provision.read_pinned_version(path), "148.0.7402.57")


class GclientConfigTest(unittest.TestCase):
    def test_written_when_missing_and_idempotent(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            provision.ensure_gclient_config(root)
            spec = (root / ".gclient").read_text()
            self.assertIn('"name": "src"', spec)
            self.assertIn(provision.CHROMIUM_SRC_URL, spec)

            (root / ".gclient").write_text(spec)
            provision.ensure_gclient_config(root)
            self.assertEqual((root / ".gclient").read_text(), spec)


class DepotToolsRecoveryTest(unittest.TestCase):
    def _git(self, cwd: Path, *args: str, check: bool = True):
        return subprocess.run(
            ["git", *args],
            cwd=cwd,
            capture_output=True,
            check=check,
            text=True,
        )

    def _create_depot_tools(self, root: Path) -> tuple[Path, Path]:
        depot_tools = root / "depot_tools"
        depot_tools.mkdir()
        self._git(depot_tools, "init")
        self._git(depot_tools, "config", "user.name", "BrowserOS test")
        self._git(depot_tools, "config", "user.email", "ci@example.invalid")
        tracked = depot_tools / "gclient.py"
        tracked.write_bytes(b"first line\nsecond line\n")
        self._git(depot_tools, "add", tracked.name)
        self._git(depot_tools, "commit", "-m", "initial")
        return depot_tools, tracked

    def _tracked_status(self, depot_tools: Path) -> str:
        return self._git(
            depot_tools,
            "status",
            "--porcelain=v1",
            "--untracked-files=no",
        ).stdout

    def test_authorized_repair_normalizes_only_crlf_and_preserves_untracked(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            depot_tools, tracked = self._create_depot_tools(root)
            tracked.write_bytes(b"first line\r\nsecond line\r\n")
            untracked = depot_tools / "local-note.txt"
            untracked.write_text("preserve me")
            self.assertTrue(self._tracked_status(depot_tools))

            provision.ensure_depot_tools(root, repair_cached_depot_tools=True)

            self.assertEqual(tracked.read_bytes(), b"first line\nsecond line\n")
            self.assertEqual(self._tracked_status(depot_tools), "")
            self.assertEqual(untracked.read_text(), "preserve me")

    def test_authorized_repair_leaves_clean_checkout_unchanged(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            depot_tools, tracked = self._create_depot_tools(root)
            before = tracked.stat().st_mtime_ns

            provision.ensure_depot_tools(root, repair_cached_depot_tools=True)

            self.assertEqual(self._tracked_status(depot_tools), "")
            self.assertEqual(tracked.stat().st_mtime_ns, before)

    def test_authorized_repair_rejects_and_preserves_substantive_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            depot_tools, tracked = self._create_depot_tools(root)
            changed = b"first line\nchanged content\n"
            tracked.write_bytes(changed)

            with self.assertRaisesRegex(
                RuntimeError,
                "substantive tracked changes",
            ):
                provision.ensure_depot_tools(root, repair_cached_depot_tools=True)

            self.assertEqual(tracked.read_bytes(), changed)
            self.assertTrue(self._tracked_status(depot_tools))

    def test_authorized_repair_rejects_trailing_whitespace_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            depot_tools, tracked = self._create_depot_tools(root)
            changed = b"first line \nsecond line\n"
            tracked.write_bytes(changed)

            with self.assertRaisesRegex(
                RuntimeError,
                "substantive tracked changes",
            ):
                provision.ensure_depot_tools(
                    root,
                    repair_cached_depot_tools=True,
                )

            self.assertEqual(tracked.read_bytes(), changed)
            self.assertTrue(self._tracked_status(depot_tools))

    def test_authorized_repair_rejects_hidden_index_flags(self):
        for index_flag in ("--assume-unchanged", "--skip-worktree"):
            with self.subTest(index_flag=index_flag):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    depot_tools, tracked = self._create_depot_tools(root)
                    self._git(
                        depot_tools,
                        "update-index",
                        index_flag,
                        tracked.name,
                    )
                    changed = b"first line\nhidden substantive change\n"
                    tracked.write_bytes(changed)
                    self.assertEqual(self._tracked_status(depot_tools), "")

                    with self.assertRaisesRegex(
                        RuntimeError,
                        "non-default index flags",
                    ):
                        provision.ensure_depot_tools(
                            root,
                            repair_cached_depot_tools=True,
                        )

                    self.assertEqual(tracked.read_bytes(), changed)

    def test_default_preserves_dirty_developer_checkout(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            depot_tools, tracked = self._create_depot_tools(root)
            changed = b"first line\r\nsecond line\r\n"
            tracked.write_bytes(changed)

            provision.ensure_depot_tools(root)

            self.assertEqual(tracked.read_bytes(), changed)
            self.assertTrue(self._tracked_status(depot_tools))

    def test_authorized_repair_rejects_invalid_repository(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "depot_tools" / ".git").mkdir(parents=True)

            with self.assertRaisesRegex(
                RuntimeError,
                "not a valid Git worktree",
            ):
                provision.ensure_depot_tools(root, repair_cached_depot_tools=True)


class CheckoutCommandsTest(unittest.TestCase):
    def _run_checkout(self, strategy: str):
        commands = []

        def fake_run(cmd, cwd, env=None):
            commands.append([str(c) for c in cmd])

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with (
                mock.patch.object(provision, "run", fake_run),
                mock.patch.object(provision, "_git_output", return_value=""),
            ):
                provision.checkout(root, "148.0.7402.57", strategy=strategy)
        return commands

    def test_shallow_fetches_pinned_tag_depth_two_no_tags(self):
        commands = self._run_checkout("shallow")
        fetch = next(c for c in commands if c[:2] == ["git", "fetch"])
        self.assertEqual(
            fetch,
            [
                "git",
                "fetch",
                "--depth",
                "2",
                "--no-tags",
                "origin",
                "+refs/tags/148.0.7402.57:refs/tags/148.0.7402.57",
            ],
        )

    def test_full_fetches_pinned_tag_without_depth(self):
        commands = self._run_checkout("full")
        fetch = next(c for c in commands if c[:2] == ["git", "fetch"])
        self.assertNotIn("--depth", fetch)
        self.assertIn("+refs/tags/148.0.7402.57:refs/tags/148.0.7402.57", fetch)

    def test_unknown_strategy_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, "Unknown strategy"):
                provision.checkout(Path(tmp), "1.2.3.4", strategy="warp")


class SyncCommandTest(unittest.TestCase):
    def test_sync_uses_no_history_shallow(self):
        commands = []

        def fake_run(cmd, cwd, env=None):
            commands.append([str(c) for c in cmd])

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with mock.patch.object(provision, "run", fake_run):
                provision.sync(root)

        self.assertEqual(len(commands), 1)
        self.assertEqual(commands[0][1:], ["sync", "-D", "--no-history", "--shallow"])


class ShallowProvisionPlanTest(unittest.TestCase):
    def test_shallow_interleaves_clean_between_checkout_and_sync(self):
        sw = Switches(preset="release", provision="shallow", sign=False, upload=False)
        steps = plan(sw, "x64", "linux")
        self.assertEqual(
            steps[:3], ["source_checkout", "clean", "source_sync"]
        )
        self.assertNotIn("git_setup", steps)

    def test_shallow_without_clean(self):
        sw = Switches(
            preset="release",
            provision="shallow",
            clean=False,
            sign=False,
            upload=False,
        )
        steps = plan(sw, "x64", "linux")
        self.assertEqual(steps[:2], ["source_checkout", "source_sync"])


if __name__ == "__main__":
    unittest.main()
