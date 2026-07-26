#!/usr/bin/env python3
"""Tests for Linux packaging architecture helpers."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest.mock import patch

from bos_build.core.context import Context
from bos_build.core.products import get_product_descriptor
from bos_build.steps.package.linux import (
    LINUX_HOST_APPIMAGETOOL,
    appimage_icon_source,
    copy_browser_files,
    copy_icon,
    create_apparmor_profile,
    create_desktop_file,
    create_launcher_script,
    create_metainfo_file,
    get_host_appimagetool,
    get_linux_architecture_config,
)


class LinuxArchitectureConfigTest(unittest.TestCase):
    def test_returns_x64_packaging_config(self) -> None:
        config = get_linux_architecture_config("x64")

        self.assertEqual(config["appimage_arch"], "x86_64")
        self.assertEqual(config["deb_arch"], "amd64")

    def test_returns_arm64_packaging_config(self) -> None:
        config = get_linux_architecture_config("arm64")

        self.assertEqual(config["appimage_arch"], "aarch64")
        self.assertEqual(config["deb_arch"], "arm64")

    def test_rejects_unsupported_architecture(self) -> None:
        with self.assertRaisesRegex(ValueError, "Unsupported Linux architecture"):
            get_linux_architecture_config("universal")


class HostAppImageToolTest(unittest.TestCase):
    """The appimagetool binary must match the BUILD machine's arch, not
    the target arch — otherwise cross-compiling arm64 packages from an x64
    host fails because the aarch64 tool can't execute on x64."""

    def test_x64_host_picks_x86_64_tool(self) -> None:
        with patch(
            "bos_build.steps.package.linux.get_platform_arch", return_value="x64"
        ):
            filename, url = get_host_appimagetool()

        self.assertEqual(filename, "appimagetool-x86_64.AppImage")
        self.assertIn("x86_64", url)

    def test_arm64_host_picks_aarch64_tool(self) -> None:
        with patch(
            "bos_build.steps.package.linux.get_platform_arch", return_value="arm64"
        ):
            filename, url = get_host_appimagetool()

        self.assertEqual(filename, "appimagetool-aarch64.AppImage")
        self.assertIn("aarch64", url)

    def test_host_lookup_independent_of_target(self) -> None:
        # Both architectures must be present in the host lookup so cross
        # builds work in either direction.
        self.assertIn("x64", LINUX_HOST_APPIMAGETOOL)
        self.assertIn("arm64", LINUX_HOST_APPIMAGETOOL)


class CopyBrowserFilesTest(unittest.TestCase):
    def test_copies_required_extensions_beside_browser_for_both_products(
        self,
    ) -> None:
        for product_id in ("browseros", "browserclaw"):
            with self.subTest(product_id=product_id), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                out_dir = root / "out" / "Release"
                target_dir = root / "package"
                extensions_dir = out_dir / "browseros_extensions"
                extensions_dir.mkdir(parents=True)
                (out_dir / product_id).write_text("browser")
                (extensions_dir / "bundled_extensions.json").write_text("{}")
                (extensions_dir / f"{product_id}.crx").write_text(product_id)
                resources_dir = out_dir / "resources"
                resources_dir.mkdir()
                (resources_dir / "not-packaged.txt").write_text("chromium")

                ctx = cast(
                    Context,
                    SimpleNamespace(
                        chromium_src=root,
                        out_dir=Path("out") / "Release",
                        BROWSEROS_APP_NAME=product_id,
                        product=get_product_descriptor(product_id),
                    ),
                )

                with patch("bos_build.steps.package.linux.log_warning"):
                    self.assertTrue(copy_browser_files(ctx, target_dir))

                self.assertEqual((target_dir / product_id).read_text(), "browser")
                self.assertEqual(
                    (
                        target_dir
                        / "browseros_extensions"
                        / f"{product_id}.crx"
                    ).read_text(),
                    product_id,
                )
                self.assertFalse((target_dir / "resources").exists())

    def test_missing_required_extensions_raises_precise_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            out_dir = root / "out" / "Release"
            target_dir = root / "package"
            out_dir.mkdir(parents=True)
            (out_dir / "browseros").write_text("browser")
            expected_dir = out_dir / "browseros_extensions"
            ctx = cast(
                Context,
                SimpleNamespace(
                    chromium_src=root,
                    out_dir=Path("out") / "Release",
                    BROWSEROS_APP_NAME="browseros",
                    product=get_product_descriptor("browseros"),
                ),
            )

            with self.assertRaises(FileNotFoundError) as raised:
                copy_browser_files(ctx, target_dir)

            self.assertEqual(
                str(raised.exception),
                f"Required bundled extensions directory not found at {expected_dir}; "
                f"expected browseros_extensions in Chromium output directory {out_dir}",
            )
            self.assertFalse(target_dir.exists())

    def test_copies_browseros_and_claw_server_roots_when_present(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            out_dir = root / "out" / "Release"
            target_dir = root / "package"
            out_dir.mkdir(parents=True)
            (out_dir / "browseros").write_text("browser")
            extensions_dir = out_dir / "browseros_extensions"
            extensions_dir.mkdir()
            (extensions_dir / "bundled_extensions.json").write_text("{}")

            for bundle_name in ("BrowserOSServer", "BrowserClawServer"):
                resources = out_dir / bundle_name / "default" / "resources"
                resources.mkdir(parents=True)
                (resources / "marker.txt").write_text(bundle_name)

            ctx = cast(
                Context,
                SimpleNamespace(
                    chromium_src=root,
                    out_dir=Path("out") / "Release",
                    BROWSEROS_APP_NAME="browseros",
                ),
            )

            with patch("bos_build.steps.package.linux.log_warning"):
                self.assertTrue(copy_browser_files(ctx, target_dir))

            self.assertTrue(
                (
                    target_dir
                    / "BrowserOSServer"
                    / "default"
                    / "resources"
                    / "marker.txt"
                ).exists()
            )
            self.assertTrue(
                (
                    target_dir
                    / "BrowserClawServer"
                    / "default"
                    / "resources"
                    / "marker.txt"
                ).exists()
            )

    def test_product_context_copies_only_owned_server_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            out_dir = root / "out" / "Release"
            target_dir = root / "package"
            out_dir.mkdir(parents=True)
            (out_dir / "browserclaw").write_text("browser")
            extensions_dir = out_dir / "browseros_extensions"
            extensions_dir.mkdir()
            (extensions_dir / "bundled_extensions.json").write_text("{}")

            for bundle_name in ("BrowserOSServer", "BrowserClawServer"):
                resources = out_dir / bundle_name / "default" / "resources"
                resources.mkdir(parents=True)
                (resources / "marker.txt").write_text(bundle_name)

            ctx = cast(
                Context,
                SimpleNamespace(
                    chromium_src=root,
                    out_dir=Path("out") / "Release",
                    BROWSEROS_APP_NAME="browserclaw",
                    product=get_product_descriptor("browserclaw"),
                ),
            )

            with patch("bos_build.steps.package.linux.log_warning"):
                self.assertTrue(copy_browser_files(ctx, target_dir))

            self.assertFalse((target_dir / "BrowserOSServer").exists())
            self.assertTrue(
                (
                    target_dir
                    / "BrowserClawServer"
                    / "default"
                    / "resources"
                    / "marker.txt"
                ).exists()
            )


class LinuxProductMetadataTest(unittest.TestCase):
    def test_helpers_use_product_package_names(self) -> None:
        for product_id in ("browseros", "browserclaw"):
            with self.subTest(product_id=product_id), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                product = get_product_descriptor(product_id)
                ctx = cast(
                    Context,
                    SimpleNamespace(
                        product=product,
                        BROWSEROS_APP_NAME=product_id,
                        get_browseros_chromium_version=lambda: "137.0.7231.69",
                    ),
                )

                desktop = create_desktop_file(
                    ctx, root / "applications", f"/usr/bin/{product_id}"
                )
                create_launcher_script(ctx, root / "bin")
                create_apparmor_profile(ctx, root / "apparmor")
                create_metainfo_file(ctx, root / "metainfo")

                desktop_content = desktop.read_text()
                self.assertEqual(desktop.name, f"{product_id}.desktop")
                self.assertIn(f"Name={product.display_name}", desktop_content)
                self.assertIn(f"Icon={product_id}", desktop_content)
                self.assertTrue((root / "bin" / product_id).exists())
                self.assertTrue((root / "apparmor" / product_id).exists())
                self.assertTrue(
                    (root / "metainfo" / f"{product_id}.metainfo.xml").exists()
                )

    def test_browserclaw_icons_copy_from_product_resource_tree(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            icon_src = root / "resources" / "browserclaw" / "icons"
            icon_src.mkdir(parents=True)
            (icon_src / "product_logo_16.png").write_text("browserclaw-16")
            browseros_icons = root / "resources" / "browseros" / "icons"
            browseros_icons.mkdir(parents=True)
            (browseros_icons / "product_logo_16.png").write_text("browseros-16")
            ctx = cast(
                Context,
                SimpleNamespace(
                    root_dir=root,
                    product=get_product_descriptor("browserclaw"),
                ),
            )

            self.assertTrue(copy_icon(ctx, root / "hicolor"))

            dest = root / "hicolor" / "16x16" / "apps" / "browserclaw.png"
            self.assertEqual(dest.read_text(), "browserclaw-16")

    def test_appimage_icon_source_uses_active_product_fallback(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            icon_dir = root / "resources" / "browserclaw" / "icons"
            icon_dir.mkdir(parents=True)
            fallback = icon_dir / "product_logo.png"
            fallback.write_text("fallback")
            ctx = cast(
                Context,
                SimpleNamespace(
                    root_dir=root,
                    product=get_product_descriptor("browserclaw"),
                ),
            )

            self.assertEqual(appimage_icon_source(ctx), fallback)

            preferred = icon_dir / "product_logo_256.png"
            preferred.write_text("preferred")
            self.assertEqual(appimage_icon_source(ctx), preferred)


if __name__ == "__main__":
    unittest.main()
