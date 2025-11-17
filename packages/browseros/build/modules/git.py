#!/usr/bin/env python3
"""
Git operations module for BlockBrowser build system
"""

import os
import sys
import subprocess
import shutil
import tarfile
import urllib.request
from pathlib import Path
from context import BuildContext
from utils import run_command, log_info, log_error, log_success, IS_WINDOWS, safe_rmtree


def setup_git(ctx: BuildContext) -> bool:
    """Setup git and checkout Chromium"""
    log_info(f"\n🔀 Setting up Chromium {ctx.chromium_version}...")

    os.chdir(ctx.chromium_src)

    # Fetch all tags and checkout
    log_info("📥 Fetching all tags from remote...")
    run_command(["git", "fetch", "--tags", "--force"])

    # Verify tag exists before checkout
    result = subprocess.run(
        ["git", "tag", "-l", ctx.chromium_version],
        text=True,
        capture_output=True,
        cwd=ctx.chromium_src,
    )
    if not result.stdout or ctx.chromium_version not in result.stdout:
        log_error(f"Tag {ctx.chromium_version} not found!")
        log_info("Available tags (last 10):")
        list_result = subprocess.run(
            ["git", "tag", "-l", "--sort=-version:refname"],
            text=True,
            capture_output=True,
            cwd=ctx.chromium_src,
        )
        if list_result.stdout:
            for tag in list_result.stdout.strip().split("\n")[:10]:
                log_info(f"  {tag}")
        raise ValueError(f"Git tag {ctx.chromium_version} not found")

    log_info(f"🔀 Checking out tag: {ctx.chromium_version}")
    run_command(["git", "checkout", f"tags/{ctx.chromium_version}"])

    # Sync dependencies
    log_info("📥 Syncing dependencies (this may take a while)...")
    # Windows gclient doesn't support --shallow flag
    if IS_WINDOWS:
        run_command(["gclient.bat", "sync", "-D", "--no-history", "--shallow"])
    else:
        run_command(["gclient", "sync", "-D", "--no-history", "--shallow"])

    log_success("Git setup complete")
    return True


def setup_sparkle(ctx: BuildContext) -> bool:
    """Download and setup Sparkle framework"""
    log_info("\n✨ Setting up Sparkle framework...")

    sparkle_dir = ctx.get_sparkle_dir()

    # Clean existing
    if sparkle_dir.exists():
        safe_rmtree(sparkle_dir)

    sparkle_dir.mkdir(parents=True)

    # Download Sparkle
    sparkle_url = ctx.get_sparkle_url()
    sparkle_archive = sparkle_dir / "sparkle.tar.xz"

    # Download using urllib (cross-platform)
    log_info(f"Downloading Sparkle from {sparkle_url}...")
    urllib.request.urlretrieve(sparkle_url, sparkle_archive)

    # Extract using tarfile module (cross-platform)
    log_info("Extracting Sparkle...")
    with tarfile.open(sparkle_archive, "r:xz") as tar:
        tar.extractall(sparkle_dir)

    # Clean up
    sparkle_archive.unlink()

    log_success("Sparkle setup complete")
    return True
