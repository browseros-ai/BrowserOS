#!/usr/bin/env python3
"""Chromium file replacement module for BrowserOS build system"""

import shutil
from pathlib import Path
from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.utils import log_info, log_success, log_error


@step("chromium_replace", phase="prep")
class ChromiumReplaceModule(Step):
    produces = []
    requires = []
    description = "Replace Chromium source files with custom versions"

    def validate(self, ctx: Context) -> None:
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

    def execute(self, ctx: Context) -> None:
        log_info("\n🔄 Replacing chromium files...")
        if not replace_chromium_files_impl(ctx):
            raise RuntimeError("Failed to replace chromium files")


def replace_chromium_files_impl(ctx: Context, replacements=None) -> bool:
    """Replace files in chromium source with custom files from chromium_files directory"""
    log_info("\n🔄 Replacing chromium files...")
    log_info(f"  Build type: {ctx.build_type}")
    log_info(f"  Product: {ctx.product.id}")

    replaced_count = 0
    skipped_count = 0
    found_overlay = False

    for replacement_dir in ctx.get_chromium_replace_roots():
        if not replacement_dir.exists():
            continue
        found_overlay = True
        log_info(f"  Overlay: {replacement_dir.relative_to(ctx.root_dir)}")
        replaced, skipped = _replace_from_root(ctx, replacement_dir)
        replaced_count += replaced
        skipped_count += skipped

    if not found_overlay:
        log_info(
            f"⚠️  No chromium_files overlays found under: {ctx.get_chromium_replace_files_dir()}"
        )
        return True

    log_success(
        f"Replaced {replaced_count} files (skipped {skipped_count} non-matching files)"
    )
    return True


def _replace_from_root(ctx: Context, replacement_dir: Path) -> tuple[int, int]:
    """Apply one Chromium overlay root to the source tree."""
    replaced_count = 0
    skipped_count = 0

    for src_file in sorted(replacement_dir.rglob("*")):
        if src_file.is_file():
            if src_file.suffix in [".debug", ".release"]:
                if (ctx.build_type == "debug" and src_file.suffix != ".debug") or (
                    ctx.build_type == "release" and src_file.suffix != ".release"
                ):
                    skipped_count += 1
                    continue

                relative_path = src_file.relative_to(replacement_dir)
                dest_relative = Path(str(relative_path).rsplit(".", 1)[0])
            else:
                relative_path = src_file.relative_to(replacement_dir)
                dest_relative = relative_path

                debug_variant = src_file.with_suffix(src_file.suffix + ".debug")
                release_variant = src_file.with_suffix(src_file.suffix + ".release")

                if (ctx.build_type == "debug" and debug_variant.exists()) or (
                    ctx.build_type == "release" and release_variant.exists()
                ):
                    log_info(
                        f"    ⏭️  Skipping {relative_path} (using {ctx.build_type} variant instead)"
                    )
                    skipped_count += 1
                    continue

            dst_file = ctx.chromium_src / dest_relative

            if not dst_file.exists():
                log_error(
                    f"    Destination file not found in chromium_src: {dest_relative}"
                )
                raise FileNotFoundError(
                    f"Destination file not found in chromium_src: {dest_relative}"
                )

            try:
                shutil.copy2(src_file, dst_file)
                log_info(f"    ✓ Replaced: {relative_path} → {dest_relative}")
                replaced_count += 1

            except Exception as e:
                log_error(f"    Error replacing file {relative_path}: {e}")
                raise

    return replaced_count, skipped_count


def add_file_to_replacements(
    file_path: Path, chromium_src: Path, root_dir: Path
) -> bool:
    """Add a file from chromium source to the replacement directory"""
    # Validate the file is within chromium_src
    try:
        relative_path = file_path.relative_to(chromium_src)
    except ValueError:
        log_error(
            f"File {file_path} is not within chromium source directory {chromium_src}"
        )
        return False

    ctx = Context(root_dir=root_dir, chromium_src=chromium_src)
    replacement_dir = ctx.get_chromium_replace_roots()[1]
    dest_file = replacement_dir / relative_path

    log_info("📂 Adding file to replacements:")
    log_info(f"  Source: {file_path}")
    log_info(f"  Destination: {dest_file}")

    try:
        # Create parent directories if needed
        dest_file.parent.mkdir(parents=True, exist_ok=True)

        # Copy the file
        shutil.copy2(file_path, dest_file)

        log_success(f"✓ File added to chromium_files replacements: {relative_path}")
        log_info(
            "  This file will be replaced during builds with --chromium-replace flag"
        )
        return True
    except Exception as e:
        log_error(f"Failed to add file: {e}")
        return False
