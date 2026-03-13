#!/usr/bin/env python3
"""Git operations module for BrowserOS build system"""

import subprocess
import tarfile
import urllib.request
import zipfile
from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import run_command, log_info, log_error, log_success, IS_WINDOWS, safe_rmtree


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
        from ...common.utils import IS_MACOS
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


class WinSparkleSetupModule(CommandModule):
    produces = []
    requires = []
    description = "Download and setup WinSparkle (Windows auto-update)"

    def validate(self, ctx: Context) -> None:
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

    def execute(self, ctx: Context) -> None:
        log_info("\n✨ Setting up WinSparkle...")

        winsparkle_dir = ctx.get_winsparkle_dir()

        if winsparkle_dir.exists():
            safe_rmtree(winsparkle_dir)

        winsparkle_dir.mkdir(parents=True)

        winsparkle_url = ctx.get_winsparkle_url()
        winsparkle_archive = winsparkle_dir / "winsparkle.zip"

        log_info(f"Downloading WinSparkle from {winsparkle_url}...")
        urllib.request.urlretrieve(winsparkle_url, winsparkle_archive)

        log_info("Extracting WinSparkle...")
        with zipfile.ZipFile(winsparkle_archive, "r") as zf:
            # Strip the top-level directory (e.g. WinSparkle-0.9.2/) from paths
            prefix = f"WinSparkle-{ctx.WINSPARKLE_VERSION}/"
            for member in zf.infolist():
                if not member.filename.startswith(prefix):
                    continue
                rel_path = member.filename[len(prefix):]
                if not rel_path:
                    continue
                target = winsparkle_dir / rel_path
                if member.is_dir():
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(zf.read(member.filename))

        winsparkle_archive.unlink()

        log_success("WinSparkle setup complete")
