#!/usr/bin/env python3
"""Appcast module - Generate Sparkle appcast XML snippets"""

from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_warning
from ..storage import BOTO3_AVAILABLE
from .common import fetch_all_release_metadata, generate_appcast_item


class AppcastModule(CommandModule):
    """Generate appcast XML snippets for macOS auto-update"""

    produces = []
    requires = []
    description = "Generate Sparkle appcast XML snippets"

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
        metadata = fetch_all_release_metadata(version, ctx.env)

        if "macos" not in metadata and "win" not in metadata:
            log_info(
                f"No macOS or Windows release metadata found for version {version}"
            )
            return

        log_info(f"\n{'='*60}")
        log_info(f"APPCAST SNIPPETS FOR v{version}")
        log_info(f"{'='*60}")

        if "macos" in metadata:
            self._print_macos_items(metadata["macos"], version)

        if "win" in metadata:
            self._print_windows_items(metadata["win"], version)

        log_info(f"\n{'='*60}")

    def _print_macos_items(self, release: dict, version: str) -> None:
        sparkle_version = release.get("sparkle_version", "")
        build_date = release.get("build_date", "")
        artifacts = release.get("artifacts", {})

        arch_to_file = {
            "arm64": "appcast.xml",
            "x64": "appcast-x86_64.xml",
            "universal": "appcast.xml",
        }

        for arch in ["arm64", "x64", "universal"]:
            if arch not in artifacts:
                continue

            artifact = artifacts[arch]
            if "sparkle_signature" not in artifact:
                log_warning(f"{arch} artifact missing sparkle_signature")

            log_info(f"\n{arch_to_file[arch]} ({arch}):")
            print(generate_appcast_item(artifact, version, sparkle_version, build_date))

    def _print_windows_items(self, release: dict, version: str) -> None:
        sparkle_version = release.get("sparkle_version", "")
        build_date = release.get("build_date", "")
        artifacts = release.get("artifacts", {})

        # WinSparkle feeds, one per arch (chrome/browser/win/winsparkle_glue.cc
        # picks the matching URL at compile time).
        key_to_file = {
            "x64_installer": "appcast-win.xml",
            "arm64_installer": "appcast-win-arm64.xml",
        }

        for key, appcast_file in key_to_file.items():
            if key not in artifacts:
                continue

            artifact = artifacts[key]
            if "sparkle_signature" not in artifact:
                log_warning(f"{key} artifact missing sparkle_signature")

            log_info(f"\n{appcast_file} ({key}):")
            print(
                generate_appcast_item(
                    artifact, version, sparkle_version, build_date, platform="win"
                )
            )
