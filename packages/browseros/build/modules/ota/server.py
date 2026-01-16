#!/usr/bin/env python3
"""Server OTA module for BrowserOS Server binary updates"""

import shutil
import tempfile
from pathlib import Path
from typing import List, Optional

from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import (
    log_info,
    log_error,
    log_success,
    log_warning,
    IS_MACOS,
    IS_WINDOWS,
)

from .common import (
    SERVER_PLATFORMS,
    SignedArtifact,
    sparkle_sign_file,
    generate_server_appcast,
    parse_existing_appcast,
    create_server_zip,
    get_appcast_path,
)
from .sign_binary import (
    sign_macos_binary,
    notarize_macos_binary,
    sign_windows_binary,
    get_entitlements_path,
)
from ..storage import get_r2_client, upload_file_to_r2


class ServerOTAModule(CommandModule):
    """OTA update module for BrowserOS Server binaries

    This module handles the full OTA workflow:
    1. Sign individual binaries (codesign for macOS, CodeSignTool for Windows)
    2. Create zip packages with proper structure
    3. Sign zips with Sparkle Ed25519
    4. Upload to R2
    5. Generate and upload appcast XML
    """

    produces = ["server_ota_artifacts", "server_appcast"]
    requires = []
    description = "Create and upload BrowserOS Server OTA update"

    def __init__(
        self,
        version: str = "",
        channel: str = "alpha",
        binaries_dir: Optional[Path] = None,
        platform_filter: Optional[str] = None,
    ):
        """
        Args:
            version: Version string (e.g., "0.0.36")
            channel: Release channel ("alpha" or "prod")
            binaries_dir: Directory containing server binaries
            platform_filter: Platform(s) to process, comma-separated (e.g., "darwin_arm64,darwin_x64")
        """
        self.version = version
        self.channel = channel
        self.binaries_dir = binaries_dir
        self.platform_filter = platform_filter

    def validate(self, context: Context) -> None:
        ctx = context
        if not self.version:
            raise ValidationError("Version is required")

        if self.channel not in ["alpha", "prod"]:
            raise ValidationError("Channel must be 'alpha' or 'prod'")

        if self.binaries_dir:
            if not self.binaries_dir.exists():
                raise ValidationError(f"Binaries directory not found: {self.binaries_dir}")
        else:
            default_dir = ctx.root_dir / "resources" / "binaries" / "browseros_server"
            if not default_dir.exists():
                raise ValidationError(f"Default binaries directory not found: {default_dir}")
            self.binaries_dir = default_dir

        platforms = self._get_platforms()
        for p in platforms:
            binary_name = p["binary"]
            binary_path = self.binaries_dir / binary_name
            if not binary_path.exists():
                raise ValidationError(f"Binary not found: {binary_path}")

        if IS_MACOS():
            if not ctx.env.macos_certificate_name:
                raise ValidationError("MACOS_CERTIFICATE_NAME required for signing")
        elif IS_WINDOWS():
            if not ctx.env.code_sign_tool_path:
                raise ValidationError("CODE_SIGN_TOOL_PATH required for signing")

        if not ctx.env.has_r2_config():
            raise ValidationError(
                "R2 configuration not set. Required env vars: "
                "R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY"
            )

    def _get_platforms(self) -> List[dict]:
        """Get platforms to process based on filter (supports comma-separated)"""
        if self.platform_filter:
            requested = [p.strip() for p in self.platform_filter.split(",")]
            return [p for p in SERVER_PLATFORMS if p["name"] in requested]
        return SERVER_PLATFORMS

    def execute(self, context: Context) -> None:
        ctx = context
        log_info(f"\n🚀 BrowserOS Server OTA v{self.version} ({self.channel})")
        log_info("=" * 70)

        platforms = self._get_platforms()
        temp_dir = Path(tempfile.mkdtemp())
        log_info(f"Temp directory: {temp_dir}")

        signed_artifacts: List[SignedArtifact] = []

        for platform in platforms:
            log_info(f"\n📦 Processing {platform['name']}...")

            binary_name = platform["binary"]
            source_binary = self.binaries_dir / binary_name

            # Copy binary to temp to preserve original
            temp_binary = temp_dir / binary_name
            shutil.copy2(source_binary, temp_binary)

            if not self._sign_binary(temp_binary, platform, ctx):
                log_warning(f"Skipping {platform['name']} due to signing failure")
                continue

            zip_name = f"browseros_server_{self.version}_{platform['name']}.zip"
            zip_path = temp_dir / zip_name
            is_windows = platform["os"] == "windows"

            if not create_server_zip(temp_binary, zip_path, is_windows):
                log_error(f"Failed to create zip for {platform['name']}")
                continue

            log_info(f"Signing {zip_name} with Sparkle...")
            signature, length = sparkle_sign_file(zip_path, ctx.env)

            if not signature:
                log_error(f"Failed to sign zip for {platform['name']}")
                continue

            log_success(f"  {platform['name']}: {length} bytes")

            artifact = SignedArtifact(
                platform=platform["name"],
                zip_path=zip_path,
                signature=signature,
                length=length,
                os=platform["os"],
                arch=platform["arch"],
            )
            signed_artifacts.append(artifact)

        if not signed_artifacts:
            log_error("No artifacts were processed successfully")
            raise RuntimeError("OTA failed - no artifacts")

        log_info("\n📝 Generating appcast...")
        appcast_path = get_appcast_path(self.channel)
        existing_appcast = parse_existing_appcast(appcast_path)

        appcast_content = generate_server_appcast(
            self.version,
            signed_artifacts,
            self.channel,
            existing=existing_appcast,
        )
        appcast_path.parent.mkdir(parents=True, exist_ok=True)
        appcast_path.write_text(appcast_content)
        log_success(f"Appcast saved to: {appcast_path}")

        log_info("\n📤 Uploading artifacts to R2...")
        r2_client = get_r2_client(ctx.env)
        if not r2_client:
            raise RuntimeError("Failed to create R2 client")

        bucket = ctx.env.r2_bucket
        for artifact in signed_artifacts:
            r2_key = f"server/{artifact.zip_path.name}"
            if not upload_file_to_r2(r2_client, artifact.zip_path, r2_key, bucket):
                raise RuntimeError(f"Failed to upload {artifact.zip_path.name}")

        ctx.artifacts["server_ota_artifacts"] = signed_artifacts
        ctx.artifacts["server_appcast"] = appcast_path

        log_info("\n" + "=" * 70)
        log_success(f"✅ Server OTA v{self.version} ({self.channel}) artifacts ready!")
        log_info("=" * 70)

        log_info("\nArtifact URLs:")
        for artifact in signed_artifacts:
            log_info(f"  https://cdn.browseros.com/server/{artifact.zip_path.name}")

        log_info(f"\nAppcast saved to: {appcast_path}")
        log_info("\n📋 Next step: Run 'browseros ota server release-appcast' to make the release live")

    def _sign_binary(self, binary_path: Path, platform: dict, ctx: Context) -> bool:
        """Sign binary based on platform"""
        os_type = platform["os"]

        if os_type == "macos":
            if not IS_MACOS():
                log_warning(f"macOS signing requires macOS - skipping {platform['name']}")
                return True

            entitlements = get_entitlements_path(ctx.root_dir)
            if not sign_macos_binary(binary_path, ctx.env, entitlements):
                return False

            log_info("Notarizing...")
            return notarize_macos_binary(binary_path, ctx.env)

        elif os_type == "windows":
            return sign_windows_binary(binary_path, ctx.env)

        elif os_type == "linux":
            log_info(f"No code signing for Linux binaries")
            return True

        return True
