#!/usr/bin/env python3
"""Cloudflare R2 client utilities for BrowserOS build system

This module provides shared R2 (S3-compatible) operations used by both
upload and download modules.
"""

import json
from pathlib import Path
from typing import Dict, Optional

from .env import EnvConfig
from .utils import log_info, log_error, log_success, log_warning

# Try to import boto3 for R2 (S3-compatible)
try:
    import boto3
    from botocore.config import Config

    BOTO3_AVAILABLE = True
except ImportError:
    BOTO3_AVAILABLE = False


def get_r2_client(env: Optional[EnvConfig] = None):
    """Create boto3 S3 client configured for R2

    Args:
        env: Optional EnvConfig instance. If not provided, creates a new one.

    Returns:
        boto3 S3 client configured for R2, or None if not available
    """
    if not BOTO3_AVAILABLE:
        log_error("boto3 not installed - run: pip install boto3")
        return None

    if env is None:
        env = EnvConfig()

    if not env.has_r2_config():
        log_error("R2 configuration not set")
        return None

    return boto3.client(
        "s3",
        endpoint_url=env.r2_endpoint_url,
        aws_access_key_id=env.r2_access_key_id,
        aws_secret_access_key=env.r2_secret_access_key,
        config=Config(
            signature_version="s3v4",
            retries={"max_attempts": 3, "mode": "standard"},
        ),
    )


def upload_file_to_r2(
    client,
    local_path: Path,
    r2_key: str,
    bucket: str,
) -> bool:
    """Upload a single file to R2

    Args:
        client: boto3 S3 client
        local_path: Path to local file
        r2_key: Key (path) in R2 bucket
        bucket: R2 bucket name

    Returns:
        True if successful, False otherwise
    """
    try:
        log_info(f"Uploading {local_path.name}...")
        client.upload_file(str(local_path), bucket, r2_key)
        log_success(f"Uploaded: {r2_key}")
        return True
    except Exception as e:
        log_error(f"Failed to upload {local_path.name}: {e}")
        return False


def download_file_from_r2(
    client,
    r2_key: str,
    dest_path: Path,
    bucket: str,
) -> bool:
    """Download a single file from R2

    Args:
        client: boto3 S3 client
        r2_key: Key (path) in R2 bucket
        dest_path: Local destination path
        bucket: R2 bucket name

    Returns:
        True if successful, False otherwise
    """
    try:
        log_info(f"Downloading {r2_key}...")
        dest_path.parent.mkdir(parents=True, exist_ok=True)
        client.download_file(bucket, r2_key, str(dest_path))
        log_success(f"Downloaded: {dest_path.name}")
        return True
    except Exception as e:
        log_error(f"Failed to download {r2_key}: {e}")
        return False


def download_from_r2(
    r2_key: str,
    dest_path: Path,
    bucket: Optional[str] = None,
    env: Optional[EnvConfig] = None,
) -> bool:
    """Download a file from R2 (convenience wrapper)

    Args:
        r2_key: Key (path) in R2 bucket
        dest_path: Local destination path
        bucket: Optional bucket name (uses default from env if not specified)
        env: Optional EnvConfig instance. If not provided, creates a new one.

    Returns:
        True if successful, False otherwise
    """
    if not BOTO3_AVAILABLE:
        log_error("boto3 not installed")
        return False

    if env is None:
        env = EnvConfig()

    if not env.has_r2_config():
        log_error("R2 configuration not set")
        return False

    client = get_r2_client(env)
    if not client:
        return False

    bucket = bucket or env.r2_bucket
    return download_file_from_r2(client, r2_key, dest_path, bucket)


def get_release_json(
    version: str,
    platform: str,
    env: Optional[EnvConfig] = None,
    product_id: str = "browseros",
) -> Optional[Dict]:
    """Fetch release.json for a specific version and platform from R2

    Args:
        version: Semantic version (e.g., "0.31.0")
        platform: Platform name (macos, win, linux)
        env: Optional EnvConfig instance. If not provided, creates a new one.

    Returns:
        Parsed release.json dict, or None if not found
    """
    if not BOTO3_AVAILABLE:
        log_error("boto3 not installed")
        return None

    if env is None:
        env = EnvConfig()

    if not env.has_r2_config():
        log_error("R2 configuration not set")
        return None

    client = get_r2_client(env)
    if not client:
        return None

    keys = [f"releases/{product_id}/{version}/{platform}/release.json"]
    if product_id == "browseros":
        keys.append(f"releases/{version}/{platform}/release.json")

    for r2_key in keys:
        try:
            response = client.get_object(Bucket=env.r2_bucket, Key=r2_key)
            content = response["Body"].read().decode("utf-8")
            return json.loads(content)
        except client.exceptions.NoSuchKey:
            continue
        except Exception as e:
            log_error(f"Failed to fetch release.json: {e}")
            return None

    log_warning(f"release.json not found: {keys[0]}")
    return None
