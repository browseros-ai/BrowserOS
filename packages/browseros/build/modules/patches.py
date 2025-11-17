#!/usr/bin/env python3
"""
Patch management module for BlockBrowser build system
"""

import sys
import shutil
import subprocess
from pathlib import Path
from typing import Iterator, List, Tuple, Optional
from context import BuildContext
from utils import (
    log_info,
    log_error,
    log_success,
    log_warning,
    IS_WINDOWS,
    IS_LINUX,
    IS_MACOS,
)

# Switch to new patching system using dev CLI
NEW_PATCHING = True


def apply_patches_with_dev_cli(
    ctx: BuildContext, interactive: bool = False, commit_each: bool = False
) -> bool:
    """Apply patches using the new dev CLI system"""
    if not ctx.apply_patches:
        log_info("\n⏭️  Skipping patches")
        return True

    log_info("\n🩹 Applying patches using new dev CLI system...")

    # Check if git is available
    if not shutil.which("git"):
        log_error("Git is not available in PATH")
        log_error("Please install Git to apply patches")
        raise RuntimeError("Git not found in PATH")

    # Import dev CLI module
    sys.path.insert(0, str(Path(__file__).parent.parent))
    from modules.dev_cli.apply import apply_all_patches

    # Call the dev CLI function directly
    applied, failed = apply_all_patches(
        build_ctx=ctx,
        commit_each=commit_each,
        dry_run=False,
        interactive=interactive,
    )

    # Handle results
    if failed and not interactive:
        # In non-interactive mode, fail if any patches failed
        raise RuntimeError(f"Failed to apply {len(failed)} patches")

    return True


def apply_patches(
    ctx: BuildContext, interactive: bool = False, commit_each: bool = False
) -> bool:
    """Apply BlockBrowser patches"""
    # Use new patching system if enabled
    if NEW_PATCHING:
        return apply_patches_with_dev_cli(ctx, interactive, commit_each)

    # Otherwise, use the legacy patching system
    if not ctx.apply_patches:
        log_info("\n⏭️  Skipping patches")
        return True

    log_info("\n🩹 Applying patches...")

    # Check if git is available
    if not shutil.which("git"):
        log_error("Git is not available in PATH")
        log_error("Please install Git to apply patches")
        raise RuntimeError("Git not found in PATH")

    # Get list of patches
    root_patches_dir = ctx.get_patches_dir()
    blockbrowser_patches_dir = ctx.get_blockbrowser_patches_dir()

    if not blockbrowser_patches_dir.exists():
        log_error(f"Patches directory not found: {blockbrowser_patches_dir}")
        raise FileNotFoundError(f"Patches directory not found: {blockbrowser_patches_dir}")

    # get all patches in blockbrowser_patches_dir
    all_patches = list(parse_series_file(root_patches_dir))

    # Filter out patches that should be skipped on this platform
    patches = []
    skipped_count = 0
    for patch_path, skip_platforms in all_patches:
        if should_skip_patch(skip_platforms):
            log_info(
                f"⏭️  Skipping {patch_path.name} (not for {get_current_platform()})"
            )
            skipped_count += 1
        else:
            patches.append((patch_path, skip_platforms))

    if not patches:
        if skipped_count > 0:
            log_info(
                f"⚠️  All {skipped_count} patches were skipped for {get_current_platform()}"
            )
        else:
            log_info("⚠️  No patches found to apply")
        return True

    log_info(
        f"Found {len(patches)} patches to apply ({skipped_count} skipped for {get_current_platform()})"
    )

    if interactive:
        log_info(
            "🔍 Interactive mode enabled - will ask for confirmation before each patch"
        )

    if commit_each:
        log_info("📝 Git commit mode enabled - will create a commit after each patch")

    # Apply each patch
    for i, (patch_path, _) in enumerate(patches, 1):
        if not patch_path.exists():
            log_info(f"⚠️  Patch file not found: {patch_path}")
            continue

        if interactive:
            # Show patch info and ask for confirmation
            log_info(f"\n{'='*60}")
            log_info(f"Patch {i}/{len(patches)}: {patch_path.name}")
            log_info(f"{'='*60}")

            while True:
                choice = input(
                    "\nOptions:\n  1) Apply this patch\n  2) Skip this patch\n  3) Stop patching here\nEnter your choice (1-3): "
                ).strip()

                if choice == "1":
                    apply_single_patch(
                        patch_path, ctx.chromium_src, i, len(patches), commit_each
                    )
                    break
                elif choice == "2":
                    log_warning(f"⏭️  Skipping patch {patch_path.name}")
                    break
                elif choice == "3":
                    log_info("Stopping patch process as requested")
                    return True
                else:
                    log_error("Invalid choice. Please enter 1, 2, or 3.")
        else:
            apply_single_patch(
                patch_path, ctx.chromium_src, i, len(patches), commit_each
            )

    log_success("Patches applied")
    return True


def get_current_platform() -> str:
    """Get the current platform name for skip checking"""
    if IS_WINDOWS:
        return "windows"
    elif IS_LINUX:
        return "linux"
    elif IS_MACOS:
        return "darwin"
    else:
        return "unknown"


def should_skip_patch(skip_platforms: Optional[List[str]]) -> bool:
    """Check if a patch should be skipped on the current platform"""
    if skip_platforms is None:
        return False

    current_platform = get_current_platform()

    # Also check for common aliases
    platform_aliases = {
        "darwin": ["darwin", "macos", "mac", "osx"],
        "linux": ["linux"],
        "windows": ["windows", "win32", "win"],
    }

    current_aliases = platform_aliases.get(current_platform, [current_platform])

    # Check if any skip platform matches our current platform or its aliases
    for skip_platform in skip_platforms:
        if skip_platform in current_aliases:
            return True

    return False


def parse_series_file(patches_dir: Path) -> Iterator[Tuple[Path, Optional[List[str]]]]:
    """Parse the series file to get list of patches with skip directives

    Returns tuples of (patch_path, skip_platforms) where skip_platforms
    is None if no platforms should be skipped, or a list of platform names
    """
    series_file = patches_dir / "series"

    # Read series file
    with series_file.open("r") as f:
        lines = f.read().splitlines()

    patches = []
    for line in lines:
        # Skip empty lines and comments
        line = line.strip()
        if not line or line.startswith("#"):
            continue

        skip_platforms = None

        # Check for #skip directive
        if " #skip:" in line:
            parts = line.split(" #skip:")
            line = parts[0].strip()
            # Parse platforms to skip
            skip_platforms = [p.strip().lower() for p in parts[1].split(",")]
        elif " #" in line:
            # Remove other inline comments
            line = line.split(" #")[0].strip()

        patches.append((patches_dir / line, skip_platforms))

    return patches


def apply_single_patch(
    patch_path: Path,
    tree_path: Path,
    current_num: int,
    total: int,
    commit_each: bool = False,
) -> bool:
    """Apply a single patch using git apply"""
    # Use git apply which is cross-platform and handles patch format better
    cmd = [
        "git",
        "apply",
        "--ignore-whitespace",
        "--whitespace=nowarn",
        "-p1",
        str(patch_path),
    ]

    log_info(f"  * Applying {patch_path.name} ({current_num}/{total})")

    # Run from the tree_path directory
    result = subprocess.run(cmd, text=True, capture_output=True, cwd=tree_path)

    if result.returncode == 0:
        if commit_each:
            commit_patch(patch_path, tree_path)
        return True

    # Patch failed - try with --3way for better conflict resolution
    log_warning(f"Standard apply failed, trying 3-way merge for {patch_path.name}")
    cmd.append("--3way")
    result = subprocess.run(
        cmd[:-1] + ["--3way", str(patch_path)],
        text=True,
        capture_output=True,
        cwd=tree_path,
    )

    if result.returncode == 0:
        log_info(f"✓ Applied {patch_path.name} with 3-way merge")
        if commit_each:
            commit_patch(patch_path, tree_path)
        return True

    # Patch still failed
    log_error(f"Failed to apply patch: {patch_path.name}")
    if result.stderr:
        log_error(f"Error: {result.stderr}")

    # Interactive prompt for handling failure
    log_error("\n============================================")
    log_error(f"Patch {patch_path.name} failed to apply.")
    log_info("Options:")
    log_info("  1) Skip this patch and continue")
    log_info("  2) Retry this patch")
    log_info("  3) Abort patching")
    log_info("  4) Interactive mode - Fix manually and continue")

    while True:
        choice = input("Enter your choice (1-4): ").strip()

        if choice == "1":
            log_warning(f"⏭️  Skipping patch {patch_path.name}")
            return True  # Continue with next patch
        elif choice == "2":
            return apply_single_patch(
                patch_path, tree_path, current_num, total, commit_each
            )
        elif choice == "3":
            log_error("Aborting patch process")
            raise RuntimeError("Patch process aborted by user")
        elif choice == "4":
            log_info("\nPlease fix the issue manually, then press Enter to continue...")
            input("Press Enter when ready: ")
            # Retry after manual fix
            return apply_single_patch(
                patch_path, tree_path, current_num, total, commit_each
            )


def commit_patch(patch_path: Path, tree_path: Path) -> bool:
    """Create a git commit for the applied patch"""
    try:
        # Stage all changes
        cmd_add = ["git", "add", "-A"]
        result = subprocess.run(cmd_add, capture_output=True, text=True, cwd=tree_path)
        if result.returncode != 0:
            log_warning(f"Failed to stage changes for patch {patch_path.name}")
            if result.stderr:
                log_warning(f"Error: {result.stderr}")
            return False

        # Create commit message
        patch_name = patch_path.stem  # Remove .patch extension
        commit_message = f"patch: {patch_name}"

        # Create the commit
        cmd_commit = ["git", "commit", "-m", commit_message]
        result = subprocess.run(
            cmd_commit, capture_output=True, text=True, cwd=tree_path
        )

        if result.returncode == 0:
            log_success(f"📝 Created commit for patch: {patch_name}")
            return True
        else:
            log_warning(f"Failed to commit patch {patch_path.name}")
            if result.stderr:
                log_warning(f"Error: {result.stderr}")
            return False

    except Exception as e:
        log_warning(f"Error creating commit for patch {patch_path.name}: {e}")
        return False
