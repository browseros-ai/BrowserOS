#!/usr/bin/env python3
"""Git operations module for BrowserOS build system"""

import subprocess
import tarfile
import urllib.request
from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import run_command, log_info, log_error, log_success, IS_MACOS, IS_WINDOWS, safe_rmtree


class GitSetupModule(CommandModule):
    produces = []
    requires = []
    description = "Checkout Chromium version and sync dependencies"

    def validate(self, ctx: Context) -> None:
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

        if not ctx.chromium_version:
            raise ValidationError("Chromium version not set")

    def execute(self, ctx: Context) -> None:
        log_info(f"\n🔀 Setting up Chromium {ctx.chromium_version}...")

        log_info("📥 Fetching all tags from remote...")
        run_command(["git", "fetch", "--tags", "--force"], cwd=ctx.chromium_src)

        self._verify_tag_exists(ctx)

        log_info(f"🔀 Checking out tag: {ctx.chromium_version}")
        run_command(["git", "checkout", f"tags/{ctx.chromium_version}"], cwd=ctx.chromium_src)

        log_info("📥 Syncing dependencies (this may take a while)...")
        if IS_WINDOWS():
            run_command(["gclient.bat", "sync", "-D", "--no-history", "--shallow"], cwd=ctx.chromium_src)
        else:
            run_command(["gclient", "sync", "-D", "--no-history", "--shallow"], cwd=ctx.chromium_src)

        log_success("Git setup complete")

    def _verify_tag_exists(self, ctx: Context) -> None:
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
            raise ValidationError(f"Git tag {ctx.chromium_version} not found")


class SparkleSetupModule(CommandModule):
    produces = []
    requires = []
    description = "Download and setup Sparkle framework (macOS only)"

    def validate(self, ctx: Context) -> None:
        if not IS_MACOS():
            raise ValidationError("Sparkle setup requires macOS")

    def execute(self, ctx: Context) -> None:
        log_info("\n✨ Setting up Sparkle framework...")

        sparkle_dir = ctx.get_sparkle_dir()

        if sparkle_dir.exists():
            safe_rmtree(sparkle_dir)

        sparkle_dir.mkdir(parents=True)

        sparkle_url = ctx.get_sparkle_url()
        sparkle_archive = sparkle_dir / "sparkle.tar.xz"

        log_info(f"Downloading Sparkle from {sparkle_url}...")
        urllib.request.urlretrieve(sparkle_url, sparkle_archive)

        log_info("Extracting Sparkle...")
        with tarfile.open(sparkle_archive, "r:xz") as tar:
            tar.extractall(sparkle_dir)

        sparkle_archive.unlink()

        log_success("Sparkle setup complete")
