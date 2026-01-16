#!/usr/bin/env python3
"""
Environment variable configuration for BrowserOS build system

This module provides centralized access to all environment variables used by the build system.
It provides type-safe access, defaults, and clear documentation of what each variable is for.

The module automatically loads .env files from the project root on import.
"""

import os
from pathlib import Path
from typing import Optional

from dotenv import load_dotenv


def _load_dotenv_file():
    """Load .env file from project root"""
    from .paths import get_package_root

    browseros_root = get_package_root()
    project_root = browseros_root.parent.parent  # repo root

    # Try loading .env from multiple locations (most specific first)
    env_locations = [
        browseros_root / ".env",  # packages/browseros/.env
        project_root / ".env",  # repo root .env
    ]

    for env_path in env_locations:
        if env_path.exists():
            load_dotenv(env_path)
            return


# Load .env on module import
_load_dotenv_file()


class EnvConfig:
    """
    Centralized environment variable configuration

    This class provides clean, type-safe access to all environment variables
    used by the build system. It serves as the single source of truth for
    what environment variables are available and what they're used for.

    Usage:
        env = EnvConfig()
        if env.chromium_src:
            chromium_path = Path(env.chromium_src)
    """

    # === Build Configuration ===

    @property
    def chromium_src(self) -> Optional[str]:
        """Path to Chromium source directory"""
        return os.environ.get("CHROMIUM_SRC")

    @property
    def arch(self) -> Optional[str]:
        """Target architecture (x64, arm64, universal)"""
        return os.environ.get("ARCH")

    @property
    def pythonpath(self) -> Optional[str]:
        """Python path for build scripts"""
        return os.environ.get("PYTHONPATH")

    @property
    def depot_tools_win_toolchain(self) -> str:
        """Windows depot_tools toolchain setting (0 = use system toolchain)"""
        return os.environ.get("DEPOT_TOOLS_WIN_TOOLCHAIN", "0")

    # === macOS Code Signing ===

    @property
    def macos_certificate_name(self) -> Optional[str]:
        """macOS code signing certificate name"""
        return os.environ.get("MACOS_CERTIFICATE_NAME")

    @property
    def macos_notarization_apple_id(self) -> Optional[str]:
        """Apple ID for macOS notarization"""
        return os.environ.get("PROD_MACOS_NOTARIZATION_APPLE_ID")

    @property
    def macos_notarization_team_id(self) -> Optional[str]:
        """Team ID for macOS notarization"""
        return os.environ.get("PROD_MACOS_NOTARIZATION_TEAM_ID")

    @property
    def macos_notarization_password(self) -> Optional[str]:
        """App-specific password for macOS notarization"""
        return os.environ.get("PROD_MACOS_NOTARIZATION_PWD")

    # === Windows Code Signing ===

    @property
    def code_sign_tool_path(self) -> Optional[str]:
        """Path to Windows code signing tool directory (legacy, use CODE_SIGN_TOOL_EXE instead)"""
        return os.environ.get("CODE_SIGN_TOOL_PATH")

    @property
    def code_sign_tool_exe(self) -> Optional[str]:
        """Path to CodeSignTool executable (CodeSignTool.sh on macOS/Linux, CodeSignTool.bat on Windows)"""
        return os.environ.get("CODE_SIGN_TOOL_EXE")

    @property
    def esigner_username(self) -> Optional[str]:
        """eSigner username for Windows code signing"""
        return os.environ.get("ESIGNER_USERNAME")

    @property
    def esigner_password(self) -> Optional[str]:
        """eSigner password for Windows code signing"""
        return os.environ.get("ESIGNER_PASSWORD")

    @property
    def esigner_totp_secret(self) -> Optional[str]:
        """eSigner TOTP secret for Windows code signing"""
        return os.environ.get("ESIGNER_TOTP_SECRET")

    @property
    def esigner_credential_id(self) -> Optional[str]:
        """eSigner credential ID for Windows code signing"""
        return os.environ.get("ESIGNER_CREDENTIAL_ID")

    # === Upload & Distribution (Cloudflare R2) ===

    @property
    def r2_account_id(self) -> Optional[str]:
        """Cloudflare account ID for R2"""
        return os.environ.get("R2_ACCOUNT_ID")

    @property
    def r2_access_key_id(self) -> Optional[str]:
        """R2 access key ID"""
        return os.environ.get("R2_ACCESS_KEY_ID")

    @property
    def r2_secret_access_key(self) -> Optional[str]:
        """R2 secret access key"""
        return os.environ.get("R2_SECRET_ACCESS_KEY")

    @property
    def r2_bucket(self) -> str:
        """R2 bucket name (default: browseros)"""
        return os.environ.get("R2_BUCKET", "browseros")

    @property
    def r2_cdn_base_url(self) -> str:
        """CDN base URL for R2 artifacts (default: http://cdn.browseros.com)"""
        return os.environ.get("R2_CDN_BASE_URL", "http://cdn.browseros.com")

    @property
    def r2_endpoint_url(self) -> Optional[str]:
        """R2 S3-compatible endpoint URL (computed from account ID)"""
        account_id = self.r2_account_id
        if account_id:
            return f"https://{account_id}.r2.cloudflarestorage.com"
        return None

    # === Sparkle Signing (macOS) ===

    @property
    def sparkle_private_key(self) -> Optional[str]:
        """Base64-encoded Sparkle Ed25519 private key for macOS auto-update signing"""
        return os.environ.get("SPARKLE_PRIVATE_KEY")

    @property
    def sparkle_sign_update_path(self) -> Optional[str]:
        """Path to Sparkle sign_update tool (overrides auto-detection)"""
        return os.environ.get("SPARKLE_SIGN_UPDATE_PATH")

    # === Notifications ===

    @property
    def slack_webhook_url(self) -> Optional[str]:
        """Slack webhook URL for build notifications"""
        return os.environ.get("SLACK_WEBHOOK_URL")

    # === Helper Methods ===

    def get_macos_signing_config(self) -> dict:
        """
        Get all macOS signing configuration as a dict

        Returns:
            dict with keys: certificate_name, apple_id, team_id, notarization_pwd
        """
        return {
            "certificate_name": self.macos_certificate_name or "",
            "apple_id": self.macos_notarization_apple_id or "",
            "team_id": self.macos_notarization_team_id or "",
            "notarization_pwd": self.macos_notarization_password or "",
        }

    def get_windows_signing_config(self) -> dict:
        """
        Get all Windows signing configuration as a dict

        Returns:
            dict with keys: code_sign_tool_path, username, password, totp_secret, credential_id
        """
        return {
            "code_sign_tool_path": self.code_sign_tool_path or "",
            "username": self.esigner_username or "",
            "password": self.esigner_password or "",
            "totp_secret": self.esigner_totp_secret or "",
            "credential_id": self.esigner_credential_id or "",
        }

    def validate_required(self, *var_names: str) -> None:
        """
        Validate that required environment variables are set

        Args:
            *var_names: Variable names to check (e.g., "chromium_src", "gcs_bucket")

        Raises:
            ValueError: If any required variable is not set

        Example:
            env = EnvConfig()
            env.validate_required("chromium_src", "macos_certificate_name")
        """
        missing = []
        for var_name in var_names:
            # Convert property name to env var name (e.g., chromium_src -> CHROMIUM_SRC)
            env_var = var_name.upper()
            if not os.environ.get(env_var):
                missing.append(env_var)

        if missing:
            raise ValueError(
                f"Missing required environment variables: {', '.join(missing)}"
            )

    def get_r2_config(self) -> dict:
        """
        Get all R2 configuration as a dict

        Returns:
            dict with keys: account_id, access_key_id, secret_access_key, bucket, cdn_base_url, endpoint_url
        """
        return {
            "account_id": self.r2_account_id or "",
            "access_key_id": self.r2_access_key_id or "",
            "secret_access_key": self.r2_secret_access_key or "",
            "bucket": self.r2_bucket,
            "cdn_base_url": self.r2_cdn_base_url,
            "endpoint_url": self.r2_endpoint_url or "",
        }

    def has_r2_config(self) -> bool:
        """Check if R2 upload configuration is available"""
        return bool(
            self.r2_account_id and self.r2_access_key_id and self.r2_secret_access_key
        )

    def has_sparkle_key(self) -> bool:
        """Check if Sparkle private key is available"""
        return bool(self.sparkle_private_key)
