#!/usr/bin/env python3
"""Download module - Download release artifacts from CDN"""

import sys
import tempfile
from pathlib import Path
from typing import Optional

import requests

from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_error
from .common import (
    PLATFORMS,
    PLATFORM_DISPLAY_NAMES,
    fetch_all_release_metadata,
    format_size,
)

OS_NAME_MAP = {
    "macos": "macos",
    "mac": "macos",
    "windows": "win",
    "win": "win",
    "linux": "linux",
}


class DownloadModule(CommandModule):
    """Download release artifacts from CDN"""

    produces = []
    requires = []
    description = "Download release artifacts from CDN"

    def __init__(self, os_filter: Optional[str] = None, output_dir: Optional[Path] = None):
        self.os_filter = os_filter
        self.output_dir = output_dir

    def validate(self, ctx: Context) -> None:
        if not ctx.release_version:
            raise ValidationError("--version is required")

        if self.os_filter:
            normalized = OS_NAME_MAP.get(self.os_filter.lower())
            if not normalized:
                valid = ", ".join(OS_NAME_MAP.keys())
                raise ValidationError(
                    f"Invalid --os value: {self.os_filter}. Valid: {valid}"
                )
            self.os_filter = normalized

    def execute(self, ctx: Context) -> None:
        version = ctx.release_version
        metadata = fetch_all_release_metadata(version, ctx.env)

        if not metadata:
            log_error(f"No release metadata found for version {version}")
            return

        if self.output_dir:
            download_dir = self.output_dir / version
        else:
            download_dir = Path(tempfile.gettempdir()) / "browseros-releases" / version
        download_dir.mkdir(parents=True, exist_ok=True)

        log_info(f"\nDownloading to {download_dir}\n")

        platforms_to_download = (
            [self.os_filter] if self.os_filter else PLATFORMS
        )

        for platform in platforms_to_download:
            if platform not in metadata:
                continue

            release = metadata[platform]
            artifacts = release.get("artifacts", {})

            if not artifacts:
                continue

            log_info(f"{PLATFORM_DISPLAY_NAMES[platform]}:")

            for key, artifact in artifacts.items():
                url = artifact.get("url")
                filename = artifact.get("filename")
                expected_size = artifact.get("size", 0)

                if not url or not filename:
                    continue

                dest_path = download_dir / filename
                self._download_file(url, dest_path, filename, expected_size)

            log_info("")

        log_info(f"Downloaded to: {download_dir}")

    def _download_file(
        self, url: str, dest: Path, filename: str, expected_size: int
    ) -> None:
        """Download a file with progress indicator"""
        try:
            response = requests.get(url, stream=True, timeout=30)
            response.raise_for_status()

            total_size = int(response.headers.get("content-length", expected_size))
            total_mb = total_size / (1024 * 1024)

            downloaded = 0
            with open(dest, "wb") as f:
                for chunk in response.iter_content(chunk_size=65536):
                    f.write(chunk)
                    downloaded += len(chunk)
                    downloaded_mb = downloaded / (1024 * 1024)
                    percent = (downloaded / total_size * 100) if total_size else 0
                    sys.stdout.write(
                        f"\r  Downloading {filename}... {percent:.0f}% ({downloaded_mb:.0f}/{total_mb:.0f} MB)  "
                    )
                    sys.stdout.flush()

            sys.stdout.write(f"\r  {filename} ({format_size(total_size)})" + " " * 40 + "\n")
            sys.stdout.flush()

        except requests.RequestException as e:
            sys.stdout.write(f"\r  {filename} - FAILED: {e}" + " " * 40 + "\n")
            sys.stdout.flush()
