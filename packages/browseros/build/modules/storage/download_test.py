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

from build.modules.storage.download import (
    ARTIFACT_METADATA_NAME,
    extract_artifact_zip,
)


class ExtractArtifactZipTest(unittest.TestCase):
    def test_extracts_declared_files_and_writes_metadata(self) -> None:
        files = {
            "resources/bin/browseros_server": b"server-binary",
            "resources/bin/third_party/bun": b"bun-binary",
            "resources/bin/third_party/rg": b"rg-binary",
        }

        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            archive_path = temp_path / "artifact.zip"
            destination = temp_path / "output"
            self._write_artifact_zip(archive_path, files)

            extracted_paths = extract_artifact_zip(archive_path, destination)

            self.assertEqual(len(extracted_paths), len(files))
            metadata_path = destination / ARTIFACT_METADATA_NAME
            self.assertTrue(metadata_path.exists())

            for relative_path, content in files.items():
                extracted_path = destination / relative_path
                self.assertEqual(extracted_path.read_bytes(), content)

                if os.name != "nt":
                    self.assertTrue(
                        os.stat(extracted_path).st_mode & stat.S_IXUSR,
                        f"{relative_path} should be executable",
                    )

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
    ) -> None:
        metadata = metadata_override or self._build_metadata(files)

        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr(ARTIFACT_METADATA_NAME, json.dumps(metadata))
            for relative_path, content in files.items():
                archive.writestr(relative_path, content)

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


if __name__ == "__main__":
    unittest.main()
