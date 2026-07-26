#!/usr/bin/env python3
"""Series-based patch module for BrowserOS build system (GNU Quilt format)"""

import shutil
import subprocess
from pathlib import Path
from typing import Iterator

from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.utils import log_info, log_success, log_error, get_platform


ENCODING = "UTF-8"


@step("series_patches", phase="prep", optional=True)
class SeriesPatchesModule(Step):
    produces = []
    requires = []
    description = "Apply series-based patches (GNU Quilt format)"

    def validate(self, ctx: Context) -> None:
        if not shutil.which("git"):
            raise ValidationError("Git is not available in PATH")

        series_dir = ctx.get_series_patches_dir()
        if not series_dir.exists():
            raise ValidationError(f"Series patches directory not found: {series_dir}")

        series_file = series_dir / "series"
        if not series_file.exists():
            raise ValidationError(f"Series file not found: {series_file}")

    def execute(self, ctx: Context) -> None:
        log_info("\n🩹 Applying series patches...")
        applied, failed = apply_series_patches_impl(ctx)

        if failed:
            raise RuntimeError(f"Failed to apply {len(failed)} series patches")

        log_success(f"Applied {len(applied)} series patches")


def parse_series(series_path: Path) -> Iterator[str]:
    """
    Parse a GNU Quilt series file, yielding patch paths.

    Format:
    - One patch path per line (relative to series directory)
    - Lines starting with # are comments
    - Inline comments with ' #' are stripped
    - Blank lines are ignored
    """
    with series_path.open(encoding=ENCODING) as f:
        lines = f.read().splitlines()

    for line in lines:
        line = line.strip()
        if not line:
            continue
        if line.startswith("#"):
            continue
        # Strip inline comments
        if " #" in line:
            line = line.split(" #")[0].strip()
        if line:
            yield line


def get_series_files(series_dir: Path) -> list[Path]:
    """
    Get all applicable series files for the current platform.

    Returns series files in order:
    1. series (common, always applied)
    2. series.{platform} (platform-specific: windows, linux, macos)

    Only returns files that exist.
    """
    files = []

    # Common series file (always)
    common = series_dir / "series"
    if common.exists():
        files.append(common)

    # Platform-specific series file
    platform = get_platform()  # "windows", "linux", or "macos"
    platform_file = series_dir / f"series.{platform}"
    if platform_file.exists():
        files.append(platform_file)

    return files


def apply_single_patch(patch_path: Path, chromium_src: Path) -> tuple[bool, str]:
    """
    Apply a single patch using git apply.

    Returns:
        (success, error_message)
    """
    cmd = [
        "git", "apply",
        "--ignore-whitespace",
        "--whitespace=nowarn",
        "-p1",
        str(patch_path)
    ]

    result = subprocess.run(
        cmd,
        cwd=chromium_src,
        capture_output=True,
        text=True
    )

    if result.returncode == 0:
        return True, ""

    # Fallback to 3-way merge
    cmd_3way = [
        "git", "apply",
        "--3way",
        "--ignore-whitespace",
        "--whitespace=nowarn",
        "-p1",
        str(patch_path)
    ]

    result = subprocess.run(
        cmd_3way,
        cwd=chromium_src,
        capture_output=True,
        text=True
    )

    if result.returncode == 0:
        return True, ""

    return False, result.stderr or result.stdout


def apply_series_patches_impl(
    ctx: Context,
    dry_run: bool = False
) -> tuple[list[Path], list[Path]]:
    """
    Apply all patches listed in series files (common + platform-specific).

    Args:
        ctx: Build context
        dry_run: If True, only check if patches would apply

    Returns:
        (applied_patches, failed_patches)
    """
    series_dir = ctx.get_series_patches_dir()
    chromium_src = ctx.chromium_src

    series_files = get_series_files(series_dir)
    if not series_files:
        log_info("  No series files found")
        return [], []

    # Collect all patches from all series files
    all_patches: list[tuple[str, Path]] = []  # (relative_path, series_file)
    for series_file in series_files:
        for relative_path in parse_series(series_file):
            all_patches.append((relative_path, series_file))

    total = len(all_patches)
    if total == 0:
        log_info("  No patches listed in series files")
        return [], []

    platform = get_platform()
    log_info(f"  Found {total} patches for platform '{platform}' across {len(series_files)} series file(s)")

    applied = []
    failed = []

    for i, (relative_path, series_file) in enumerate(all_patches, 1):
        patch_path = series_dir / relative_path

        if not patch_path.exists():
            log_error(f"  [{i}/{total}] ✗ Patch file not found: {relative_path}")
            failed.append(patch_path)
            continue

        if dry_run:
            cmd = [
                "git", "apply",
                "--check",
                "--ignore-whitespace",
                "-p1",
                str(patch_path)
            ]
            result = subprocess.run(
                cmd,
                cwd=chromium_src,
                capture_output=True,
                text=True
            )
            if result.returncode == 0:
                log_info(f"  [{i}/{total}] ✓ Would apply: {relative_path}")
                applied.append(patch_path)
            else:
                log_error(f"  [{i}/{total}] ✗ Would fail: {relative_path}")
                failed.append(patch_path)
        else:
            success, error = apply_single_patch(patch_path, chromium_src)
            if success:
                log_info(f"  [{i}/{total}] ✓ Applied: {relative_path}")
                applied.append(patch_path)
            else:
                log_error(f"  [{i}/{total}] ✗ Failed: {relative_path}")
                if error:
                    log_error(f"      {error.strip()}")
                failed.append(patch_path)

    return applied, failed
