#!/usr/bin/env python3
"""Server OTA module for BrowserOS Server binary updates"""

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
    create_server_zip,
    upload_to_r2,
    get_appcast_path,
)
from .sign_binary import (
    sign_macos_binary,
    notarize_macos_binary,
    sign_windows_binary,
    get_entitlements_path,
)


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
        skip_sign: bool = False,
        skip_upload: bool = False,
        platform_filter: Optional[str] = None,
    ):
        """
        Args:
            version: Version string (e.g., "0.0.36")
            channel: Release channel ("alpha" or "prod")
            binaries_dir: Directory containing server binaries
            skip_sign: Skip binary signing (for already-signed binaries)
            skip_upload: Skip R2 upload (for local testing)
            platform_filter: Only process specific platform (e.g., "darwin_arm64")
        """
        self.version = version
        self.channel = channel
        self.binaries_dir = binaries_dir
        self.skip_sign = skip_sign
        self.skip_upload = skip_upload
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

        if not self.skip_sign:
            if IS_MACOS():
                env = ctx.env
                if not env.macos_certificate_name:
                    raise ValidationError("MACOS_CERTIFICATE_NAME required for signing")
            elif IS_WINDOWS():
                env = ctx.env
                if not env.code_sign_tool_path:
                    raise ValidationError("CODE_SIGN_TOOL_PATH required for signing")

        if not self.skip_upload:
            try:
                import subprocess
                result = subprocess.run(
                    ["wrangler", "--version"],
                    capture_output=True,
                    check=False,
                )
                if result.returncode != 0:
                    raise ValidationError("wrangler not installed")
            except FileNotFoundError:
                raise ValidationError("wrangler not installed")

    def _get_platforms(self) -> List[dict]:
        """Get platforms to process based on filter"""
        if self.platform_filter:
            return [p for p in SERVER_PLATFORMS if p["name"] == self.platform_filter]
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
            binary_path = self.binaries_dir / binary_name

            if not self.skip_sign:
                if not self._sign_binary(binary_path, platform, ctx):
                    log_warning(f"Skipping {platform['name']} due to signing failure")
                    continue

            zip_name = f"browseros_server_{self.version}_{platform['name']}.zip"
            zip_path = temp_dir / zip_name
            is_windows = platform["os"] == "windows"

            if not create_server_zip(binary_path, zip_path, is_windows):
                log_error(f"Failed to create zip for {platform['name']}")
                continue

            log_info(f"Signing {zip_name} with Sparkle...")
            signature, length = sparkle_sign_file(zip_path, ctx.env, ctx.chromium_src)

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

        if not self.skip_upload:
            log_info("\n📤 Uploading to R2...")
            for artifact in signed_artifacts:
                r2_path = f"browseros/server/{artifact.zip_path.name}"
                if not upload_to_r2(artifact.zip_path, r2_path, ctx.env):
                    log_error(f"Failed to upload {artifact.zip_path.name}")

        log_info("\n📝 Generating appcast...")
        appcast_content = generate_server_appcast(
            self.version,
            signed_artifacts,
            self.channel,
        )

        appcast_path = get_appcast_path(self.channel)
        appcast_path.parent.mkdir(parents=True, exist_ok=True)
        appcast_path.write_text(appcast_content)
        log_success(f"Appcast saved to: {appcast_path}")

        if not self.skip_upload:
            r2_appcast_name = f"appcast-server.{self.channel}.xml" if self.channel == "alpha" else "appcast-server.xml"
            r2_appcast_path = f"browseros/{r2_appcast_name}"

            log_info(f"Uploading appcast to {r2_appcast_path}...")
            upload_to_r2(appcast_path, r2_appcast_path, ctx.env)

        ctx.artifacts["server_ota_artifacts"] = signed_artifacts
        ctx.artifacts["server_appcast"] = appcast_path

        log_info("\n" + "=" * 70)
        log_success(f"✅ Server OTA v{self.version} ({self.channel}) complete!")
        log_info("=" * 70)

        log_info("\nURLs:")
        appcast_url = f"https://cdn.browseros.com/appcast-server.{self.channel}.xml" if self.channel == "alpha" else "https://cdn.browseros.com/appcast-server.xml"
        log_info(f"  {appcast_url}")
        for artifact in signed_artifacts:
            log_info(f"  https://cdn.browseros.com/server/{artifact.zip_path.name}")

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
            if not IS_WINDOWS():
                log_warning(f"Windows signing requires Windows - skipping {platform['name']}")
                return True

            return sign_windows_binary(binary_path, ctx.env)

        elif os_type == "linux":
            log_info(f"No code signing for Linux binaries")
            return True

        return True
