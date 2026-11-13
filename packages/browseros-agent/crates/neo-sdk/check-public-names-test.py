#!/usr/bin/env python3
"""Exercise the public naming command against disposable Git and source-snapshot inputs."""

from pathlib import Path
import os
import subprocess
import tempfile
import unittest


CHECK = Path(__file__).with_name("check-public-names.py")
REPO = CHECK.resolve().parents[4]
NAMES = (b"zu" + b"zu", b"mirror" + b"d", b"mem" + b"0")
FILES = (
    b"fill" + b".js", b"login_" + b"fill" + b".js", b"code_" + b"entry" + b".js",
    b"focus_" + b"box" + b".js", b"private_" + b"values" + b".js",
    b"auth_" + b"login" + b".go",
)
PRODUCT_PATHS = (
    "docs/browseros/features/connect-mcps.mdx",
    "docs/browseros/features/use-with-claude-code.mdx",
    "packages/browseros-agent/apps/app/components/mcp/McpServerIcon.tsx",
    "packages/browseros-agent/apps/server/src/api/services/klavis/catalog.ts",
)


class PublicNamesTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="public-names-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")

    def git(self, *args):
        subprocess.run(["git", "-C", str(self.root), *args], check=True, capture_output=True)

    def write(self, relative, data):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        self.git("add", "--", relative)
        return path

    def check(self, expected, env=None):
        result = subprocess.run(
            ["python3", str(CHECK), str(self.root)], capture_output=True, env=env,
        )
        self.assertEqual(result.returncode, expected, result.stderr.decode())

    def test_tracked_bytes_and_filenames_reject_private_references(self):
        for name in NAMES + FILES:
            with self.subTest(name=name):
                self.write("probe.txt", b"prefix " + name.upper() + b" suffix\0")
                self.check(1)
                self.write("probe.txt", b"a host")
                path = self.write(os.fsdecode(name) + ".txt", b"a host")
                self.check(1)
                path.unlink()
                self.git("add", "-u")
        self.write("probe.txt", b"prepare-fill.js nest-f3-fill.js")
        self.check(0)

    def test_compound_identifiers_fail_and_untracked_files_are_not_scanned(self):
        self.write("probe.txt", b"prefix" + NAMES[0].upper() + b"Suffix")
        self.check(1)
        self.write("probe.txt", b"a host")
        (self.root / "untracked.txt").write_bytes(NAMES[0])
        self.check(0)

    def test_product_lines_allow_only_reviewed_content_at_the_reviewed_paths(self):
        for relative in PRODUCT_PATHS:
            self.write(relative, (REPO / relative).read_bytes())
        self.check(0)
        for name in NAMES + FILES:
            with self.subTest(name=name):
                path = self.root / PRODUCT_PATHS[2]
                original = (REPO / PRODUCT_PATHS[2]).read_bytes()
                path.write_bytes(original + b"\n" + name)
                self.check(1)
                path.write_bytes(original)

    def test_symlinks_scan_link_text_without_reading_the_target(self):
        outside = self.root / "untracked.txt"
        outside.write_bytes(NAMES[0])
        link = self.root / "link"
        link.symlink_to(outside)
        self.git("add", "link")
        self.check(0)
        link.unlink()
        link.symlink_to(os.fsdecode(NAMES[1]))
        self.check(1)

    def test_snapshot_requires_explicit_source_root_and_scans_supplied_files(self):
        (self.root / ".git").rename(self.root / "metadata")
        self.write_snapshot()
        env = {**os.environ, "NEO_SDK_SOURCE_ROOT": str(self.root)}
        self.check(0, env)
        (self.root / "packages/browseros-agent/probe.txt").write_bytes(NAMES[0])
        self.check(1, env)
        env.pop("NEO_SDK_SOURCE_ROOT")
        self.check(2, env)

    def write_snapshot(self):
        for relative in ("packages/browseros-agent/probe.txt", ".github/workflows/test.yml"):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"a host")


if __name__ == "__main__":
    unittest.main()
