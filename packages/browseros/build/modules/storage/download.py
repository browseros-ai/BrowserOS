#!/usr/bin/env python3
"""Download module for fetching build resources from Cloudflare R2"""

import hashlib
import json
import shutil
import tempfile
import yaml
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any, List

from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import (
    log_info,
    log_success,
    get_platform,
)

from .r2 import (
    BOTO3_AVAILABLE,
    get_r2_client,
    download_file_from_r2,
)

ARTIFACT_ZIP_DOWNLOAD = "artifact_zip"
ARTIFACT_METADATA_NAME = "artifact-metadata.json"
COPY_CHUNK_SIZE = 1024 * 1024


def extract_artifact_zip(archive_path: Path, destination: Path) -> list[Path]:
    """Extract a BrowserOS resource artifact zip into a destination directory."""
    with zipfile.ZipFile(archive_path, "r") as archive:
        metadata_bytes = _read_artifact_metadata_bytes(archive)
        metadata = _parse_artifact_metadata(metadata_bytes)
        extracted_paths = _extract_artifact_files(
            archive, metadata["files"], destination
        )

    destination.mkdir(parents=True, exist_ok=True)
    (destination / ARTIFACT_METADATA_NAME).write_bytes(metadata_bytes)
    return extracted_paths


def _read_artifact_metadata_bytes(archive: zipfile.ZipFile) -> bytes:
    try:
        return archive.read(ARTIFACT_METADATA_NAME)
    except KeyError as exc:
        raise RuntimeError(
            f"Artifact archive is missing {ARTIFACT_METADATA_NAME}"
        ) from exc


def _parse_artifact_metadata(metadata_bytes: bytes) -> dict[str, Any]:
    try:
        metadata = json.loads(metadata_bytes.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise RuntimeError("Artifact metadata is not valid JSON") from exc

    if not isinstance(metadata, dict):
        raise RuntimeError("Artifact metadata must be a JSON object")

    files = metadata.get("files")
    if not isinstance(files, list) or not files:
        raise RuntimeError("Artifact metadata must contain a non-empty files list")

    return metadata


def _extract_artifact_files(
    archive: zipfile.ZipFile, files: list[dict[str, Any]], destination: Path
) -> list[Path]:
    extracted_paths = []

    for entry in files:
        relative_path, expected_size, expected_sha256 = _parse_artifact_entry(entry)
        archive_member = relative_path.as_posix()
        dest_path = destination.joinpath(*relative_path.parts)

        try:
            source_file = archive.open(archive_member, "r")
        except KeyError as exc:
            raise RuntimeError(
                f"Artifact archive is missing declared file: {archive_member}"
            ) from exc

        dest_path.parent.mkdir(parents=True, exist_ok=True)
        sha256 = hashlib.sha256()
        total_size = 0

        with source_file, open(dest_path, "wb") as output_file:
            while chunk := source_file.read(COPY_CHUNK_SIZE):
                output_file.write(chunk)
                sha256.update(chunk)
                total_size += len(chunk)

        if total_size != expected_size:
            raise RuntimeError(
                f"Artifact file size mismatch for {archive_member}: "
                f"expected {expected_size}, got {total_size}"
            )

        actual_sha256 = sha256.hexdigest()
        if actual_sha256 != expected_sha256:
            raise RuntimeError(
                f"Artifact checksum mismatch for {archive_member}: "
                f"expected {expected_sha256}, got {actual_sha256}"
            )

        if _should_mark_executable(relative_path):
            dest_path.chmod(dest_path.stat().st_mode | 0o755)

        extracted_paths.append(dest_path)

    return extracted_paths


def _parse_artifact_entry(entry: Any) -> tuple[PurePosixPath, int, str]:
    if not isinstance(entry, dict):
        raise RuntimeError("Artifact metadata file entries must be objects")

    relative_path = _normalize_artifact_path(entry.get("path"))
    expected_sha256 = entry.get("sha256")
    expected_size = entry.get("size")

    if not isinstance(expected_sha256, str) or len(expected_sha256) != 64:
        raise RuntimeError(
            f"Artifact metadata has invalid sha256 for {relative_path.as_posix()}"
        )

    if not isinstance(expected_size, int) or expected_size < 0:
        raise RuntimeError(
            f"Artifact metadata has invalid size for {relative_path.as_posix()}"
        )

    return relative_path, expected_size, expected_sha256.lower()


def _normalize_artifact_path(raw_path: Any) -> PurePosixPath:
    if not isinstance(raw_path, str) or not raw_path:
        raise RuntimeError("Artifact metadata file entry is missing path")

    relative_path = PurePosixPath(raw_path)
    if relative_path.is_absolute() or ".." in relative_path.parts:
        raise RuntimeError(f"Artifact metadata path is unsafe: {raw_path}")

    if raw_path.endswith("/") or relative_path == PurePosixPath("."):
        raise RuntimeError(f"Artifact metadata path is not a file: {raw_path}")

    return relative_path


def _should_mark_executable(relative_path: PurePosixPath) -> bool:
    parts = relative_path.parts
    if get_platform() == "windows":
        return False

    return len(parts) >= 2 and parts[0] == "resources" and parts[1] == "bin"


def _clear_destination(dest_path: Path) -> None:
    if not dest_path.exists():
        return

    if dest_path.is_dir():
        shutil.rmtree(dest_path)
        return

    dest_path.unlink()


class DownloadResourcesModule(CommandModule):
    """Download resources from Cloudflare R2 before build

    This module downloads binaries and other resources from R2 that are
    required for the build but not stored in the repository.

    Behavior:
        - Always clears existing files and re-downloads (ensures latest)
        - Fails immediately if any download fails
        - For universal builds on macOS, downloads both arm64 and x64 binaries
    """

    produces = []
    requires = []
    description = "Download resources from Cloudflare R2"

    def validate(self, context: Context) -> None:
        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )

        if not context.env.has_r2_config():
            raise ValidationError(
                "R2 configuration not set. Required env vars: "
                "R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY"
            )

        config_path = context.get_download_resources_config()
        if not config_path.exists():
            raise ValidationError(
                f"Download configuration file not found: {config_path}"
            )

    def execute(self, context: Context) -> None:
        log_info("\nDownloading resources from R2...")

        config_path = context.get_download_resources_config()
        with open(config_path, "r") as f:
            config = yaml.safe_load(f)

        if "download_operations" not in config:
            log_info("No download_operations defined in configuration")
            return

        operations = config["download_operations"]
        filtered_ops = self._filter_operations(operations, context)

        if not filtered_ops:
            log_info("No downloads needed for current platform/architecture")
            return

        log_info(f"Downloading {len(filtered_ops)} resource(s)...")

        client = get_r2_client(context.env)
        if not client:
            raise RuntimeError("Failed to create R2 client")

        bucket = context.env.r2_bucket

        for op in filtered_ops:
            name = op.get("name", "Unnamed")
            destination = op["destination"]
            dest_path = context.root_dir / destination

            log_info(f"  {name}")

            # Clear existing destination (always re-download)
            if dest_path.exists():
                _clear_destination(dest_path)
                log_info(f"    Cleared existing: {dest_path.name}")

            self._download_operation(client, bucket, op, dest_path)

        log_success(f"Downloaded {len(filtered_ops)} resource(s) from R2")

    def _download_operation(
        self, client, bucket: str, operation: dict[str, Any], dest_path: Path
    ) -> None:
        download_type = operation.get("download_type", "file")
        r2_key = operation["r2_key"]

        if download_type == ARTIFACT_ZIP_DOWNLOAD:
            self._download_artifact_zip(client, bucket, r2_key, dest_path)
            return

        if not download_file_from_r2(client, r2_key, dest_path, bucket):
            raise RuntimeError(f"Failed to download: {operation.get('name', r2_key)}")

        if operation.get("executable", False):
            dest_path.chmod(dest_path.stat().st_mode | 0o755)
            log_info("    Set executable permissions")

    def _download_artifact_zip(
        self, client, bucket: str, r2_key: str, dest_path: Path
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            archive_path = Path(temp_dir) / "artifact.zip"
            if not download_file_from_r2(client, r2_key, archive_path, bucket):
                raise RuntimeError(f"Failed to download artifact zip: {r2_key}")

            extracted_paths = extract_artifact_zip(archive_path, dest_path)
            log_info(f"    Extracted {len(extracted_paths)} artifact file(s)")

    def _filter_operations(
        self,
        operations: List[dict],
        context: Context,
    ) -> List[dict]:
        """Filter operations based on os, arch, and build_type conditions

        For universal builds on macOS, includes both arm64 and x64 operations.
        """
        current_os = get_platform()
        current_arch = context.architecture
        current_build_type = context.build_type

        # For universal builds, we need both arm64 and x64
        target_archs = [current_arch]
        if current_arch == "universal":
            target_archs = ["arm64", "x64", "universal"]

        filtered = []

        for op in operations:
            # Check OS condition
            os_condition = op.get("os")
            if os_condition and current_os not in os_condition:
                continue

            # Check architecture condition
            arch_condition = op.get("arch")
            if arch_condition:
                # Check if any target arch matches any condition arch
                if not any(arch in arch_condition for arch in target_archs):
                    continue

            # Check build_type condition
            build_type_condition = op.get("build_type")
            if build_type_condition and build_type_condition != current_build_type:
                continue

            filtered.append(op)

        return filtered
