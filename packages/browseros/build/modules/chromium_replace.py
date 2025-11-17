#!/usr/bin/env python3
"""
Chromium file replacement module for BlockBrowser build system
"""

import sys
import shutil
from pathlib import Path
from context import BuildContext
from utils import log_info, log_success, log_error, log_warning


def replace_chromium_files(ctx: BuildContext, replacements=None) -> bool:
    """Replace files in chromium source with custom files from chromium_files directory"""
    log_info("\n🔄 Replacing chromium files...")
    log_info(f"  Build type: {ctx.build_type}")

    # Source directory containing replacement files
    replacement_dir = ctx.get_chromium_replace_files_dir()

    if not replacement_dir.exists():
        log_info(f"⚠️  No chromium_files directory found at: {replacement_dir}")
        return True

    replaced_count = 0
    skipped_count = 0

    # Find all files recursively in the replacement directory
    for src_file in replacement_dir.rglob("*"):
        if src_file.is_file():
            # Skip build-type specific files that don't match current build type
            if src_file.suffix in [".debug", ".release"]:
                # Check if this file matches the current build type
                if (ctx.build_type == "debug" and src_file.suffix != ".debug") or (
                    ctx.build_type == "release" and src_file.suffix != ".release"
                ):
                    skipped_count += 1
                    continue

                # For matching build type files, determine the actual destination
                # Remove the .debug/.release suffix for the destination path
                relative_path = src_file.relative_to(replacement_dir)
                # Convert path to string, remove suffix, then back to Path
                dest_relative = Path(str(relative_path).rsplit(".", 1)[0])
            else:
                # Regular file without build type suffix
                relative_path = src_file.relative_to(replacement_dir)
                dest_relative = relative_path

                # Check if build-type specific version exists
                debug_variant = src_file.with_suffix(src_file.suffix + ".debug")
                release_variant = src_file.with_suffix(src_file.suffix + ".release")

                # If a build-type specific variant exists for current build type, skip the generic file
                if (ctx.build_type == "debug" and debug_variant.exists()) or (
                    ctx.build_type == "release" and release_variant.exists()
                ):
                    log_info(
                        f"    ⏭️  Skipping {relative_path} (using {ctx.build_type} variant instead)"
                    )
                    skipped_count += 1
                    continue

            # Destination path in actual chromium source
            dst_file = ctx.chromium_src / dest_relative

            # Check if destination exists
            if not dst_file.exists():
                log_error(
                    f"    Destination file not found in chromium_src: {dest_relative}"
                )
                raise FileNotFoundError(
                    f"Destination file not found in chromium_src: {dest_relative}"
                )

            try:
                # Replace the file
                shutil.copy2(src_file, dst_file)
                log_info(f"    ✓ Replaced: {relative_path} → {dest_relative}")
                replaced_count += 1

            except Exception as e:
                log_error(f"    Error replacing file {relative_path}: {e}")
                raise

    log_success(
        f"Replaced {replaced_count} files (skipped {skipped_count} non-matching files)"
    )
    return True


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

    # Create destination path
    from context import BuildContext

    ctx = BuildContext(root_dir=root_dir)
    replacement_dir = ctx.get_chromium_replace_files_dir()
    dest_file = replacement_dir / relative_path

    log_info(f"📂 Adding file to replacements:")
    log_info(f"  Source: {file_path}")
    log_info(f"  Destination: {dest_file}")

    try:
        # Create parent directories if needed
        dest_file.parent.mkdir(parents=True, exist_ok=True)

        # Copy the file
        shutil.copy2(file_path, dest_file)

        log_success(f"✓ File added to chromium_files replacements: {relative_path}")
        log_info(
            f"  This file will be replaced during builds with --chromium-replace flag"
        )
        return True
    except Exception as e:
        log_error(f"Failed to add file: {e}")
        return False
