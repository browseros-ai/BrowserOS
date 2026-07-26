#!/usr/bin/env python3
"""Self-contained env + logging + R2 helpers for the vendor upload tool.

Deliberately duplicates a slice of bos_build (EnvConfig R2 fields,
log helpers, R2 client) so this operational tool has zero dependency
on the build system package.
"""

import os
from pathlib import Path
from typing import Optional

from dotenv import load_dotenv

try:
    import boto3
    from botocore.config import Config

    BOTO3_AVAILABLE = True
except ImportError:
    BOTO3_AVAILABLE = False


def _load_dotenv_file() -> None:
    """Load .env from cwd or any parent, mirroring the build system."""
    current = Path.cwd()
    for candidate in [current, *current.parents]:
        env_path = candidate / ".env"
        if env_path.exists():
            load_dotenv(env_path)
            return


_load_dotenv_file()


def log_info(message: str) -> None:
    print(message, flush=True)


def log_success(message: str) -> None:
    print(f"\033[92m{message}\033[0m", flush=True)


def log_warning(message: str) -> None:
    print(f"\033[93m{message}\033[0m", flush=True)


def log_error(message: str) -> None:
    print(f"\033[91m{message}\033[0m", flush=True)


class EnvConfig:
    """R2 credentials from the environment (subset of bos_build's EnvConfig)."""

    @property
    def r2_account_id(self) -> Optional[str]:
        return os.environ.get("R2_ACCOUNT_ID")

    @property
    def r2_access_key_id(self) -> Optional[str]:
        return os.environ.get("R2_ACCESS_KEY_ID")

    @property
    def r2_secret_access_key(self) -> Optional[str]:
        return os.environ.get("R2_SECRET_ACCESS_KEY")

    @property
    def r2_bucket(self) -> str:
        return os.environ.get("R2_BUCKET", "browseros")

    @property
    def r2_endpoint_url(self) -> Optional[str]:
        account_id = self.r2_account_id
        if not account_id:
            return None
        return f"https://{account_id}.r2.cloudflarestorage.com"

    def has_r2_config(self) -> bool:
        return bool(
            self.r2_account_id and self.r2_access_key_id and self.r2_secret_access_key
        )


def get_r2_client(env: Optional[EnvConfig] = None):
    """Create boto3 S3 client configured for R2, or None if unavailable."""
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


def upload_file_to_r2(client, local_path: Path, r2_key: str, bucket: str) -> bool:
    """Upload a single file to R2; returns False on failure."""
    try:
        log_info(f"Uploading {local_path.name}...")
        client.upload_file(str(local_path), bucket, r2_key)
        log_success(f"Uploaded: {r2_key}")
        return True
    except Exception as e:
        log_error(f"Failed to upload {local_path.name}: {e}")
        return False
