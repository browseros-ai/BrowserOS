#!/usr/bin/env python3
"""Tests for copy_resources against a mock chromium checkout."""

import hashlib
import json
import shutil
import tempfile
import unittest
import zipfile
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest.mock import patch

import yaml
from .resources import ResourcesModule, copy_resources_impl, stage_prepared_onboarding
from ...core.context import Context
from ...core.step import ValidationError
from ...lib.testing import MockBrowserOSRoot, MockChromium, make_context
from ...lib.utils import get_platform
from ...release.prepared_resources import PreparedFile, PreparedResourcesManifest


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

    def test_optional_rename_skips_when_target_already_exists(self):
        src_dir = self.root.root / "resources" / "server"
        (src_dir / "bin").mkdir(parents=True)
        (src_dir / "bin" / "browseros-claw-server").write_text("canonical")
        self.root.write_copy_config(
            {
                "copy_operations": [
                    {
                        "name": "Rust Claw",
                        "source": "resources/server",
                        "destination": "chrome/server",
                        "type": "directory",
                        "renames": [
                            {
                                "from": "bin/browseros-claw-server-rs",
                                "to": "bin/browseros-claw-server",
                                "optional": True,
                            }
                        ],
                    }
                ]
            }
        )

        self.assertTrue(copy_resources_impl(self.ctx))

        dest = self.chromium.src / "chrome" / "server" / "bin"
        self.assertEqual((dest / "browseros-claw-server").read_text(), "canonical")

    def test_optional_rename_normalizes_legacy_source(self):
        src_dir = self.root.root / "resources" / "server"
        (src_dir / "bin").mkdir(parents=True)
        (src_dir / "bin" / "browseros-claw-server-rs").write_text("legacy")
        self.root.write_copy_config(
            {
                "copy_operations": [
                    {
                        "name": "Rust Claw",
                        "source": "resources/server",
                        "destination": "chrome/server",
                        "type": "directory",
                        "renames": [
                            {
                                "from": "bin/browseros-claw-server-rs",
                                "to": "bin/browseros-claw-server",
                                "optional": True,
                            }
                        ],
                    }
                ]
            }
        )

        self.assertTrue(copy_resources_impl(self.ctx))

        dest = self.chromium.src / "chrome" / "server" / "bin"
        self.assertEqual((dest / "browseros-claw-server").read_text(), "legacy")
        self.assertFalse((dest / "browseros-claw-server-rs").exists())

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

        with patch(
            "bos_build.steps.resources.resources.get_platform",
            return_value="macos",
        ):
            for product_id, marker in (
                ("browseros", "browseros"),
                ("browserclaw", "claw"),
            ):
                with self.subTest(product=product_id):
                    self._seed_required_resources(product_id, "x64")
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
                            theme / "chromium" / "linux" / "product_logo_24.png"
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

    def test_required_missing_source_fails(self):
        self.root.write_copy_config(
            {
                "copy_operations": [
                    {
                        "name": "Required server",
                        "source": "resources/missing-server",
                        "destination": "chrome/server",
                        "type": "directory",
                        "required": True,
                    }
                ]
            }
        )

        self.assertFalse(copy_resources_impl(self.ctx))

    def test_prepared_onboarding_replaces_managed_source_directory(self):
        prepared_root = self.root.root / "prepared"
        prepared_root.mkdir()
        archive = prepared_root / "onboarding.zip"
        content = b"current-onboarding"
        metadata = {
            "version": "0.0.12",
            "target": "universal",
            "files": [
                {
                    "path": "resources/index.html",
                    "size": len(content),
                    "sha256": hashlib.sha256(content).hexdigest(),
                }
            ],
        }
        with zipfile.ZipFile(archive, "w") as bundle:
            bundle.writestr("artifact-metadata.json", json.dumps(metadata))
            bundle.writestr("resources/index.html", content)
        prepared_file = PreparedFile(
            path="onboarding.zip",
            size=archive.stat().st_size,
            sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),
            version="0.0.12",
        )
        manifest = PreparedResourcesManifest(
            product="browseros",
            parent_sha="1" * 40,
            source_sha="2" * 40,
            browser_version="0.0.1",
            component_versions={"app-onboard": "0.0.12"},
            files={"onboarding": prepared_file},
        )
        destination = self.root.root / "resources/binaries/browseros_onboarding"
        destination.mkdir(parents=True)
        (destination / "stale").write_text("stale")
        self.ctx.resource_mode = "source"
        self.ctx.source_sha = "2" * 40
        self.ctx.prepared_resources = prepared_root

        with patch(
            "bos_build.steps.resources.resources.validated_common_resources",
            return_value=manifest,
        ):
            staged = stage_prepared_onboarding(self.ctx)

        self.assertEqual((staged / "resources/index.html").read_bytes(), content)
        self.assertFalse((staged / "stale").exists())

    def test_real_config_copies_only_the_active_browseros_server(self):
        self.root.write_copy_config(self._real_copy_config())
        self._seed_required_resources("browseros", "arm64")
        stale = (
            self.chromium.src
            / "chrome/browser/browseros/claw_server/resources/bin/stale"
        )
        stale.parent.mkdir(parents=True)
        stale.write_text("stale")

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

        binary = (
            self.chromium.src
            / "chrome/browser/browseros/server/resources/bin/browseros_server"
        )
        self.assertEqual(binary.read_text(), "browseros")
        self.assertFalse(stale.exists())

    def test_real_config_copies_only_the_active_browserclaw_server(self):
        self.root.write_copy_config(self._real_copy_config())
        self._seed_required_resources("browserclaw", "arm64")
        stale = (
            self.chromium.src / "chrome/browser/browseros/server/resources/bin/stale"
        )
        stale.parent.mkdir(parents=True)
        stale.write_text("stale")

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

        binary = (
            self.chromium.src
            / "chrome/browser/browseros/claw_server/resources/bin/browseros-claw-server"
        )
        self.assertEqual(binary.read_text(), "browserclaw")
        self.assertFalse(stale.exists())

    def test_real_config_uses_rust_claw_server_resources(
        self,
    ):
        config = self._real_copy_config()
        active_names = [op["name"] for op in config["copy_operations"]]

        self.assertIn(
            "BrowserOS Claw Rust Server Resources - macOS ARM64",
            active_names,
        )
        self.assertNotIn(
            "BrowserOS Claw Server Resources - macOS ARM64",
            active_names,
        )

    def test_real_config_marks_servers_managed_required_and_product_owned(self):
        config = self._real_copy_config()
        server_ops = [
            op
            for op in config["copy_operations"]
            if op["name"].startswith("BrowserOS Server Resources")
            or op["name"].startswith("BrowserOS Claw Server Resources")
            or op["name"].startswith("BrowserOS Claw Rust Server Resources")
        ]

        self.assertTrue(server_ops)
        for op in server_ops:
            with self.subTest(name=op["name"]):
                self.assertIn(op["product"], ("browseros", "browserclaw"))
                self.assertTrue(op["managed"])
                self.assertTrue(op["required"])

    def test_real_config_copies_neutral_onboarding_for_each_product(self):
        self.root.write_copy_config(self._real_copy_config())
        onboard_source = (
            self.root.root
            / "resources"
            / "binaries"
            / "browseros_onboarding"
            / "resources"
        )
        onboard_dest = (
            self.chromium.src
            / "chrome"
            / "browser"
            / "browseros"
            / "onboarding"
            / "resources"
        )

        for product in ("browseros", "browserclaw"):
            with self.subTest(product=product):
                if onboard_source.exists():
                    shutil.rmtree(onboard_source)
                (onboard_source / "icon").mkdir(parents=True)
                (onboard_source / "index.html").write_text(
                    f"<html>{product}</html>"
                )
                (onboard_source / "icon" / "32.png").write_text(
                    f"{product}-icon-bytes"
                )
                self._seed_required_resources(product, "arm64")
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
                    (onboard_dest / "index.html").read_text(),
                    f"<html>{product}</html>",
                )
                self.assertEqual(
                    (onboard_dest / "icon" / "32.png").read_text(),
                    f"{product}-icon-bytes",
                )

    def test_real_config_has_one_required_neutral_onboarding_copy(self):
        config = self._real_copy_config()
        operations = [
            operation
            for operation in config["copy_operations"]
            if operation["destination"]
            == "chrome/browser/browseros/onboarding/resources"
        ]

        self.assertEqual(1, len(operations))
        self.assertEqual(
            "resources/binaries/browseros_onboarding/resources",
            operations[0]["source"],
        )
        self.assertTrue(operations[0]["managed"])
        self.assertTrue(operations[0]["required"])
        self.assertNotIn("product", operations[0])

    def test_real_config_debug_needs_only_neutral_onboarding_directory(self):
        self.root.write_copy_config(self._real_copy_config())
        self._seed_required_resources("browseros", "arm64")
        self._seed_required_resources("browserclaw", "arm64")
        source = (
            self.root.root
            / "resources/binaries/browseros_onboarding/resources/index.html"
        )
        source.write_text("debug-onboarding")

        with patch(
            "bos_build.steps.resources.resources.get_platform",
            return_value="macos",
        ):
            ctx = make_context(
                self.chromium,
                self.root,
                architecture="arm64",
                build_type="debug",
                product="browseros",
            )
            self.assertTrue(copy_resources_impl(ctx))

        destination = (
            self.chromium.src
            / "chrome/browser/browseros/onboarding/resources/index.html"
        )
        self.assertEqual("debug-onboarding", destination.read_text())

    def _real_copy_config(self) -> dict:
        config_path = (
            Path(__file__).resolve().parents[2] / "config" / "copy_resources.yaml"
        )
        with open(config_path, "r") as f:
            return yaml.safe_load(f)

    def _seed_required_resources(self, product: str, architecture: str) -> None:
        family, binary = (
            ("browseros_server", "browseros_server")
            if product == "browseros"
            else ("browseros_claw_server_rust", "browseros-claw-server")
        )
        server = (
            self.root.root
            / "resources"
            / "binaries"
            / family
            / f"darwin-{architecture}"
            / "resources/bin"
        )
        server.mkdir(parents=True, exist_ok=True)
        (server / binary).write_text(product)
        onboarding = (
            self.root.root / "resources/binaries/browseros_onboarding/resources"
        )
        onboarding.mkdir(parents=True, exist_ok=True)
        index = onboarding / "index.html"
        if not index.exists():
            index.write_text("onboarding")


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
