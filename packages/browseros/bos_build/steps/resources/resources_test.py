#!/usr/bin/env python3
"""Tests for copy_resources against a mock chromium checkout."""

import shutil
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest.mock import patch

import yaml
from .resources import ResourcesModule, copy_resources_impl
from ...core.context import Context
from ...core.step import ValidationError
from ...lib.testing import MockBrowserOSRoot, MockChromium, make_context
from ...lib.utils import get_platform


class CopyResourcesTest(unittest.TestCase):
    def setUp(self):
        self._chromium_tmp = tempfile.TemporaryDirectory()
        self._root_tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._chromium_tmp.cleanup)
        self.addCleanup(self._root_tmp.cleanup)
        self.chromium = MockChromium(Path(self._chromium_tmp.name))
        self.root = MockBrowserOSRoot(Path(self._root_tmp.name))
        self.ctx = make_context(
            self.chromium, self.root, architecture="x64", build_type="release"
        )

    def test_missing_config_raises(self):
        with self.assertRaises(FileNotFoundError):
            copy_resources_impl(self.ctx)

    def test_config_without_operations_is_noop(self):
        self.root.write_copy_config({"something_else": True})
        self.assertTrue(copy_resources_impl(self.ctx))

    def test_directory_operation_copies_tree(self):
        src_dir = self.root.root / "resources" / "icons"
        (src_dir / "nested").mkdir(parents=True)
        (src_dir / "app.png").write_text("png-bytes")
        (src_dir / "nested" / "small.png").write_text("small-bytes")
        self.root.write_copy_config(
            {
                "copy_operations": [
                    {
                        "name": "Icons",
                        "source": "resources/icons",
                        "destination": "chrome/app/theme/browseros",
                        "type": "directory",
                    }
                ]
            }
        )

        self.assertTrue(copy_resources_impl(self.ctx))

        dest = self.chromium.src / "chrome" / "app" / "theme" / "browseros"
        self.assertEqual((dest / "app.png").read_text(), "png-bytes")
        self.assertEqual((dest / "nested" / "small.png").read_text(), "small-bytes")

    def test_file_operation_copies_and_creates_parents(self):
        (self.root.root / "resources").mkdir(exist_ok=True)
        (self.root.root / "resources" / "logo.icns").write_text("icns")
        self.root.write_copy_config(
            {
                "copy_operations": [
                    {
                        "name": "Logo",
                        "source": "resources/logo.icns",
                        "destination": "chrome/app/theme/logo.icns",
                        "type": "file",
                    }
                ]
            }
        )

        self.assertTrue(copy_resources_impl(self.ctx))

        dest = self.chromium.src / "chrome" / "app" / "theme" / "logo.icns"
        self.assertEqual(dest.read_text(), "icns")

    def test_files_operation_copies_glob_matches(self):
        ext_dir = self.root.root / "resources" / "ext"
        ext_dir.mkdir(parents=True)
        (ext_dir / "a.js").write_text("a")
        (ext_dir / "b.js").write_text("b")
        (ext_dir / "ignore.txt").write_text("x")
        self.root.write_copy_config(
            {
                "copy_operations": [
                    {
                        "name": "Scripts",
                        "source": "resources/ext/*.js",
                        "destination": "chrome/browser/resources/browseros",
                        "type": "files",
                    }
                ]
            }
        )

        self.assertTrue(copy_resources_impl(self.ctx))

        dest = self.chromium.src / "chrome" / "browser" / "resources" / "browseros"
        self.assertEqual((dest / "a.js").read_text(), "a")
        self.assertEqual((dest / "b.js").read_text(), "b")
        self.assertFalse((dest / "ignore.txt").exists())

    def test_condition_mismatches_skip_operation(self):
        (self.root.root / "resources").mkdir(exist_ok=True)
        (self.root.root / "resources" / "skipped.txt").write_text("x")
        self.root.write_copy_config(
            {
                "copy_operations": [
                    {
                        "name": "Wrong build type",
                        "source": "resources/skipped.txt",
                        "destination": "chrome/one.txt",
                        "type": "file",
                        "build_type": "debug",
                    },
                    {
                        "name": "Wrong os",
                        "source": "resources/skipped.txt",
                        "destination": "chrome/two.txt",
                        "type": "file",
                        "os": ["never-os"],
                    },
                    {
                        "name": "Wrong arch",
                        "source": "resources/skipped.txt",
                        "destination": "chrome/three.txt",
                        "type": "file",
                        "arch": ["arm64"],
                    },
                ]
            }
        )

        self.assertTrue(copy_resources_impl(self.ctx))

        self.assertFalse((self.chromium.src / "chrome" / "one.txt").exists())
        self.assertFalse((self.chromium.src / "chrome" / "two.txt").exists())
        self.assertFalse((self.chromium.src / "chrome" / "three.txt").exists())

    def test_matching_conditions_run_operation(self):
        (self.root.root / "resources").mkdir(exist_ok=True)
        (self.root.root / "resources" / "kept.txt").write_text("kept")
        self.root.write_copy_config(
            {
                "copy_operations": [
                    {
                        "name": "Matches everything",
                        "source": "resources/kept.txt",
                        "destination": "chrome/kept.txt",
                        "type": "file",
                        "build_type": "release",
                        "os": [get_platform()],
                        "arch": ["x64"],
                    }
                ]
            }
        )

        self.assertTrue(copy_resources_impl(self.ctx))

        self.assertEqual(
            (self.chromium.src / "chrome" / "kept.txt").read_text(), "kept"
        )

    def test_real_config_copies_icons_for_active_product(self):
        self.root.write_copy_config(self._real_copy_config())
        for product_id, marker in (
            ("browseros", "browseros"),
            ("browserclaw", "claw"),
        ):
            icons = self.root.root / "resources" / product_id / "icons"
            (icons / "linux").mkdir(parents=True)
            (icons / "default_100_percent").mkdir(parents=True)
            (icons / "product_logo_16.png").write_text(f"{marker}-root")
            (icons / "linux" / "product_logo_24.png").write_text(f"{marker}-linux")
            (icons / "default_100_percent" / "product_logo_16.png").write_text(
                f"{marker}-dpi"
            )

        for product_id, marker in (
            ("browseros", "browseros"),
            ("browserclaw", "claw"),
        ):
            with self.subTest(product=product_id):
                ctx = make_context(
                    self.chromium,
                    self.root,
                    architecture="x64",
                    build_type="release",
                    product=product_id,
                )
                self.assertTrue(copy_resources_impl(ctx))

                theme = self.chromium.src / "chrome" / "app" / "theme"
                self.assertEqual(
                    (theme / "chromium" / "product_logo_16.png").read_text(),
                    f"{marker}-root",
                )
                self.assertEqual(
                    (
                        theme
                        / "chromium"
                        / "linux"
                        / "product_logo_24.png"
                    ).read_text(),
                    f"{marker}-linux",
                )
                self.assertEqual(
                    (
                        theme
                        / "default_100_percent"
                        / "chromium"
                        / "product_logo_16.png"
                    ).read_text(),
                    f"{marker}-dpi",
                )

    def test_missing_source_is_tolerated(self):
        self.root.write_copy_config(
            {
                "copy_operations": [
                    {
                        "name": "Ghost",
                        "source": "resources/missing-dir",
                        "destination": "chrome/ghost",
                        "type": "directory",
                    }
                ]
            }
        )

        self.assertTrue(copy_resources_impl(self.ctx))

        self.assertFalse((self.chromium.src / "chrome" / "ghost").exists())

    def test_real_config_copies_server_resources_for_browseros_product(self):
        self.root.write_copy_config(self._real_copy_config())
        browseros_source = (
            self.root.root
            / "resources"
            / "binaries"
            / "browseros_server"
            / "darwin-arm64"
            / "resources"
        )
        claw_source = (
            self.root.root
            / "resources"
            / "binaries"
            / "browseros_claw_server"
            / "darwin-arm64"
            / "resources"
        )
        claw_rust_source = (
            self.root.root
            / "resources"
            / "binaries"
            / "browseros_claw_server_rust"
            / "darwin-arm64"
            / "resources"
        )
        (browseros_source / "bin").mkdir(parents=True)
        (browseros_source / "bin" / "browseros_server").write_text("browseros")
        (claw_source / "bin").mkdir(parents=True)
        (claw_source / "bin" / "browseros-claw-server").write_text("claw")
        (claw_rust_source / "bin").mkdir(parents=True)
        (claw_rust_source / "bin" / "browseros-claw-server-rs").write_text(
            "claw-rust"
        )

        with patch(
            "bos_build.steps.resources.resources.get_platform",
            return_value="macos",
        ):
            ctx = make_context(
                self.chromium,
                self.root,
                architecture="arm64",
                build_type="release",
            )
            self.assertTrue(copy_resources_impl(ctx))

        browseros_dest = (
            self.chromium.src
            / "chrome"
            / "browser"
            / "browseros"
            / "server"
            / "resources"
            / "bin"
            / "browseros_server"
        )
        claw_dest = (
            self.chromium.src
            / "chrome"
            / "browser"
            / "browseros"
            / "claw_server"
            / "resources"
            / "bin"
            / "browseros-claw-server"
        )
        claw_rust_dest = (
            self.chromium.src
            / "chrome"
            / "browser"
            / "browseros"
            / "claw_server_rust"
            / "resources"
            / "bin"
            / "browseros-claw-server-rs"
        )
        self.assertEqual(browseros_dest.read_text(), "browseros")
        self.assertFalse(claw_dest.exists())
        self.assertFalse(claw_rust_dest.exists())

    def test_real_config_copies_rust_server_resources_for_browserclaw_by_default(
        self,
    ):
        self.root.write_copy_config(self._real_copy_config())
        browseros_source = (
            self.root.root
            / "resources"
            / "binaries"
            / "browseros_server"
            / "darwin-arm64"
            / "resources"
        )
        claw_source = (
            self.root.root
            / "resources"
            / "binaries"
            / "browseros_claw_server"
            / "darwin-arm64"
            / "resources"
        )
        claw_rust_source = (
            self.root.root
            / "resources"
            / "binaries"
            / "browseros_claw_server_rust"
            / "darwin-arm64"
            / "resources"
        )
        (browseros_source / "bin").mkdir(parents=True)
        (browseros_source / "bin" / "browseros_server").write_text("browseros")
        (claw_source / "bin").mkdir(parents=True)
        (claw_source / "bin" / "browseros-claw-server").write_text("claw")
        (claw_rust_source / "bin").mkdir(parents=True)
        (claw_rust_source / "bin" / "browseros-claw-server-rs").write_text(
            "claw-rust"
        )
        stale_ts_only_file = (
            self.chromium.src
            / "chrome"
            / "browser"
            / "browseros"
            / "claw_server"
            / "resources"
            / "bin"
            / "third_party"
            / "bun"
        )
        stale_ts_only_file.parent.mkdir(parents=True)
        stale_ts_only_file.write_text("stale")

        with patch(
            "bos_build.steps.resources.resources.get_platform",
            return_value="macos",
        ):
            ctx = make_context(
                self.chromium,
                self.root,
                architecture="arm64",
                build_type="release",
                product="browserclaw",
            )
            self.assertTrue(copy_resources_impl(ctx))

        browseros_dest = (
            self.chromium.src
            / "chrome"
            / "browser"
            / "browseros"
            / "server"
            / "resources"
            / "bin"
            / "browseros_server"
        )
        claw_dest = (
            self.chromium.src
            / "chrome"
            / "browser"
            / "browseros"
            / "claw_server"
            / "resources"
            / "bin"
            / "browseros-claw-server"
        )
        claw_rust_dest = (
            self.chromium.src
            / "chrome"
            / "browser"
            / "browseros"
            / "claw_server_rust"
            / "resources"
            / "bin"
            / "browseros-claw-server-rs"
        )
        self.assertEqual(browseros_dest.read_text(), "browseros")
        self.assertEqual(claw_dest.read_text(), "claw-rust")
        self.assertFalse(claw_rust_dest.exists())
        self.assertFalse(stale_ts_only_file.exists())

    def test_real_config_keeps_typescript_claw_server_switch_block_commented(
        self,
    ):
        config_path = (
            Path(__file__).resolve().parents[2] / "config" / "copy_resources.yaml"
        )
        text = config_path.read_text()
        config = self._real_copy_config()
        active_names = [op["name"] for op in config["copy_operations"]]

        self.assertIn(
            "# BrowserOS Claw Server resources - switch variants by swapping which block is commented.",
            text,
        )
        self.assertIn(
            '# - name: "BrowserOS Claw Server Resources - macOS ARM64"',
            text,
        )
        self.assertIn(
            '#   destination: "chrome/browser/browseros/claw_server/resources"',
            text,
        )
        self.assertNotIn(
            "BrowserOS Claw Server Resources - macOS ARM64",
            active_names,
        )
        self.assertIn(
            "BrowserOS Claw Rust Server Resources - macOS ARM64",
            active_names,
        )

    def test_real_config_copies_claw_onboard_resources_for_both_products(self):
        # The downloaded onboarding dist must land in the grit resources dir
        # for every product, since the onboarding pak builds unconditionally.
        self.root.write_copy_config(self._real_copy_config())
        onboard_source = (
            self.root.root / "resources" / "binaries" / "browseros_claw_onboard" / "resources"
        )
        (onboard_source / "icon").mkdir(parents=True)
        (onboard_source / "index.html").write_text("<html>onboard</html>")
        (onboard_source / "icon" / "32.png").write_text("icon-bytes")

        onboard_dest = (
            self.chromium.src / "chrome" / "browser" / "browseros" / "onboarding" / "resources"
        )

        for product in ("browseros", "browserclaw"):
            with self.subTest(product=product):
                if onboard_dest.exists():
                    shutil.rmtree(onboard_dest)

                with patch(
                    "bos_build.steps.resources.resources.get_platform",
                    return_value="macos",
                ):
                    ctx = make_context(
                        self.chromium,
                        self.root,
                        architecture="arm64",
                        build_type="release",
                        product=product,
                    )
                    self.assertTrue(copy_resources_impl(ctx))

                self.assertEqual(
                    (onboard_dest / "index.html").read_text(), "<html>onboard</html>"
                )
                self.assertEqual(
                    (onboard_dest / "icon" / "32.png").read_text(), "icon-bytes"
                )

    def _real_copy_config(self) -> dict:
        config_path = (
            Path(__file__).resolve().parents[2] / "config" / "copy_resources.yaml"
        )
        with open(config_path, "r") as f:
            return yaml.safe_load(f)


class ResourcesModuleValidateTest(unittest.TestCase):
    def test_missing_copy_config_raises_validation_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            ctx = cast(
                Context,
                SimpleNamespace(
                    get_copy_resources_config=lambda: Path(tmp) / "missing.yaml"
                ),
            )
            with self.assertRaises(ValidationError):
                ResourcesModule().validate(ctx)


if __name__ == "__main__":
    unittest.main()
