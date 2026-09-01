#!/usr/bin/env python3
"""Compatibility tests for the registered Linux packaging step."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from bos_build.core.step import ValidationError
from bos_build.steps.package.linux import (
    LinuxPackageModule,
    get_linux_architecture_config,
)
from bos_build.steps.package.linux_packaging import LinuxArtifactPair


class LinuxPackageStepTest(unittest.TestCase):
    def test_target_architecture_names_remain_distinct_from_host_tools(self) -> None:
        self.assertEqual(
            get_linux_architecture_config("x64"),
            {"appimage_arch": "x86_64", "deb_arch": "amd64"},
        )
        self.assertEqual(
            get_linux_architecture_config("arm64"),
            {"appimage_arch": "aarch64", "deb_arch": "arm64"},
        )

    def test_unknown_target_architecture_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "Unsupported Linux architecture"):
            get_linux_architecture_config("riscv64")

    def test_validate_requires_the_product_browser_binary(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ctx = SimpleNamespace(
                architecture="x64",
                chromium_src=Path(tmp),
                out_dir="out/Default_browseros_x64",
                BROWSEROS_APP_NAME="browseros",
            )
            with (
                mock.patch("bos_build.steps.package.linux.IS_LINUX", return_value=True),
                self.assertRaisesRegex(ValidationError, "Browser binary not found"),
            ):
                LinuxPackageModule().validate(ctx)

    def test_execute_delegates_to_the_complete_pair_interface(self) -> None:
        pair = LinuxArtifactPair(Path("BrowserOS.AppImage"), Path("BrowserOS.deb"))
        ctx = SimpleNamespace(
            BROWSEROS_APP_BASE_NAME="BrowserOS",
            architecture="x64",
            get_browseros_chromium_version=lambda: "136.0.0.0.1",
        )
        with mock.patch(
            "bos_build.steps.package.linux.build_linux_artifacts",
            return_value=pair,
        ) as build:
            LinuxPackageModule().execute(ctx)

        build.assert_called_once_with(ctx)
        self.assertEqual(LinuxPackageModule.produces, ["appimage", "deb"])


if __name__ == "__main__":
    unittest.main()
