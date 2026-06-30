#!/usr/bin/env python3
"""Tests for the mock chromium checkout fixture."""

import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

from .testing import MockBrowserOSRoot, MockChromium, make_context


class MockChromiumTest(unittest.TestCase):
    def test_baseline_markers_created(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))

            gclient = Path(tmp) / ".gclient"
            self.assertTrue(gclient.exists())
            self.assertIn("solutions", gclient.read_text())
            self.assertIn('"src"', gclient.read_text())

            version_file = m.src / "chrome" / "VERSION"
            self.assertTrue(version_file.exists())
            version = dict(
                line.split("=") for line in version_file.read_text().strip().split("\n")
            )
            self.assertEqual(version["MAJOR"], "137")
            self.assertEqual(version["MINOR"], "0")
            self.assertEqual(version["BUILD"], "7151")
            self.assertEqual(version["PATCH"], "69")

            self.assertTrue((m.src / "BUILD.gn").exists())

    def test_src_is_under_gclient_root(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            self.assertEqual(m.src, Path(tmp) / "src")
            self.assertEqual(m.src.parent / ".gclient", Path(tmp) / ".gclient")

    def test_add_file_creates_parents_and_content(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            written = m.add_file("chrome/browser/foo/bar.cc", "int x = 1;\n")
            self.assertEqual(written, m.src / "chrome" / "browser" / "foo" / "bar.cc")
            self.assertEqual(written.read_text(), "int x = 1;\n")

    def test_with_out_dir_creates_arch_dir_and_args_gn(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            out = m.with_out_dir("arm64", args_gn='is_debug = false\n')
            self.assertEqual(out, m.src / "out" / "Default_browseros_arm64")
            self.assertTrue(out.is_dir())
            self.assertEqual((out / "args.gn").read_text(), "is_debug = false\n")

    def test_with_out_dir_without_args_gn(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            out = m.with_out_dir("x64")
            self.assertTrue(out.is_dir())
            self.assertFalse((out / "args.gn").exists())

    def test_with_git_creates_repo_with_initial_commit(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp)).with_git()
            self.assertTrue((m.src / ".git").is_dir())
            head = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=m.src,
                capture_output=True,
                text=True,
            )
            self.assertEqual(head.returncode, 0, head.stderr)
            self.assertEqual(len(head.stdout.strip()), 40)

    def test_commit_all_returns_hash_listing_added_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp)).with_git()
            m.add_file("chrome/new_file.txt", "hello\n")
            commit = m.commit_all("add new file")
            self.assertEqual(len(commit), 40)

            shown = subprocess.run(
                ["git", "diff-tree", "--no-commit-id", "--name-only", "-r", commit],
                cwd=m.src,
                capture_output=True,
                text=True,
            )
            self.assertIn("chrome/new_file.txt", shown.stdout)

    def test_commit_all_allow_empty(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp)).with_git()
            commit = m.commit_all("empty", allow_empty=True)
            self.assertEqual(len(commit), 40)

    def test_with_sparkle_and_pkg_dmg_markers(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            sparkle = m.with_sparkle()
            pkg_dmg = m.with_pkg_dmg()
            self.assertEqual(sparkle, m.src / "third_party" / "sparkle")
            self.assertTrue(sparkle.is_dir())
            self.assertEqual(
                pkg_dmg, m.src / "chrome" / "installer" / "mac" / "pkg-dmg"
            )
            self.assertTrue(pkg_dmg.is_file())

    def test_with_branding_files_contains_replaceable_strings(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            m.with_branding_files()
            grd = m.src / "chrome" / "app" / "chromium_strings.grd"
            grdp = m.src / "chrome" / "app" / "settings_chromium_strings.grdp"
            self.assertTrue(grd.exists())
            self.assertTrue(grdp.exists())
            self.assertIn("Google Chrome", grd.read_text())
            self.assertIn("Chromium", grd.read_text())


class MockBrowserOSRootTest(unittest.TestCase):
    def test_version_files_written(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = MockBrowserOSRoot(Path(tmp))
            self.assertIn("MAJOR=137", (r.root / "CHROMIUM_VERSION").read_text())
            self.assertEqual(
                (r.root / "build" / "config" / "BROWSEROS_BUILD_OFFSET")
                .read_text()
                .strip(),
                "80",
            )
            browseros_version = (
                r.root / "resources" / "BROWSEROS_VERSION"
            ).read_text()
            self.assertIn("BROWSEROS_MAJOR=0", browseros_version)
            self.assertIn("BROWSEROS_MINOR=31", browseros_version)

    def test_add_patch_and_replacement_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = MockBrowserOSRoot(Path(tmp))
            patch = r.add_patch("chrome/foo.cc.patch", "--- a/x\n")
            repl = r.add_replacement_file("chrome/bar.h", "// custom\n")
            self.assertEqual(
                patch, r.root / "chromium_patches" / "chrome" / "foo.cc.patch"
            )
            self.assertEqual(patch.read_text(), "--- a/x\n")
            self.assertEqual(
                repl,
                r.root
                / "chromium_files"
                / "products"
                / "browseros"
                / "chrome"
                / "bar.h",
            )
            self.assertEqual(repl.read_text(), "// custom\n")

    def test_write_features_yaml_roundtrip(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = MockBrowserOSRoot(Path(tmp))
            features = {
                "llm-chat": {
                    "description": "feat: LLM chat",
                    "files": ["chrome/a.cc"],
                }
            }
            path = r.write_features_yaml(features)
            self.assertEqual(path, r.root / "build" / "features.yaml")
            loaded = yaml.safe_load(path.read_text())
            self.assertEqual(loaded["features"], features)

    def test_write_copy_config_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = MockBrowserOSRoot(Path(tmp))
            path = r.write_copy_config({"copy_operations": []})
            self.assertEqual(path, r.root / "build" / "config" / "copy_resources.yaml")
            self.assertEqual(yaml.safe_load(path.read_text()), {"copy_operations": []})

    def test_write_gn_flags_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = MockBrowserOSRoot(Path(tmp))
            path = r.write_gn_flags("macos", "release", "is_debug = false\n")
            self.assertEqual(
                path,
                r.root / "build" / "config" / "gn" / "flags.macos.release.gn",
            )
            self.assertEqual(path.read_text(), "is_debug = false\n")


class MakeContextTest(unittest.TestCase):
    def test_context_loads_versions_from_mocks(self):
        with (
            tempfile.TemporaryDirectory() as chromium_tmp,
            tempfile.TemporaryDirectory() as root_tmp,
        ):
            m = MockChromium(Path(chromium_tmp))
            r = MockBrowserOSRoot(Path(root_tmp))
            ctx = make_context(m, r, architecture="x64", build_type="release")

            self.assertEqual(ctx.chromium_src, m.src)
            self.assertEqual(ctx.root_dir, r.root)
            self.assertEqual(ctx.architecture, "x64")
            self.assertEqual(ctx.build_type, "release")
            self.assertEqual(ctx.chromium_version, "137.0.7151.69")
            # BUILD 7151 + offset 80 = 7231
            self.assertEqual(ctx.browseros_chromium_version, "137.0.7231.69")
            self.assertEqual(ctx.semantic_version, "0.31.0")
            self.assertEqual(
                ctx.get_features_yaml_path(), r.root / "build" / "features.yaml"
            )
            self.assertEqual(
                ctx.get_patches_dir(), r.root / "chromium_patches"
            )


if __name__ == "__main__":
    unittest.main()
