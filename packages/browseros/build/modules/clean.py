#!/usr/bin/env python3
"""
Clean module for BlockBrowser build system
"""

import os
import shutil
from pathlib import Path
from context import BuildContext
from utils import run_command, log_info, log_success, safe_rmtree


def clean(ctx: BuildContext) -> bool:
    """Clean build artifacts"""
    log_info("🧹 Cleaning build artifacts...")

    out_path = ctx.chromium_src / ctx.out_dir
    if out_path.exists():
        safe_rmtree(out_path)
        log_success("Cleaned build directory")

    log_info("\n🔀 Resetting git branch and removing all tracked files...")
    git_reset(ctx)

    log_info("\n🧹 Cleaning Sparkle build artifacts...")
    clean_sparkle(ctx)

    return True


def clean_sparkle(ctx: BuildContext) -> bool:
    """Clean Sparkle build artifacts"""
    log_info("\n🧹 Cleaning Sparkle build artifacts...")
    sparkle_dir = ctx.get_sparkle_dir()
    if sparkle_dir.exists():
        safe_rmtree(sparkle_dir)
    log_success("Cleaned Sparkle build directory")
    return True


def git_reset(ctx: BuildContext) -> bool:
    """Reset git branch and clean with exclusions"""
    os.chdir(ctx.chromium_src)
    run_command(["git", "reset", "--hard", "HEAD"])
    os.chdir(ctx.root_dir)

    log_info("\n🧹 Running git clean with exclusions for important directories...")
    os.chdir(ctx.chromium_src)
    run_command(
        [
            "git",
            "clean",
            "-fdx",
            "chrome/",
            "components/",
            "--exclude=third_party/",
            "--exclude=build_tools/",
            "--exclude=uc_staging/",
            "--exclude=buildtools/",
            "--exclude=tools/",
            "--exclude=build/",
        ]
    )
    os.chdir(ctx.root_dir)
    log_success("Git reset and clean complete")
    return True
