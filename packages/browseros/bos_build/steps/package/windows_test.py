#!/usr/bin/env python3
"""Tests for the Windows packaging module (autoninja routing and validate())."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock

from . import windows
from ..compile import standard
from ...core.context import ArtifactRegistry, Context
from ...core.products import get_product_descriptor
from ...core.step import ValidationError


class BuildMiniInstallerTest(unittest.TestCase):
    def test_routes_through_shared_argv_builder_with_override(self):
        ctx = cast(
            Context,
            SimpleNamespace(
                out_dir="out/Default_x64", chromium_src=Path("/tmp/chromium-src")
            ),
        )
        with (
            mock.patch.object(windows, "run_command") as run_cmd,
            mock.patch.object(standard, "IS_WINDOWS", return_value=False),
            mock.patch("os.chdir"),
            mock.patch("os.getcwd", return_value="/anywhere"),
            mock.patch.dict("os.environ", {"BROWSEROS_NINJA_JOBS": "8"}, clear=True),
        ):
            result = windows.build_mini_installer(ctx)
        run_cmd.assert_called_once_with(
            ["autoninja", "-C", "out/Default_x64", "-j", "8", "setup", "mini_installer"]
        )
        # Artifacts were never produced (run_command is mocked), so it reports failure.
        self.assertFalse(result)

    def test_unsigned_step_leaves_product_alias_for_packaging(self):
        for product_id in ("browseros", "browserclaw"):
            with self.subTest(product=product_id), tempfile.TemporaryDirectory() as tmp:
                product = get_product_descriptor(product_id)
                build_output_dir = Path(tmp) / "out" / "Default"
                build_output_dir.mkdir(parents=True)
                chrome_path = build_output_dir / "chrome.exe"
                product_path = build_output_dir / f"{product.app_base_name}.exe"
                chrome_path.write_bytes(b"chrome")
                ctx = cast(
                    Context,
                    SimpleNamespace(
                        chromium_src=Path(tmp),
                        out_dir="out/Default",
                        get_app_path=lambda: product_path,
                    ),
                )

                with mock.patch.object(
                    windows, "build_mini_installer", return_value=True
                ):
                    windows.MiniInstallerModule().execute(ctx)

                self.assertEqual(chrome_path.read_bytes(), b"chrome")
                self.assertFalse(product_path.exists())


class WindowsExecutableFinalizationTest(unittest.TestCase):
    def test_product_alias_is_copied_after_installer_outputs(self):
        for product_id in ("browseros", "browserclaw"):
            with self.subTest(product=product_id), tempfile.TemporaryDirectory() as tmp:
                product = get_product_descriptor(product_id)
                root = Path(tmp)
                build_output_dir = root / "out" / "Default"
                build_output_dir.mkdir(parents=True)
                chrome_path = build_output_dir / "chrome.exe"
                product_path = build_output_dir / f"{product.app_base_name}.exe"
                chrome_path.write_bytes(b"signed chrome")
                product_path.write_bytes(b"stale product")
                registry = ArtifactRegistry()
                ctx = cast(
                    Context,
                    SimpleNamespace(
                        chromium_src=root,
                        out_dir="out/Default",
                        artifact_registry=registry,
                        get_chromium_app_path=lambda: chrome_path,
                        get_app_path=lambda: product_path,
                    ),
                )
                order = []
                installer_path = root / "dist" / "installer.exe"
                zip_path = root / "dist" / "installer.zip"

                def create_installer(_ctx):
                    self.assertEqual(product_path.read_bytes(), b"stale product")
                    order.append("installer")
                    return installer_path

                def create_zip(_ctx):
                    self.assertEqual(product_path.read_bytes(), b"stale product")
                    order.append("zip")
                    return zip_path

                module = windows.WindowsPackageModule()
                with (
                    mock.patch.object(
                        module, "_create_installer", side_effect=create_installer
                    ),
                    mock.patch.object(
                        module, "_create_portable_zip", side_effect=create_zip
                    ),
                ):
                    module.execute(ctx)

                self.assertEqual(order, ["installer", "zip"])
                self.assertEqual(chrome_path.read_bytes(), b"signed chrome")
                self.assertEqual(product_path.read_bytes(), b"signed chrome")
                self.assertEqual(registry.get("built_app"), product_path)
                self.assertEqual(registry.get("installer"), installer_path)
                self.assertEqual(registry.get("installer_zip"), zip_path)


class WindowsPackageModuleValidateTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.chromium_src = Path(self._tmp.name) / "chromium" / "src"
        self.out_dir = "out/Default_x64"
        self.build_output_dir = self.chromium_src / self.out_dir
        self.build_output_dir.mkdir(parents=True)
        self.ctx = cast(
            Context,
            SimpleNamespace(chromium_src=self.chromium_src, out_dir=self.out_dir),
        )

    def _touch_output(self, name: str) -> Path:
        path = self.build_output_dir / name
        path.write_text("")
        return path

    def test_validate_raises_when_winsparkle_dll_missing(self):
        self._touch_output("mini_installer.exe")
        winsparkle_path = self.build_output_dir / "WinSparkle.dll"

        with mock.patch.object(windows, "IS_WINDOWS", return_value=True):
            with self.assertRaises(ValidationError) as raised:
                windows.WindowsPackageModule().validate(self.ctx)

        message = str(raised.exception)
        self.assertIn("WinSparkle.dll", message)
        self.assertIn(str(winsparkle_path), message)
        self.assertIn("auto-update", message)

    def test_validate_passes_when_installer_and_winsparkle_dll_exist(self):
        self._touch_output("mini_installer.exe")
        self._touch_output("WinSparkle.dll")

        with mock.patch.object(windows, "IS_WINDOWS", return_value=True):
            windows.WindowsPackageModule().validate(self.ctx)

    def test_validate_raises_when_mini_installer_missing(self):
        self._touch_output("WinSparkle.dll")
        mini_installer_path = self.build_output_dir / "mini_installer.exe"

        with mock.patch.object(windows, "IS_WINDOWS", return_value=True):
            with self.assertRaises(ValidationError) as raised:
                windows.WindowsPackageModule().validate(self.ctx)

        self.assertEqual(
            str(raised.exception), f"mini_installer.exe not found: {mini_installer_path}"
        )

    def test_validate_raises_when_not_windows(self):
        with mock.patch.object(windows, "IS_WINDOWS", return_value=False):
            with self.assertRaisesRegex(
                ValidationError, "Windows packaging requires Windows"
            ):
                windows.WindowsPackageModule().validate(self.ctx)


if __name__ == "__main__":
    unittest.main()
