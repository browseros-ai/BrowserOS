#!/usr/bin/env python3
"""Environment variable configuration for BrowserOS builds."""

import os
from typing import Optional

from dotenv import load_dotenv


# Keep this registry beside EnvConfig's credential properties so logging code
# has one package-wide source of truth for values that must never reach output.
SENSITIVE_ENV_VARS: frozenset[str] = frozenset(
    {
        "BROWSERCLAW_KEY",
        "BROWSEROS_AGENT_V2_KEY",
        "BROWSEROS_CONTROLLER_KEY",
        "BUGREPORTER_KEY",
        "CLOUDFLARE_API_TOKEN",
        "ESIGNER_PASSWORD",
        "ESIGNER_TOTP_SECRET",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "MACOS_CERTIFICATE_P12",
        "MACOS_CERTIFICATE_PWD",
        "MACOS_KEYCHAIN_PASSWORD",
        "POSTHOG_API_KEY",
        "PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64",
        "PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64",
        "PROD_MACOS_NOTARIZATION_PWD",
        "R2_SECRET_ACCESS_KEY",
        "SENTRY_AUTH_TOKEN",
        "SLACK_WEBHOOK_URL",
        "SPARKLE_PRIVATE_KEY",
    }
)


def _load_dotenv_file():
    """Load .env file from project root"""
    from .paths import get_package_root

    browseros_root = get_package_root()
    project_root = browseros_root.parent.parent
    env_locations = [
        browseros_root / ".env",
        project_root / ".env",
    ]

    for env_path in env_locations:
        if env_path.exists():
            load_dotenv(env_path)
            return


_load_dotenv_file()


class EnvConfig:
    """Centralize build environment access and defaults."""

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
    def browseros_server_resource_version(self) -> Optional[str]:
        """Exact BrowserOS server resource version for release builds."""
        return os.environ.get("BROWSEROS_SERVER_RESOURCE_VERSION")

    @property
    def browserclaw_server_resource_version(self) -> Optional[str]:
        """Exact BrowserClaw server resource version for release builds."""
        return os.environ.get("BROWSERCLAW_SERVER_RESOURCE_VERSION")

    @property
    def onboarding_resource_version(self) -> Optional[str]:
        """Exact product-selected onboarding version for release builds."""
        return os.environ.get("ONBOARDING_RESOURCE_VERSION")

    @property
    def bundled_extensions_manifest_url(self) -> Optional[str]:
        """Run-scoped bundled extension manifest URL for release builds."""
        return os.environ.get("BUNDLED_EXTENSIONS_MANIFEST_URL")

    @property
    def bundled_product_extension_version(self) -> Optional[str]:
        """Exact product extension version for release builds."""
        return os.environ.get("BUNDLED_PRODUCT_EXTENSION_VERSION")

    @property
    def depot_tools_win_toolchain(self) -> str:
        """Windows depot_tools toolchain setting (0 = use system toolchain)"""
        return os.environ.get("DEPOT_TOOLS_WIN_TOOLCHAIN", "0")

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

    @property
    def macos_browseros_passkey_profile_path(self) -> Optional[str]:
        """Developer ID profile authorizing BrowserOS platform passkeys."""
        return os.environ.get("PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_PATH")

    @property
    def macos_browserclaw_passkey_profile_path(self) -> Optional[str]:
        """Developer ID profile authorizing BrowserOS neo platform passkeys."""
        return os.environ.get("PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_PATH")

    @property
    def macos_keychain_password(self) -> Optional[str]:
        """macOS login keychain password (used to unlock keychain on build servers)"""
        return os.environ.get("MACOS_KEYCHAIN_PASSWORD")

    @property
    def macos_keychain_path(self) -> Optional[str]:
        """Explicit macOS signing keychain path."""
        return os.environ.get("MACOS_KEYCHAIN_PATH")

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

    @property
    def sparkle_private_key(self) -> Optional[str]:
        """Base64-encoded Sparkle Ed25519 private key for macOS auto-update signing"""
        return os.environ.get("SPARKLE_PRIVATE_KEY")

    @property
    def sparkle_sign_update_path(self) -> Optional[str]:
        """Path to Sparkle sign_update tool (overrides auto-detection)"""
        return os.environ.get("SPARKLE_SIGN_UPDATE_PATH")

    @property
    def slack_webhook_url(self) -> Optional[str]:
        """Slack webhook URL for build notifications"""
        return os.environ.get("SLACK_WEBHOOK_URL")

    def get_macos_signing_config(self) -> dict:
        """Return the macOS signing configuration."""
        return {
            "certificate_name": self.macos_certificate_name or "",
            "apple_id": self.macos_notarization_apple_id or "",
            "team_id": self.macos_notarization_team_id or "",
            "notarization_pwd": self.macos_notarization_password or "",
        }

    def get_windows_signing_config(self) -> dict:
        """Return the Windows signing configuration."""
        return {
            "code_sign_tool_path": self.code_sign_tool_path or "",
            "username": self.esigner_username or "",
            "password": self.esigner_password or "",
            "totp_secret": self.esigner_totp_secret or "",
            "credential_id": self.esigner_credential_id or "",
        }

    def validate_required(self, *var_names: str) -> None:
        """Require the named environment variables."""
        missing = []
        for var_name in var_names:
            env_var = var_name.upper()
            if not os.environ.get(env_var):
                missing.append(env_var)

        if missing:
            raise ValueError(
                f"Missing required environment variables: {', '.join(missing)}"
            )

    def get_r2_config(self) -> dict:
        """Return the R2 configuration."""
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
