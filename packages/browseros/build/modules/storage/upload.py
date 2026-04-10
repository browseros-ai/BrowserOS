#!/usr/bin/env python3
"""Upload module for BrowserOS build artifacts to Cloudflare R2"""

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple, cast

from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import (
    log_info,
    log_error,
    log_success,
    log_warning,
    IS_WINDOWS,
    IS_MACOS,
)
from ...common.notify import get_notifier, COLOR_GREEN

from .r2 import (
    BOTO3_AVAILABLE,
    get_r2_client,
    get_release_json,
    upload_file_to_r2,
)


def _get_platform() -> str:
    """Get platform name for R2 path"""
    if IS_MACOS():
        return "macos"
    elif IS_WINDOWS():
        return "win"
    else:
        return "linux"


class UploadModule(CommandModule):
    """Upload build artifacts to Cloudflare R2"""

    produces = []
    requires = []
    description = "Upload build artifacts to Cloudflare R2"

    def validate(self, ctx: Context) -> None:
        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )

        if not ctx.env.has_r2_config():
            raise ValidationError(
                "R2 configuration not set. Required env vars: "
                "R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY"
            )

    def execute(self, ctx: Context) -> None:
        log_info("\nUploading package artifacts to R2...")

        extra_metadata = {}
        sparkle_signatures = cast(
            Optional[dict[str, tuple[str, int]]],
            ctx.artifacts.get("sparkle_signatures"),
        )
        if sparkle_signatures:
            for filename, (sig, length) in sparkle_signatures.items():
                extra_metadata[filename] = {
                    "sparkle_signature": sig,
                    "sparkle_length": length,
                }

        success, release_json = upload_release_artifacts(ctx, extra_metadata)
        if not success:
            raise RuntimeError("Failed to upload artifacts to R2")


def generate_release_json(
    ctx: Context,
    artifacts: List[Dict],
    platform: str,
) -> Dict:
    """Generate release.json metadata for a platform

    Args:
        ctx: Build context
        artifacts: List of artifact dicts with filename, size, and any extra fields
        platform: Platform name (macos, win, linux)

    Returns:
        Dict containing release metadata
    """
    env = ctx.env

    release_data = {
        "platform": platform,
        "version": ctx.get_semantic_version(),
        "chromium_version": ctx.chromium_version,
        "browseros_chromium_version": ctx.browseros_chromium_version,
        "build_date": datetime.now(timezone.utc).isoformat(),
        "artifacts": {},
    }

    if platform == "macos":
        release_data["sparkle_version"] = ctx.get_sparkle_version()

    base_url = f"{env.r2_cdn_base_url}/{ctx.get_release_path(platform)}"

    for artifact in artifacts:
        filename = artifact["filename"]
        artifact_key = _get_artifact_key(filename, platform)

        artifact_data = {
            "filename": filename,
            "url": f"{base_url}{filename}",
        }

        for key, value in artifact.items():
            if key != "filename":
                artifact_data[key] = value

        release_data["artifacts"][artifact_key] = artifact_data

    return release_data


def merge_release_metadata(existing: Optional[Dict], new: Dict) -> Dict:
    if not existing:
        return new

    merged = dict(existing)
    merged.update({key: value for key, value in new.items() if key != "artifacts"})

    artifacts = dict(existing.get("artifacts", {}))
    artifacts.update(new.get("artifacts", {}))
    merged["artifacts"] = artifacts
    return merged


def _get_linux_artifact_key(filename: str) -> Optional[str]:
    lower = filename.lower()

    if ".appimage" in lower:
        if "arm64" in lower or "aarch64" in lower:
            return "arm64_appimage"
        if "x64" in lower or "x86_64" in lower:
            return "x64_appimage"
    elif ".deb" in lower:
        if "arm64" in lower or "aarch64" in lower:
            return "arm64_deb"
        if "amd64" in lower or "x64" in lower or "x86_64" in lower:
            return "x64_deb"

    return None


def _get_artifact_key(filename: str, platform: str) -> str:
    """Get artifact key name from filename

    Examples:
        BrowserOS_v0.31.0_arm64.dmg -> arm64
        BrowserOS_v0.31.0_x64.dmg -> x64
        BrowserOS_v0.31.0_x64_installer.exe -> x64_installer
        BrowserOS_v0.31.0_x64.AppImage -> x64_appimage
        browseros_0.31.0_amd64.deb -> x64_deb
    """
    lower = filename.lower()

    if platform == "macos":
        if "arm64" in lower:
            return "arm64"
        elif "x64" in lower or "x86_64" in lower:
            return "x64"
        elif "universal" in lower:
            return "universal"

    elif platform == "win":
        if "installer.exe" in lower:
            return "x64_installer"
        elif "installer.zip" in lower:
            return "x64_zip"

    elif platform == "linux":
        artifact_key = _get_linux_artifact_key(filename)
        if artifact_key:
            return artifact_key
        log_warning(f"Unrecognized Linux artifact name: {filename}; using stem key")

    return Path(filename).stem


def detect_artifacts(ctx: Context) -> List[Path]:
    """Detect artifacts in dist directory based on platform

    Returns:
        List of artifact file paths found
    """
    dist_dir = ctx.get_dist_dir()
    if not dist_dir.exists():
        return []

    artifacts = []

    if IS_MACOS():
        artifacts.extend(dist_dir.glob("*.dmg"))
    elif IS_WINDOWS():
        artifacts.extend(dist_dir.glob("*.exe"))
        artifacts.extend(dist_dir.glob("*.zip"))
    else:  # Linux
        artifacts.extend(dist_dir.glob("*.AppImage"))
        artifacts.extend(dist_dir.glob("*.deb"))

    return sorted(artifacts)


def upload_release_artifacts(
    ctx: Context,
    extra_metadata: Optional[Dict[str, Dict[str, Any]]] = None,
) -> Tuple[bool, Optional[Dict]]:
    """Upload release artifacts to R2 and generate release.json

    Args:
        ctx: Build context
        extra_metadata: Optional dict mapping filename to extra metadata fields
                       e.g. {"file.dmg": {"sparkle_signature": "...", "sparkle_length": 123}}

    Returns:
        (success, release_json_data) tuple
    """
    if not BOTO3_AVAILABLE:
        log_warning("boto3 not installed. Skipping R2 upload.")
        log_info("Install with: pip install boto3")
        return True, None

    env = ctx.env

    if not env.has_r2_config():
        log_warning("R2 configuration not set. Skipping upload.")
        return True, None

    artifacts = detect_artifacts(ctx)
    if not artifacts:
        log_info("No artifacts found to upload")
        return True, None

    platform = _get_platform()
    release_path = ctx.get_release_path(platform)

    log_info(f"\nUploading to R2: {env.r2_bucket}/{release_path}")
    log_info(f"Found {len(artifacts)} artifact(s):")
    for artifact in artifacts:
        log_info(f"  - {artifact.name}")

    client = get_r2_client(env)
    if not client:
        log_error("Failed to create R2 client")
        return False, None

    artifact_metadata = []
    for artifact_path in artifacts:
        r2_key = f"{release_path}{artifact_path.name}"

        if not upload_file_to_r2(client, artifact_path, r2_key, env.r2_bucket):
            return False, None

        metadata = {
            "filename": artifact_path.name,
            "size": artifact_path.stat().st_size,
        }

        if extra_metadata and artifact_path.name in extra_metadata:
            metadata.update(extra_metadata[artifact_path.name])

        artifact_metadata.append(metadata)

    release_data = generate_release_json(ctx, artifact_metadata, platform)
    if platform == "linux":
        # Linux x64 and arm64 release jobs must be sequenced. A parallel
        # fetch-merge-upload flow can still race and drop one architecture.
        existing_release_data = get_release_json(
            ctx.get_semantic_version(), platform, env
        )
        release_data = merge_release_metadata(existing_release_data, release_data)
    release_json_path = ctx.get_dist_dir() / "release.json"
    release_json_path.write_text(json.dumps(release_data, indent=2))

    r2_key = f"{release_path}release.json"
    if not upload_file_to_r2(client, release_json_path, r2_key, env.r2_bucket):
        return False, None

    log_success(f"\nSuccessfully uploaded {len(artifacts)} artifact(s) to R2")
    log_info("\nRelease metadata:")
    log_info(f"  Version: {release_data['version']}")
    if platform == "macos":
        log_info(f"  Sparkle version: {release_data.get('sparkle_version', 'N/A')}")
    log_info(f"  Artifacts: {list(release_data['artifacts'].keys())}")

    notifier = get_notifier()
    artifact_urls = [
        f"{a['filename']}: {a['url']}" for a in release_data["artifacts"].values()
    ]
    notifier.notify(
        "Upload Complete",
        f"Uploaded {len(artifacts)} artifact(s) to R2",
        {
            "Version": release_data["version"],
            "Platform": platform,
            "Artifacts": "\n".join(artifact_urls),
        },
        color=COLOR_GREEN,
    )

    return True, release_data
