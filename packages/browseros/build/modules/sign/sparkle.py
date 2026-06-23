#!/usr/bin/env python3
"""Sparkle/WinSparkle Ed25519 signing module for auto-update"""

from pathlib import Path
from typing import Dict, List, Tuple

from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.sparkle import sparkle_sign_file
from ...common.utils import (
    log_info,
    log_success,
    log_warning,
)


def find_signable_artifacts(dist_dir: Path) -> List[Path]:
    """Update artifacts the appcast points at: DMGs on macOS, the installer
    EXE on Windows (WinSparkle downloads and runs the installer directly;
    the portable ZIP is not an update enclosure, so it is not signed).
    """
    return sorted(dist_dir.glob("*.dmg")) + sorted(dist_dir.glob("*.exe"))


class SparkleSignModule(CommandModule):
    """Sign update artifacts with the Sparkle Ed25519 key"""

    produces = ["sparkle_signatures"]
    requires = []
    description = "Sign update artifacts with Sparkle Ed25519 key for auto-update"

    def validate(self, ctx: Context) -> None:
        if not ctx.env.has_sparkle_key():
            raise ValidationError(
                "SPARKLE_PRIVATE_KEY environment variable not set"
            )

    def execute(self, ctx: Context) -> None:
        log_info("\n🔐 Signing update artifacts with Sparkle...")

        dist_dir = ctx.get_dist_dir()
        if not dist_dir.exists():
            log_warning(f"Dist directory not found: {dist_dir}")
            return

        artifact_files = find_signable_artifacts(dist_dir)
        if not artifact_files:
            log_warning("No signable artifacts (*.dmg, *.exe) found to sign")
            return

        # Sign each artifact and collect signatures
        signatures = sign_files_with_sparkle(ctx, artifact_files)

        # Store signatures in artifact registry for upload module
        for filename, (sig, length) in signatures.items():
            ctx.artifact_registry.add(f"sparkle_sig_{filename}", Path(filename))
            log_info(f"  {filename}: sig={sig[:20]}... length={length}")

        # Store signatures for upload module to access via ctx.artifacts
        ctx.artifacts["sparkle_signatures"] = signatures

        log_success(f"✅ Signed {len(signatures)} artifact(s) with Sparkle")


def sign_files_with_sparkle(
    ctx: Context,
    files: list,
) -> Dict[str, Tuple[str, int]]:
    """Sign files with Sparkle and return signatures

    Args:
        ctx: Build context
        files: List of file paths to sign

    Returns:
        Dict mapping filename to (signature, length) tuple
    """
    signatures = {}

    for file_path in files:
        log_info(f"🔐 Signing {file_path.name}...")
        sig, length = sparkle_sign_file(file_path, ctx.env)
        if sig:
            signatures[file_path.name] = (sig, length)
            log_success(f"✓ Signed {file_path.name}")

    return signatures


def get_sparkle_signatures(ctx: Context) -> Dict[str, Tuple[str, int]]:
    """Get stored Sparkle signatures from context

    Args:
        ctx: Build context

    Returns:
        Dict mapping filename to (signature, length) tuple
    """
    return ctx.artifacts.get("sparkle_signatures", {})
