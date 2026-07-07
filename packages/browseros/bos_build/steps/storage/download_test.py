#!/usr/bin/env python3
"""Tests for BrowserOS resource artifact downloads."""

import hashlib
import json
import os
import stat
import tempfile
import unittest
import zipfile
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest.mock import patch

import yaml
from bos_build.core.context import Context
from bos_build.core.products import get_product_descriptor
from bos_build.steps.storage.download import (
    ARTIFACT_METADATA_NAME,
    DownloadResourcesModule,
    extract_artifact_zip,
)


class ExtractArtifactZipTest(unittest.TestCase):
    def test_extracts_declared_files_and_writes_metadata(self) -> None:
        executable_files = {
            "resources/bin/browseros_server": b"server-binary",
            "resources/bin/third_party/lima/bin/limactl": b"limactl-binary",
            "resources/bin/third_party/linux/helper": b"linux-helper",
        }
        data_files = {
            (
                "resources/bin/third_party/lima/share/lima/"
                "lima-guestagent.Linux-aarch64.gz"
            ): b"guest-agent",
            "resources/vm/browseros-vm.yaml": b"vm-template",
        }
        files = executable_files | data_files

        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            archive_path = temp_path / "artifact.zip"
            destination = temp_path / "output"
            self._write_artifact_zip(
                archive_path,
                files,
                file_modes={
                    relative_path: 0o755 for relative_path in executable_files
                }
                | {relative_path: 0o644 for relative_path in data_files},
            )

            extracted_paths = extract_artifact_zip(archive_path, destination)

            self.assertEqual(len(extracted_paths), len(files))
            metadata_path = destination / ARTIFACT_METADATA_NAME
            self.assertTrue(metadata_path.exists())

            for relative_path, content in files.items():
                extracted_path = destination / relative_path
                self.assertEqual(extracted_path.read_bytes(), content)

                if os.name != "nt":
                    mode = os.stat(extracted_path).st_mode
                    if relative_path in data_files:
                        self.assertFalse(
                            mode & stat.S_IXUSR,
                            f"{relative_path} should not be executable",
                        )
                        continue

                    self.assertTrue(
                        mode & stat.S_IXUSR,
                        f"{relative_path} should be executable",
                    )

    def test_extracts_zip_members_without_unix_modes(self) -> None:
        files = {
            "resources/bin/browseros_server": b"server-binary",
        }

        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            archive_path = temp_path / "artifact.zip"
            destination = temp_path / "output"
            self._write_artifact_zip(archive_path, files)

            extracted_paths = extract_artifact_zip(archive_path, destination)

            self.assertEqual(len(extracted_paths), len(files))
            extracted_path = destination / "resources/bin/browseros_server"
            self.assertEqual(extracted_path.read_bytes(), b"server-binary")

            if os.name != "nt":
                mode = os.stat(extracted_path).st_mode
                self.assertTrue(mode & stat.S_IRUSR)
                self.assertFalse(mode & stat.S_IXUSR)

    def test_rejects_missing_declared_files(self) -> None:
        files = {
            "resources/bin/browseros_server": b"server-binary",
        }
        metadata_override = {
            "version": "0.0.67",
            "target": "darwin-arm64",
            "generatedAt": "2026-03-06T16:19:09.676Z",
            "files": [
                {
                    "path": "resources/bin/browseros_server",
                    "sha256": hashlib.sha256(files["resources/bin/browseros_server"]).hexdigest(),
                    "size": len(files["resources/bin/browseros_server"]),
                },
                {
                    "path": "resources/bin/third_party/rg",
                    "sha256": hashlib.sha256(b"missing").hexdigest(),
                    "size": len(b"missing"),
                },
            ],
        }

        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            archive_path = temp_path / "artifact.zip"
            self._write_artifact_zip(archive_path, files, metadata_override)

            with self.assertRaisesRegex(RuntimeError, "missing declared file"):
                extract_artifact_zip(archive_path, temp_path / "output")

    def test_rejects_checksum_mismatches(self) -> None:
        files = {
            "resources/bin/browseros_server": b"server-binary",
        }
        metadata_override = {
            "version": "0.0.67",
            "target": "darwin-arm64",
            "generatedAt": "2026-03-06T16:19:09.676Z",
            "files": [
                {
                    "path": "resources/bin/browseros_server",
                    "sha256": hashlib.sha256(b"not-the-file").hexdigest(),
                    "size": len(files["resources/bin/browseros_server"]),
                }
            ],
        }

        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            archive_path = temp_path / "artifact.zip"
            self._write_artifact_zip(archive_path, files, metadata_override)

            with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
                extract_artifact_zip(archive_path, temp_path / "output")

    def test_rejects_non_object_metadata_root(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            archive_path = temp_path / "artifact.zip"

            with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
                archive.writestr(ARTIFACT_METADATA_NAME, json.dumps(["not-a-dict"]))

            with self.assertRaisesRegex(RuntimeError, "JSON object"):
                extract_artifact_zip(archive_path, temp_path / "output")

    def _write_artifact_zip(
        self,
        archive_path: Path,
        files: dict[str, bytes],
        metadata_override: dict | None = None,
        file_modes: dict[str, int] | None = None,
    ) -> None:
        metadata = metadata_override or self._build_metadata(files)

        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr(ARTIFACT_METADATA_NAME, json.dumps(metadata))
            for relative_path, content in files.items():
                info = zipfile.ZipInfo(relative_path)
                mode = (file_modes or {}).get(relative_path)
                if mode is not None:
                    info.external_attr = mode << 16
                archive.writestr(info, content)

    def _build_metadata(self, files: dict[str, bytes]) -> dict:
        return {
            "version": "0.0.67",
            "target": "darwin-arm64",
            "generatedAt": "2026-03-06T16:19:09.676Z",
            "files": [
                {
                    "path": relative_path,
                    "sha256": hashlib.sha256(content).hexdigest(),
                    "size": len(content),
                }
                for relative_path, content in files.items()
            ],
        }


class DownloadResourceConfigTest(unittest.TestCase):
    def test_real_config_includes_server_artifacts_by_target(self) -> None:
        cases = [
            (
                "macos",
                "arm64",
                [
                    (
                        "BrowserOS Server Resources - macOS ARM64",
                        "artifacts/server/latest/browseros-server-resources-darwin-arm64.zip",
                        "resources/binaries/browseros_server/darwin-arm64",
                    ),
                    (
                        "BrowserOS Claw Server Resources - macOS ARM64",
                        "claw-server/prod-resources/latest/browseros-claw-server-resources-darwin-arm64.zip",
                        "resources/binaries/browseros_claw_server/darwin-arm64",
                    ),
                    (
                        "BrowserOS Claw Rust Server Resources - macOS ARM64",
                        "claw-server-rust/prod-resources/latest/browseros-claw-server-rust-resources-darwin-arm64.zip",
                        "resources/binaries/browseros_claw_server_rust/darwin-arm64",
                    ),
                ],
            ),
            (
                "macos",
                "x64",
                [
                    (
                        "BrowserOS Server Resources - macOS x64",
                        "artifacts/server/latest/browseros-server-resources-darwin-x64.zip",
                        "resources/binaries/browseros_server/darwin-x64",
                    ),
                    (
                        "BrowserOS Claw Server Resources - macOS x64",
                        "claw-server/prod-resources/latest/browseros-claw-server-resources-darwin-x64.zip",
                        "resources/binaries/browseros_claw_server/darwin-x64",
                    ),
                    (
                        "BrowserOS Claw Rust Server Resources - macOS x64",
                        "claw-server-rust/prod-resources/latest/browseros-claw-server-rust-resources-darwin-x64.zip",
                        "resources/binaries/browseros_claw_server_rust/darwin-x64",
                    ),
                ],
            ),
            (
                "linux",
                "arm64",
                [
                    (
                        "BrowserOS Server Resources - Linux ARM64",
                        "artifacts/server/latest/browseros-server-resources-linux-arm64.zip",
                        "resources/binaries/browseros_server/linux-arm64",
                    ),
                    (
                        "BrowserOS Claw Server Resources - Linux ARM64",
                        "claw-server/prod-resources/latest/browseros-claw-server-resources-linux-arm64.zip",
                        "resources/binaries/browseros_claw_server/linux-arm64",
                    ),
                    (
                        "BrowserOS Claw Rust Server Resources - Linux ARM64",
                        "claw-server-rust/prod-resources/latest/browseros-claw-server-rust-resources-linux-arm64.zip",
                        "resources/binaries/browseros_claw_server_rust/linux-arm64",
                    ),
                ],
            ),
            (
                "linux",
                "x64",
                [
                    (
                        "BrowserOS Server Resources - Linux x64",
                        "artifacts/server/latest/browseros-server-resources-linux-x64.zip",
                        "resources/binaries/browseros_server/linux-x64",
                    ),
                    (
                        "BrowserOS Claw Server Resources - Linux x64",
                        "claw-server/prod-resources/latest/browseros-claw-server-resources-linux-x64.zip",
                        "resources/binaries/browseros_claw_server/linux-x64",
                    ),
                    (
                        "BrowserOS Claw Rust Server Resources - Linux x64",
                        "claw-server-rust/prod-resources/latest/browseros-claw-server-rust-resources-linux-x64.zip",
                        "resources/binaries/browseros_claw_server_rust/linux-x64",
                    ),
                ],
            ),
            (
                "windows",
                "x64",
                [
                    (
                        "BrowserOS Server Resources - Windows x64",
                        "artifacts/server/latest/browseros-server-resources-windows-x64.zip",
                        "resources/binaries/browseros_server/windows-x64",
                    ),
                    (
                        "BrowserOS Claw Server Resources - Windows x64",
                        "claw-server/prod-resources/latest/browseros-claw-server-resources-windows-x64.zip",
                        "resources/binaries/browseros_claw_server/windows-x64",
                    ),
                    (
                        "BrowserOS Claw Rust Server Resources - Windows x64",
                        "claw-server-rust/prod-resources/latest/browseros-claw-server-rust-resources-windows-x64.zip",
                        "resources/binaries/browseros_claw_server_rust/windows-x64",
                    ),
                ],
            ),
        ]
        operations = self._real_download_operations()

        for platform, arch, expected in cases:
            with self.subTest(platform=platform, arch=arch):
                filtered = self._filter_operations(operations, platform, arch)
                actual = [
                    (op["name"], op["r2_key"], op["destination"])
                    for op in filtered
                    if "Server Resources" in op["name"]
                ]
                self.assertEqual(expected, actual)

    def test_real_config_includes_both_macos_arches_for_universal(self) -> None:
        operations = self._real_download_operations()

        filtered = self._filter_operations(operations, "macos", "universal")

        self.assertEqual(
            [
                "BrowserOS Server Resources - macOS ARM64",
                "BrowserOS Server Resources - macOS x64",
                "BrowserOS Claw Server Resources - macOS ARM64",
                "BrowserOS Claw Server Resources - macOS x64",
                "BrowserOS Claw Rust Server Resources - macOS ARM64",
                "BrowserOS Claw Rust Server Resources - macOS x64",
                "BrowserOS Claw Onboarding Resources",
            ],
            [op["name"] for op in filtered],
        )

    def test_real_config_includes_claw_onboard_resources_everywhere(self) -> None:
        # The onboarding dist is platform-independent and its grit pak is
        # built for every product, so the operation must carry no gates.
        operations = self._real_download_operations()
        expected = (
            "BrowserOS Claw Onboarding Resources",
            "claw-onboard/prod-resources/latest/browseros-claw-onboard-resources.zip",
            "resources/binaries/browseros_claw_onboard",
        )

        onboard_ops = [op for op in operations if op["name"] == expected[0]]
        self.assertEqual(1, len(onboard_ops))
        self.assertEqual("artifact_zip", onboard_ops[0]["download_type"])

        for platform, architecture in [
            ("macos", "arm64"),
            ("macos", "x64"),
            ("macos", "universal"),
            ("linux", "arm64"),
            ("linux", "x64"),
            ("windows", "x64"),
        ]:
            for product in ("browseros", "browserclaw"):
                with self.subTest(
                    platform=platform, arch=architecture, product=product
                ):
                    filtered = self._filter_operations(
                        operations, platform, architecture, product
                    )
                    actual = [
                        (op["name"], op["r2_key"], op["destination"])
                        for op in filtered
                        if op["name"] == expected[0]
                    ]
                    self.assertEqual([expected], actual)

    def test_real_config_downloads_both_claw_servers_for_browserclaw(
        self,
    ) -> None:
        operations = self._real_download_operations()

        filtered = self._filter_operations(
            operations,
            "macos",
            "arm64",
            product="browserclaw",
        )

        self.assertEqual(
            [
                (
                    "BrowserOS Server Resources - macOS ARM64",
                    "artifacts/server/latest/browseros-server-resources-darwin-arm64.zip",
                    "resources/binaries/browseros_server/darwin-arm64",
                ),
                (
                    "BrowserOS Claw Server Resources - macOS ARM64",
                    "claw-server/prod-resources/latest/browseros-claw-server-resources-darwin-arm64.zip",
                    "resources/binaries/browseros_claw_server/darwin-arm64",
                ),
                (
                    "BrowserOS Claw Rust Server Resources - macOS ARM64",
                    "claw-server-rust/prod-resources/latest/browseros-claw-server-rust-resources-darwin-arm64.zip",
                    "resources/binaries/browseros_claw_server_rust/darwin-arm64",
                ),
            ],
            [
                (op["name"], op["r2_key"], op["destination"])
                for op in filtered
                if "Server Resources" in op["name"]
            ],
        )

    def _real_download_operations(self) -> list[dict]:
        config_path = (
            Path(__file__).resolve().parents[2] / "config" / "download_resources.yaml"
        )
        with open(config_path, "r") as f:
            return yaml.safe_load(f)["download_operations"]

    def _filter_operations(
        self,
        operations: list[dict],
        platform: str,
        architecture: str,
        product: str = "browseros",
    ) -> list[dict]:
        ctx = cast(
            Context,
            SimpleNamespace(
                architecture=architecture,
                build_type="release",
                product=get_product_descriptor(product),
            ),
        )
        with patch("bos_build.steps.storage.download.get_platform", return_value=platform):
            return DownloadResourcesModule()._filter_operations(operations, ctx)


if __name__ == "__main__":
    unittest.main()
