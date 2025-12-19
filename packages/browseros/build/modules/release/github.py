#!/usr/bin/env python3
"""GitHub module - Create GitHub releases from R2 artifacts"""

import subprocess
import tempfile
from pathlib import Path
from typing import Dict, List, Optional, Tuple

from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_error, log_success, log_warning
from ..upload import BOTO3_AVAILABLE
from .common import (
    PLATFORMS,
    PLATFORM_DISPLAY_NAMES,
    fetch_all_release_metadata,
    generate_appcast_item,
    generate_release_notes,
    get_repo_from_git,
    check_gh_cli,
)


def create_github_release(
    version: str,
    repo: str,
    title: str,
    notes: str,
    draft: bool = True,
) -> Tuple[bool, str]:
    """Create GitHub release via gh CLI"""
    cmd = [
        "gh",
        "release",
        "create",
        f"v{version}",
        "--repo",
        repo,
        "--title",
        title,
        "--notes",
        notes,
    ]
    if draft:
        cmd.append("--draft")

    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return True, result.stdout.strip()
    except subprocess.CalledProcessError as e:
        if "already exists" in e.stderr:
            return False, f"Release v{version} already exists"
        return False, e.stderr


def download_file(url: str, dest: Path) -> bool:
    """Download file from URL using curl"""
    try:
        subprocess.run(
            ["curl", "-L", "-o", str(dest), url],
            check=True,
            capture_output=True,
        )
        return True
    except Exception:
        return False


def upload_to_github_release(version: str, repo: str, file_path: Path) -> bool:
    """Upload file to existing GitHub release"""
    try:
        subprocess.run(
            ["gh", "release", "upload", f"v{version}", str(file_path), "--repo", repo],
            check=True,
            capture_output=True,
        )
        return True
    except Exception:
        return False


def normalize_version(version: str) -> str:
    """Normalize version to MAJOR.MINOR.BUILD (strip patch if present)"""
    parts = version.split(".")
    if len(parts) >= 3:
        return ".".join(parts[:3])
    return version


def download_and_upload_artifacts(
    version: str,
    repo: str,
    metadata: Dict[str, Dict],
    platforms: Optional[List[str]] = None,
) -> List[Tuple[str, bool]]:
    """Download artifacts from R2 and upload to GitHub release"""
    if platforms is None:
        platforms = PLATFORMS

    results = []

    with tempfile.TemporaryDirectory() as tmpdir:
        tmppath = Path(tmpdir)

        for platform in platforms:
            if platform not in metadata:
                continue

            for key, artifact in metadata[platform].get("artifacts", {}).items():
                url = artifact["url"]
                filename = artifact["filename"]
                local_path = tmppath / filename

                log_info(f"  Downloading {filename}...")
                if not download_file(url, local_path):
                    log_error(f"  Failed to download {filename}")
                    results.append((filename, False))
                    continue

                log_info(f"  Uploading {filename}...")
                if upload_to_github_release(version, repo, local_path):
                    log_success(f"  Uploaded {filename}")
                    results.append((filename, True))
                else:
                    log_error(f"  Failed to upload {filename}")
                    results.append((filename, False))

    return results


class GithubModule(CommandModule):
    """Create GitHub release from R2 artifacts"""

    produces = []
    requires = []
    description = "Create GitHub release from R2 artifacts"

    def __init__(
        self,
        draft: bool = True,
        skip_upload: bool = False,
        title: Optional[str] = None,
    ):
        self.draft = draft
        self.skip_upload = skip_upload
        self.title = title

    def validate(self, ctx: Context) -> None:
        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )

        if not ctx.env.has_r2_config():
            raise ValidationError("R2 configuration not set")

        if not ctx.release_version:
            raise ValidationError("--version is required")

        if not check_gh_cli():
            raise ValidationError(
                "gh CLI not found. Install from: https://cli.github.com"
            )

        # Determine repo
        if not ctx.github_repo:
            repo = get_repo_from_git()
            if not repo:
                raise ValidationError(
                    "Could not detect repo from git remote. Use --repo flag."
                )
            ctx.github_repo = repo

    def execute(self, ctx: Context) -> None:
        version = ctx.release_version
        tag_version = normalize_version(version)
        repo = ctx.github_repo

        metadata = fetch_all_release_metadata(version, ctx.env)
        if not metadata:
            log_error(f"No release metadata found for version {version}")
            return

        log_info(f"\n{'='*60}")
        log_info(f"Creating GitHub Release: v{tag_version}")
        log_info(f"{'='*60}")

        for platform, release in metadata.items():
            artifacts = release.get("artifacts", {})
            log_info(f"  {PLATFORM_DISPLAY_NAMES[platform]}: {len(artifacts)} artifact(s)")

        log_info(f"  Repo: {repo}")
        log_info(f"  Draft: {self.draft}")

        # Create release
        release_title = self.title or f"v{tag_version}"
        notes = generate_release_notes(tag_version, metadata)

        log_info("\nCreating GitHub release...")
        success, result = create_github_release(tag_version, repo, release_title, notes, self.draft)

        if success:
            log_success(f"Release created: {result}")
        else:
            if "already exists" in result:
                log_warning(result)
            else:
                log_error(f"Failed to create release: {result}")
                return

        # Upload artifacts
        if not self.skip_upload:
            log_info("\nUploading artifacts to GitHub release...")
            results = download_and_upload_artifacts(tag_version, repo, metadata)

            failed = [f for f, ok in results if not ok]
            if failed:
                log_warning(f"Failed to upload: {', '.join(failed)}")

        # Print appcast snippet
        if "macos" in metadata:
            log_info("\n" + "=" * 60)
            log_info("APPCAST SNIPPET")
            log_info("=" * 60)

            release = metadata["macos"]
            sparkle_version = release.get("sparkle_version", "")
            build_date = release.get("build_date", "")

            arch_to_file = {"arm64": "appcast.xml", "x64": "appcast-x86_64.xml", "universal": "appcast.xml"}

            for arch in ["arm64", "x64", "universal"]:
                if arch in release.get("artifacts", {}):
                    artifact = release["artifacts"][arch]
                    log_info(f"\n{arch_to_file[arch]} ({arch}):")
                    print(generate_appcast_item(artifact, tag_version, sparkle_version, build_date))

        log_info(f"\n{'='*60}")
        log_success(f"Release v{tag_version} complete!")
