#!/usr/bin/env python3
"""Git operations module for BrowserOS build system"""

import re
import subprocess
import tarfile
import urllib.request
from typing import List

from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import (
    run_command,
    log_info,
    log_warning,
    log_error,
    log_success,
    IS_LINUX,
    IS_WINDOWS,
    safe_rmtree,
)


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

        # On Linux, depot_tools fetches per-arch sysroots automatically when
        # `.gclient` declares `target_cpus`. Ensure both x64 and arm64 are
        # listed before sync so cross-compilation just works on x64 hosts.
        if IS_LINUX():
            self._ensure_gclient_target_cpus(ctx, ["x64", "arm64"])

        log_info("📥 Syncing dependencies (this may take a while)...")
        if IS_WINDOWS():
            run_command(["gclient.bat", "sync", "-D", "--no-history", "--shallow"], cwd=ctx.chromium_src)
        else:
            run_command(["gclient", "sync", "-D", "--no-history", "--shallow"], cwd=ctx.chromium_src)

        log_success("Git setup complete")

    def _ensure_gclient_target_cpus(self, ctx: Context, required: List[str]) -> None:
        """Idempotently add `target_cpus` to .gclient so depot_tools fetches
        the matching Linux sysroots for cross-compilation.

        depot_tools convention: .gclient lives one directory above
        chromium_src (i.e. ../.gclient). It is a Python file with a list
        of solution dicts followed by optional top-level assignments.
        We append a `target_cpus = [...]` line if missing or merge in any
        archs that aren't already present.
        """
        gclient_path = ctx.chromium_src.parent / ".gclient"
        if not gclient_path.exists():
            log_warning(
                f"⚠️  .gclient not found at {gclient_path}; "
                f"skipping target_cpus bootstrap. "
                f"Cross-arch builds may fail until you run `fetch chromium`."
            )
            return

        content = gclient_path.read_text()
        match = re.search(r"^\s*target_cpus\s*=\s*\[([^\]]*)\]", content, re.MULTILINE)

        if match:
            existing = re.findall(r"['\"]([^'\"]+)['\"]", match.group(1))
            missing = [arch for arch in required if arch not in existing]
            if not missing:
                log_info(f"✓ .gclient target_cpus already includes {required}")
                return
            merged = sorted(set(existing) | set(required))
            new_line = f"target_cpus = {merged!r}"
            content = (
                content[: match.start()] + new_line + content[match.end() :]
            )
            log_info(
                f"📝 Updating .gclient target_cpus: {existing} → {merged}"
            )
        else:
            new_line = f"\ntarget_cpus = {required!r}\n"
            content = content.rstrip() + "\n" + new_line
            log_info(f"📝 Adding target_cpus = {required} to .gclient")

        gclient_path.write_text(content)

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
