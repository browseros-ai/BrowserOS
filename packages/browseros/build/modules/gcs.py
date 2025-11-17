#!/usr/bin/env python3
"""
Google Cloud Storage upload module for BlockBrowser build artifacts
"""

import os
import sys
from pathlib import Path
from typing import List, Optional, Tuple
from context import BuildContext
from utils import (
    log_info,
    log_error,
    log_success,
    log_warning,
    IS_WINDOWS,
    IS_MACOS,
    IS_LINUX,
    join_paths,
)

# Try to import google-cloud-storage
try:
    from google.cloud import storage
    from google.oauth2 import service_account

    GCS_AVAILABLE = True
except ImportError:
    GCS_AVAILABLE = False

# Service account file name
SERVICE_ACCOUNT_FILE = "gclient.json"

# GCS bucket configuration
GCS_BUCKET_NAME = "blockbrowser"


def _get_platform_dir(platform_override: Optional[str] = None) -> str:
    """Get platform directory name for GCS path"""
    if platform_override:
        return platform_override

    if IS_WINDOWS:
        return "win"
    elif IS_MACOS:
        return "macos"
    else:
        return "linux"


def upload_to_gcs(
    ctx: BuildContext,
    file_paths: List[Path],
    platform_override: Optional[str] = None
) -> Tuple[bool, List[str]]:
    """Upload build artifacts to Google Cloud Storage

    Args:
        ctx: BuildContext with root_dir and blockbrowser_version
        file_paths: List of file paths to upload
        platform_override: Optional platform override (macos/linux/win)

    Returns:
        (success, list of GCS URIs)
    """
    if not GCS_AVAILABLE:
        log_warning("google-cloud-storage not installed. Skipping GCS upload.")
        log_info("Install with: pip install google-cloud-storage")
        return True, []  # Not a fatal error

    if not file_paths:
        log_info("No files to upload to GCS")
        return True, []

    # Determine platform subdirectory
    platform_dir = _get_platform_dir(platform_override)

    # Build GCS path: gs://blockbrowser/resources/<version>/<platform>/
    gcs_prefix = f"resources/{ctx.blockbrowser_version}/{platform_dir}"

    log_info(f"\n☁️  Uploading artifacts to gs://{GCS_BUCKET_NAME}/{gcs_prefix}/")

    # Check for service account file
    service_account_path = join_paths(ctx.root_dir, SERVICE_ACCOUNT_FILE)
    if not service_account_path.exists():
        log_error(f"Service account file not found: {SERVICE_ACCOUNT_FILE}")
        log_info(
            f"Please place the service account JSON file at: {service_account_path}"
        )
        return False, []

    try:
        # Initialize GCS client with service account
        credentials = service_account.Credentials.from_service_account_file(
            str(service_account_path)
        )
        client = storage.Client(credentials=credentials)
        bucket = client.bucket(GCS_BUCKET_NAME)

        uploaded_files = []
        gcs_uris = []

        for file_path in file_paths:
            if not file_path.exists():
                log_warning(f"File not found, skipping: {file_path}")
                continue

            # Determine blob name (file name in GCS)
            blob_name = f"{gcs_prefix}/{file_path.name}"

            try:
                blob = bucket.blob(blob_name)

                log_info(f"📤 Uploading {file_path.name}...")
                blob.upload_from_filename(str(file_path))

                # Note: With uniform bucket-level access, objects inherit bucket's IAM policies
                # No need to set individual object ACLs

                public_url = f"https://storage.googleapis.com/{GCS_BUCKET_NAME}/{blob_name}"
                gcs_uri = f"gs://{GCS_BUCKET_NAME}/{blob_name}"
                uploaded_files.append(public_url)
                gcs_uris.append(gcs_uri)
                log_success(f"✓ Uploaded: {public_url}")

            except Exception as e:
                log_error(f"Failed to upload {file_path.name}: {e}")
                return False, []

        if uploaded_files:
            log_success(
                f"\n☁️  Successfully uploaded {len(uploaded_files)} file(s) to GCS"
            )
            log_info("\nPublic URLs:")
            for url in uploaded_files:
                log_info(f"  {url}")

        return True, gcs_uris

    except Exception as e:
        log_error(f"GCS upload failed: {e}")
        return False, []


def upload_package_artifacts(ctx: BuildContext) -> tuple[bool, List[str]]:
    """Upload package artifacts (DMG, ZIP, EXE) to GCS
    Returns: (success, list of GCS URIs)"""
    log_info("\n☁️  Preparing to upload package artifacts to GCS...")

    artifacts = []

    # Look for files in the dist/<version> directory
    dist_dir = ctx.get_dist_dir()
    if dist_dir.exists():
        if IS_MACOS:
            # Look for DMG files
            artifacts.extend(dist_dir.glob("*.dmg"))
        elif IS_WINDOWS:
            # Look for installer and ZIP files
            artifacts.extend(dist_dir.glob("*.exe"))
            artifacts.extend(dist_dir.glob("*.zip"))
        else:  # Linux
            # Look for AppImage and .deb files
            artifacts.extend(dist_dir.glob("*.AppImage"))
            artifacts.extend(dist_dir.glob("*.deb"))

    if not artifacts:
        log_info("No package artifacts found to upload")
        return True, []

    log_info(f"Found {len(artifacts)} artifact(s) to upload:")
    for artifact in artifacts:
        log_info(f"  - {artifact.name}")

    return upload_to_gcs(ctx, artifacts)


def upload_signed_artifacts(ctx: BuildContext) -> bool:
    """Upload signed artifacts to GCS"""
    # For now, this is the same as package artifacts
    # Can be extended in the future for specific signed artifacts
    return upload_package_artifacts(ctx)


def download_from_gcs(
    bucket_name: str,
    source_path: str,
    dest_path: Path,
    ctx: Optional[BuildContext] = None,
) -> bool:
    """Download a file from GCS (utility function)"""
    if not GCS_AVAILABLE:
        log_error("google-cloud-storage not installed")
        return False

    try:
        # Try to use service account if available
        client = None
        if ctx:
            service_account_path = join_paths(ctx.root_dir, SERVICE_ACCOUNT_FILE)
            if service_account_path.exists():
                credentials = service_account.Credentials.from_service_account_file(
                    str(service_account_path)
                )
                client = storage.Client(credentials=credentials)

        # Fall back to anonymous client for public buckets
        if not client:
            client = storage.Client.create_anonymous_client()

        bucket = client.bucket(bucket_name)
        blob = bucket.blob(source_path)

        log_info(f"📥 Downloading gs://{bucket_name}/{source_path}...")
        blob.download_to_filename(str(dest_path))
        log_success(f"Downloaded to: {dest_path}")
        return True

    except Exception as e:
        log_error(f"Failed to download from GCS: {e}")
        return False


def _detect_artifacts(dist_path: Path, platform_override: Optional[str] = None) -> List[Path]:
    """Detect artifacts in a dist directory based on platform

    Args:
        dist_path: Path to the dist/<version> directory
        platform_override: Optional platform override (macos/linux/win)

    Returns:
        List of artifact file paths found
    """
    artifacts = []

    # Determine which file types to look for
    if platform_override:
        if platform_override == "macos":
            patterns = ["*.dmg"]
        elif platform_override == "win":
            patterns = ["*.exe", "*.zip"]
        elif platform_override == "linux":
            patterns = ["*.AppImage", "*.deb"]
        else:
            log_error(f"Invalid platform: {platform_override}. Must be macos/linux/win")
            return []
    else:
        # Auto-detect based on current platform
        if IS_MACOS:
            patterns = ["*.dmg"]
        elif IS_WINDOWS:
            patterns = ["*.exe", "*.zip"]
        else:  # Linux
            patterns = ["*.AppImage", "*.deb"]

    # Find all matching files
    for pattern in patterns:
        artifacts.extend(dist_path.glob(pattern))

    return sorted(artifacts)


def handle_upload_dist(
    dist_path: Path,
    root_dir: Path,
    platform_override: Optional[str] = None
) -> bool:
    """Upload pre-built artifacts from a dist directory to GCS

    This is the main entry point for manual uploads of already-built artifacts.

    Args:
        dist_path: Path to dist/<version> directory containing artifacts
        root_dir: Root directory of the project (for finding gclient.json)
        platform_override: Optional platform override (macos/linux/win)

    Returns:
        True if successful, False otherwise

    Example:
        handle_upload_dist(Path("dist/61"), Path("."), platform_override="macos")
    """
    log_info("=" * 60)
    log_info("📤 Manual GCS Upload")
    log_info("=" * 60)

    # 1. Validate dist_path exists
    if not dist_path.exists():
        log_error(f"Distribution directory does not exist: {dist_path}")
        return False

    if not dist_path.is_dir():
        log_error(f"Path is not a directory: {dist_path}")
        return False

    # 2. Extract version from path (assume dist/<version> structure)
    version = dist_path.name
    log_info(f"📦 Version detected: {version}")

    # 3. Determine platform
    platform_dir = _get_platform_dir(platform_override)
    if platform_override:
        log_info(f"🖥️  Platform (override): {platform_dir}")
    else:
        log_info(f"🖥️  Platform (auto-detected): {platform_dir}")

    # 4. Scan for artifacts
    log_info(f"\n🔍 Scanning for artifacts in: {dist_path}")
    artifacts = _detect_artifacts(dist_path, platform_override)

    if not artifacts:
        log_warning("No artifacts found to upload")
        log_info("\nExpected file types by platform:")
        log_info("  - macOS: *.dmg")
        log_info("  - Windows: *.exe, *.zip")
        log_info("  - Linux: *.AppImage")
        return False

    # 5. Preview files
    log_info(f"\n📋 Found {len(artifacts)} artifact(s):")
    total_size = 0
    for artifact in artifacts:
        size_mb = artifact.stat().st_size / (1024 * 1024)
        total_size += size_mb
        log_info(f"  - {artifact.name} ({size_mb:.2f} MB)")

    log_info(f"\nTotal size: {total_size:.2f} MB")
    log_info(f"Upload destination: gs://{GCS_BUCKET_NAME}/resources/{version}/{platform_dir}/")

    # 6. Create minimal BuildContext for upload
    # BuildContext will try to load chromium_src, but we'll provide a dummy one
    # since we don't need it for uploads
    try:
        ctx = BuildContext(
            root_dir=root_dir,
            chromium_src=Path("/dev/null"),  # Dummy path, won't be used
            architecture="",  # Not needed for upload
            build_type="release",  # Not needed for upload
        )
        # Override the version with what we detected
        ctx.blockbrowser_version = version
    except Exception as e:
        # If BuildContext fails, we can still upload with minimal info
        log_warning(f"Could not create full BuildContext: {e}")
        log_info("Creating minimal context for upload...")

        # Create a simple object with just what we need
        class MinimalContext:
            def __init__(self, root_dir: Path, version: str):
                self.root_dir = root_dir
                self.blockbrowser_version = version

        ctx = MinimalContext(root_dir, version)

    # 7. Upload using existing upload_to_gcs function
    success, gcs_uris = upload_to_gcs(ctx, artifacts, platform_override=platform_override)

    if success:
        log_success("\n✅ Upload completed successfully!")
        if gcs_uris:
            log_info("\nUploaded URIs:")
            for uri in gcs_uris:
                log_info(f"  {uri}")
        return True
    else:
        log_error("\n❌ Upload failed")
        return False
