#!/usr/bin/env python3
"""Git operations module for BrowserOS build system"""

import re
import shutil
import subprocess
import tarfile
import urllib.request
import zipfile
from pathlib import Path
from typing import List

from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.utils import (
    run_command,
    log_info,
    log_warning,
    log_error,
    log_success,
    IS_LINUX,
    IS_WINDOWS,
    safe_rmtree,
)

BROWSEROS_BRANCH = "browseros"


@step("git_setup", phase="setup")
class GitSetupModule(Step):
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

        self._checkout_browseros_branch(ctx)

        # On Linux, depot_tools fetches per-arch sysroots automatically when
        # `.gclient` declares `target_cpus`. Ensure both x64 and arm64 are
        # listed before sync so cross-compilation just works on x64 hosts.
        if IS_LINUX():
            self._ensure_gclient_target_cpus(ctx, ["x64", "arm64"])

        log_info("📥 Syncing dependencies (this may take a while)...")
        if IS_WINDOWS():
            run_command(
                ["gclient.bat", "sync", "-D", "--no-history", "--shallow"],
                cwd=ctx.chromium_src,
            )
        else:
            run_command(
                ["gclient", "sync", "-D", "--no-history", "--shallow"],
                cwd=ctx.chromium_src,
            )

        log_success("Git setup complete")

    def _checkout_browseros_branch(self, ctx: Context) -> None:
        """Create/reset local `browseros` branch at the pinned tag, not detached HEAD."""
        # `-B` creates or force-resets in one step from an explicit start-point,
        # so no redundant detached-HEAD checkout is needed first. gclient's
        # solution is `managed: False`, so the later sync ignores this branch.
        log_info(
            f"🔀 Checking out tag {ctx.chromium_version} as branch: {BROWSEROS_BRANCH}"
        )
        run_command(
            ["git", "checkout", "-B", BROWSEROS_BRANCH, f"tags/{ctx.chromium_version}"],
            cwd=ctx.chromium_src,
        )

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
            content = content[: match.start()] + new_line + content[match.end() :]
            log_info(f"📝 Updating .gclient target_cpus: {existing} → {merged}")
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


@step("sparkle_setup", phase="setup", platforms=("macos",))
class SparkleSetupModule(Step):
    produces = []
    requires = []
    description = "Download and setup Sparkle framework (macOS only)"

    def validate(self, ctx: Context) -> None:
        from ...lib.utils import IS_MACOS

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


@step("winsparkle_setup", phase="setup", platforms=("windows",))
class WinSparkleSetupModule(Step):
    produces = []
    requires = []
    description = "Download and setup WinSparkle library (Windows only)"

    def validate(self, ctx: Context) -> None:
        if not IS_WINDOWS():
            raise ValidationError("WinSparkle setup requires Windows")

    def execute(self, ctx: Context) -> None:
        log_info("\n✨ Setting up WinSparkle library...")

        winsparkle_dir = ctx.get_winsparkle_dir()

        if winsparkle_dir.exists():
            safe_rmtree(winsparkle_dir)

        winsparkle_dir.mkdir(parents=True)

        winsparkle_url = ctx.get_winsparkle_url()
        winsparkle_archive = winsparkle_dir / "winsparkle.zip"

        log_info(f"Downloading WinSparkle from {winsparkle_url}...")
        urllib.request.urlretrieve(winsparkle_url, winsparkle_archive)

        log_info("Extracting WinSparkle...")
        extract_winsparkle_zip(winsparkle_archive, winsparkle_dir)

        winsparkle_archive.unlink()

        log_success("WinSparkle setup complete")


def extract_winsparkle_zip(archive: Path, dest: Path) -> None:
    """Extract the release zip stripping its top-level WinSparkle-<version>/
    directory, so //third_party/winsparkle paths stay version-independent
    (include/, x64/Release/, ...) and match the vendored BUILD.gn.
    """
    with zipfile.ZipFile(archive) as zf:
        infos = zf.infolist()

        # The official archive wraps everything in a single version dir; a
        # different layout would silently produce a broken tree, so fail fast.
        top_levels = {
            Path(info.filename).parts[0] for info in infos if info.filename.strip("/")
        }
        if len(top_levels) != 1:
            raise RuntimeError(
                f"Expected a single top-level directory in {archive.name}, "
                f"got: {sorted(top_levels)}"
            )

        resolved_dest = dest.resolve()
        for info in infos:
            parts = Path(info.filename).parts
            if len(parts) <= 1:
                continue
            target = dest.joinpath(*parts[1:])
            # Guards against zip-slip (.., absolute or drive-relative paths).
            if not target.resolve().is_relative_to(resolved_dest):
                raise RuntimeError(f"Unsafe path in archive: {info.filename}")
            if info.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with zf.open(info) as src, open(target, "wb") as out:
                shutil.copyfileobj(src, out)
