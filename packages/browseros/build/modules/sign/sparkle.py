#!/usr/bin/env python3
"""Sparkle signing module for macOS auto-update"""

from pathlib import Path
from typing import Dict, Tuple

from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.sparkle import sparkle_sign_file
from ...common.utils import (
    log_info,
    log_success,
    log_warning,
)


class SparkleSignModule(CommandModule):
    """Sign DMGs with Sparkle for macOS auto-update"""

    produces = ["sparkle_signatures"]
    requires = []
    description = "Sign DMG files with Sparkle Ed25519 key for auto-update"

    def validate(self, ctx: Context) -> None:
        if not ctx.env.has_sparkle_key():
            raise ValidationError(
                "SPARKLE_PRIVATE_KEY environment variable not set"
            )

    def execute(self, ctx: Context) -> None:
        log_info("\n🔐 Signing DMGs with Sparkle...")

        # Find DMG files in dist directory
        dist_dir = ctx.get_dist_dir()
        if not dist_dir.exists():
            log_warning(f"Dist directory not found: {dist_dir}")
            return

        dmg_files = list(dist_dir.glob("*.dmg"))
        if not dmg_files:
            log_warning("No DMG files found to sign")
            return

        # Sign each DMG and collect signatures
        signatures = sign_dmgs_with_sparkle(ctx, dmg_files)

        # Store signatures in artifact registry for upload module
        for filename, (sig, length) in signatures.items():
            ctx.artifact_registry.add(f"sparkle_sig_{filename}", Path(filename))
            log_info(f"  {filename}: sig={sig[:20]}... length={length}")

        # Store signatures for upload module to access via ctx.artifacts
        ctx.artifacts["sparkle_signatures"] = signatures

        log_success(f"✅ Signed {len(signatures)} DMG(s) with Sparkle")


def sign_dmgs_with_sparkle(
    ctx: Context,
    dmg_files: list,
) -> Dict[str, Tuple[str, int]]:
    """Sign DMG files with Sparkle and return signatures

    Args:
        ctx: Build context
        dmg_files: List of DMG file paths to sign

    Returns:
        Dict mapping filename to (signature, length) tuple
    """
    signatures = {}

    for dmg_path in dmg_files:
        log_info(f"🔐 Signing {dmg_path.name}...")
        sig, length = sparkle_sign_file(dmg_path, ctx.env)
        if sig:
            signatures[dmg_path.name] = (sig, length)
            log_success(f"✓ Signed {dmg_path.name}")

    return signatures


def get_sparkle_signatures(ctx: Context) -> Dict[str, Tuple[str, int]]:
    """Get stored Sparkle signatures from context

    Args:
        ctx: Build context

    Returns:
        Dict mapping filename to (signature, length) tuple
    """
    return ctx.artifacts.get("sparkle_signatures", {})
