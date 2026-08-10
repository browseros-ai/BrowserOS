#!/usr/bin/env python3
"""Publish module - Copy versioned artifacts to download/ paths for fresh installs"""

from typing import Dict, List, Optional, Tuple

from ..core.context import Context
from ..core.step import Step, ValidationError
from ..lib.utils import log_error, log_info, log_success, log_warning
from ..lib.r2 import BOTO3_AVAILABLE, get_r2_client
from .common import (
    PLATFORMS,
    PLATFORM_DISPLAY_NAMES,
    fetch_all_release_metadata,
    get_download_path_mapping,
    validate_release_metadata,
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


class PublishModule(Step):
    """Copy versioned artifacts to download/ paths (make release "live")"""

    produces = []
    requires = []
    description = "Publish versioned artifacts to latest download URLs"

    def __init__(
        self,
        platforms: Optional[List[str]] = None,
        macos_arch: str = "universal",
        source_sha: str = "",
        workflow_run_id: str = "",
        workflow_run_attempt: str = "",
    ):
        self.selected_platforms = platforms or PLATFORMS
        self.allow_missing_platforms = platforms is None
        self.macos_arch = macos_arch
        self.source_sha = source_sha
        self.workflow_run_id = workflow_run_id
        self.workflow_run_attempt = workflow_run_attempt

    def validate(self, context: Context) -> None:
        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )

        if not context.env.has_r2_config():
            raise ValidationError("R2 configuration not set")

        if not context.release_version:
            raise ValidationError("--version is required")
        invalid = sorted(set(self.selected_platforms) - set(PLATFORMS))
        if invalid:
            raise ValidationError(
                f"Unknown platform(s): {', '.join(invalid)}. "
                f"Valid: {', '.join(PLATFORMS)}"
            )

    def execute(self, context: Context) -> None:
        version = context.release_version
        env = context.env

        metadata = fetch_all_release_metadata(version, env, context.product.id)
        if not metadata:
            raise RuntimeError(f"No release metadata found for version {version}")
        missing_platforms = [
            platform for platform in self.selected_platforms if platform not in metadata
        ]
        if missing_platforms and not self.allow_missing_platforms:
            raise RuntimeError(
                "Missing release metadata for requested platform(s): "
                f"{', '.join(missing_platforms)}"
            )
        if missing_platforms:
            log_warning(
                "Skipping platforms with no release metadata: "
                f"{', '.join(missing_platforms)}"
            )
        loaded_platforms = [
            platform for platform in self.selected_platforms if platform in metadata
        ]
        if any(
            (
                self.source_sha,
                self.workflow_run_id,
                self.workflow_run_attempt,
            )
        ):
            validated: Dict[str, dict] = {}
            for platform in loaded_platforms:
                selection = "windows" if platform == "win" else platform
                validated.update(
                    validate_release_metadata(
                        metadata,
                        version=version,
                        product_id=context.product.id,
                        platforms=selection,
                        macos_arch=self.macos_arch,
                        source_sha=self.source_sha,
                        workflow_run_id=self.workflow_run_id,
                        workflow_run_attempt=self.workflow_run_attempt,
                    )
                )
            metadata = validated

        candidates: Dict[str, List[Tuple[str, str, str]]] = {}
        for platform in loaded_platforms:
            release = metadata[platform]
            artifacts = release.get("artifacts", {})
            platform_mapping = get_download_path_mapping(context.product).get(
                platform, {}
            )
            platform_candidates: List[Tuple[str, str, str]] = []
            for artifact_key, artifact in artifacts.items():
                if artifact_key not in platform_mapping:
                    continue
                dest_path = platform_mapping[artifact_key]
                source_key = _release_source_key(context, platform, version, artifact)
                platform_candidates.append(
                    (artifact["filename"], source_key, dest_path)
                )
            if not platform_candidates:
                raise RuntimeError(
                    f"No promotable artifacts found for requested platform {platform}"
                )
            candidates[platform] = platform_candidates

        log_info(f"\n{'='*60}")
        log_info(f"Publishing v{version} to download/ paths")
        log_info(f"{'='*60}")

        client = get_r2_client(env)
        if not client:
            raise RuntimeError("Failed to create R2 client")

        results: List[Tuple[str, str, bool]] = []

        for platform, platform_candidates in candidates.items():
            log_info(f"\n{PLATFORM_DISPLAY_NAMES[platform]}:")
            for filename, source_key, dest_path in platform_candidates:
                log_info(f"  Copying {filename} → {dest_path}")
                success = copy_to_download_path(
                    client, env.r2_bucket, source_key, dest_path
                )
                results.append((filename, dest_path, success))

                if success:
                    log_success(f"    ✓ Published to {env.r2_cdn_base_url}/{dest_path}")

        log_info(f"\n{'='*60}")
        succeeded = sum(1 for _, _, ok in results if ok)
        failed = sum(1 for _, _, ok in results if not ok)

        if not results:
            raise RuntimeError("No promotable artifacts found in release metadata")
        if failed:
            for filename, dest, ok in results:
                if not ok:
                    log_error(f"  Failed: {filename} → {dest}")
            raise RuntimeError(
                f"Failed to publish {failed}/{succeeded + failed} artifact(s)"
            )
        log_success(f"Published {succeeded} artifact(s) to download/ paths")


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
