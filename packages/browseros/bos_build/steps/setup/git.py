#!/usr/bin/env python3
"""Git operations module for BrowserOS build system"""

import shutil
import tarfile
import urllib.request
import zipfile
from pathlib import Path

from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.utils import (
    log_info,
    log_success,
    IS_WINDOWS,
    safe_rmtree,
)

from ..source.provision import prepare

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
        # Build owns the checkout lock through packaging. This adapter must not
        # reacquire it or add destructive intent when --no-clean was selected.
        prepare(
            ctx.chromium_src,
            ctx.chromium_version,
            strategy="full",
            branch=BROWSEROS_BRANCH,
        )


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
