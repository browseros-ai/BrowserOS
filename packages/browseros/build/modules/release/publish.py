#!/usr/bin/env python3
"""Publish module - Copy versioned artifacts to download/ paths for fresh installs"""

from typing import List, Optional, Tuple

from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_error, log_success, log_warning
from ..storage import BOTO3_AVAILABLE, get_r2_client
from .common import (
    PLATFORMS,
    PLATFORM_DISPLAY_NAMES,
    fetch_all_release_metadata,
    get_download_path_mapping,
)


def copy_to_download_path(
    client,
    bucket: str,
    source_key: str,
    dest_key: str,
) -> bool:
    """Copy object within R2 bucket"""
    try:
        client.copy_object(
            Bucket=bucket,
            CopySource={"Bucket": bucket, "Key": source_key},
            Key=dest_key,
        )
        return True
    except Exception as e:
        log_error(f"Failed to copy {source_key} → {dest_key}: {e}")
        return False


class PublishModule(CommandModule):
    """Copy versioned artifacts to download/ paths (make release "live")"""

    produces = []
    requires = []
    description = "Publish versioned artifacts to latest download URLs"

    def __init__(self, platforms: Optional[List[str]] = None):
        self.platforms = platforms or PLATFORMS

    def validate(self, ctx: Context) -> None:
        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )

        if not ctx.env.has_r2_config():
            raise ValidationError("R2 configuration not set")

        if not ctx.release_version:
            raise ValidationError("--version is required")

    def execute(self, ctx: Context) -> None:
        version = ctx.release_version
        env = ctx.env

        metadata = fetch_all_release_metadata(version, env, ctx.product.id)
        if not metadata:
            log_error(f"No release metadata found for version {version}")
            return

        log_info(f"\n{'='*60}")
        log_info(f"Publishing v{version} to download/ paths")
        log_info(f"{'='*60}")

        client = get_r2_client(env)
        if not client:
            log_error("Failed to create R2 client")
            return

        results: List[Tuple[str, str, bool]] = []

        for platform in self.platforms:
            if platform not in metadata:
                log_warning(f"Skipping {platform}: no release metadata")
                continue

            release = metadata[platform]
            artifacts = release.get("artifacts", {})
            platform_mapping = get_download_path_mapping(ctx.product).get(platform, {})

            log_info(f"\n{PLATFORM_DISPLAY_NAMES[platform]}:")

            for artifact_key, artifact in artifacts.items():
                if artifact_key not in platform_mapping:
                    log_info(f"  Skipping {artifact_key}: no download path mapping")
                    continue

                dest_path = platform_mapping[artifact_key]
                source_key = _release_source_key(ctx, platform, version, artifact)

                log_info(f"  Copying {artifact['filename']} → {dest_path}")
                success = copy_to_download_path(client, env.r2_bucket, source_key, dest_path)
                results.append((artifact["filename"], dest_path, success))

                if success:
                    log_success(f"    ✓ Published to {env.r2_cdn_base_url}/{dest_path}")

        log_info(f"\n{'='*60}")
        succeeded = sum(1 for _, _, ok in results if ok)
        failed = sum(1 for _, _, ok in results if not ok)

        if failed == 0:
            log_success(f"Published {succeeded} artifact(s) to download/ paths")
        else:
            log_warning(f"Published {succeeded}/{succeeded + failed} artifact(s)")
            for filename, dest, ok in results:
                if not ok:
                    log_error(f"  Failed: {filename} → {dest}")


def _release_source_key(
    ctx: Context,
    platform: str,
    version: str,
    artifact: dict,
) -> str:
    """Resolve the R2 key for an artifact in release metadata."""
    cdn_prefix = f"{ctx.env.r2_cdn_base_url.rstrip('/')}/"
    url = artifact.get("url", "")
    if url.startswith(cdn_prefix):
        return url.removeprefix(cdn_prefix)
    return (
        f"releases/{ctx.product.release_prefix}/{version}/"
        f"{platform}/{artifact['filename']}"
    )
