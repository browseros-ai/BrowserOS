"""
Common functions shared across apply module commands.

Contains core patch application logic used by apply_all, apply_feature, and apply_patch.
"""

from pathlib import Path
from typing import List, Tuple, Optional

from .utils import run_git_command, file_exists_in_commit, reset_file_to_commit
from ...common.utils import log_info, log_error, log_success, log_warning


def find_patch_files(patches_dir: Path) -> List[Path]:
    """Find all valid patch files in a directory.

    Args:
        patches_dir: Directory to search for patches

    Returns:
        List of patch file paths, sorted
    """
    if not patches_dir.exists():
        return []

    return sorted(
        [
            p
            for p in patches_dir.rglob("*")
            if p.is_file()
            and not p.name.endswith(".deleted")
            and not p.name.endswith(".binary")
            and not p.name.endswith(".rename")
            and not p.name.startswith(".")
        ]
    )


def apply_single_patch(
    patch_path: Path,
    chromium_src: Path,
    dry_run: bool = False,
    relative_to: Optional[Path] = None,
    reset_to: Optional[str] = None,
) -> Tuple[bool, Optional[str]]:
    """Apply a single patch file.

    Args:
        patch_path: Path to the patch file
        chromium_src: Chromium source directory
        dry_run: If True, only check if patch would apply
        relative_to: Base path for displaying relative paths (optional)
        reset_to: Commit to reset file to before applying (optional)

    Returns:
        Tuple of (success: bool, error_message: Optional[str])
    """
    display_path = patch_path.relative_to(relative_to) if relative_to else patch_path

    # Reset file to base commit if requested
    if reset_to and not dry_run:
        file_path = str(display_path)
        if file_exists_in_commit(file_path, reset_to, chromium_src):
            log_info(f"  Resetting to {reset_to[:8]}: {file_path}")
            reset_file_to_commit(file_path, reset_to, chromium_src)

    if dry_run:
        # Just check if patch would apply
        result = run_git_command(
            ["git", "apply", "--check", "-p1", str(patch_path)], cwd=chromium_src
        )
        if result.returncode == 0:
            log_success(f"  ✓ Would apply: {display_path}")
            return True, None
        else:
            log_error(f"  ✗ Would fail: {display_path}")
            return False, result.stderr
    else:
        # Try standard apply first
        result = run_git_command(
            [
                "git",
                "apply",
                "--ignore-whitespace",
                "--whitespace=nowarn",
                "-p1",
                str(patch_path),
            ],
            cwd=chromium_src,
        )

        if result.returncode != 0:
            # Try with 3-way merge
            result = run_git_command(
                [
                    "git",
                    "apply",
                    "--ignore-whitespace",
                    "--whitespace=nowarn",
                    "-p1",
                    "--3way",
                    str(patch_path),
                ],
                cwd=chromium_src,
            )

        if result.returncode == 0:
            log_success(f"  ✓ Applied: {display_path}")
            return True, None
        else:
            log_error(f"  ✗ Failed: {display_path}")
            if result.stderr:
                log_error(f"    {result.stderr}")
            return False, result.stderr


def create_patch_commit(
    patch_identifier: str, chromium_src: Path, feature_name: Optional[str] = None
) -> bool:
    """Create a git commit after applying a patch.

    Args:
        patch_identifier: Patch name or path for commit message
        chromium_src: Chromium source directory
        feature_name: Optional feature name for commit message

    Returns:
        True if commit was created successfully
    """
    # Stage all changes
    result = run_git_command(["git", "add", "-A"], cwd=chromium_src)
    if result.returncode != 0:
        log_warning("Failed to stage changes for commit")
        return False

    # Create commit message
    if feature_name:
        commit_msg = f"Apply {feature_name}: {Path(patch_identifier).name}"
    else:
        commit_msg = f"Apply patch: {patch_identifier}"

    result = run_git_command(["git", "commit", "-m", commit_msg], cwd=chromium_src)

    if result.returncode == 0:
        log_success(f"📝 Created commit: {commit_msg}")
        return True
    else:
        log_warning("Failed to create commit")
        return False


def process_patch_list(
    patch_list: List[Tuple[Path, str]],
    chromium_src: Path,
    patches_dir: Path,
    dry_run: bool = False,
    interactive: bool = False,
    reset_to: Optional[str] = None,
) -> Tuple[int, List[str]]:
    """Process a list of patches.

    Args:
        patch_list: List of (patch_path, display_name) tuples
        chromium_src: Chromium source directory
        patches_dir: Base directory for relative path display
        dry_run: Only check if patches would apply
        interactive: Ask for confirmation before each patch
        reset_to: Commit to reset files to before applying (optional)

    Returns:
        Tuple of (applied_count, failed_list)
    """
    applied = 0
    failed = []
    skipped = 0

    total = len(patch_list)

    for i, (patch_path, display_name) in enumerate(patch_list, 1):
        if interactive and not dry_run:
            # Show patch info and ask for confirmation
            log_info(f"\n{'='*60}")
            log_info(f"Patch {i}/{total}: {display_name}")
            log_info(f"{'='*60}")

            while True:
                choice = input(
                    "\nOptions:\n  1) Apply this patch\n  2) Skip this patch\n  3) Stop patching\nChoice (1-3): "
                ).strip()

                if choice == "1":
                    break  # Apply the patch
                elif choice == "2":
                    log_warning(f"⏭️  Skipping patch: {display_name}")
                    skipped += 1
                    continue  # Skip to next patch
                elif choice == "3":
                    log_info(
                        f"Stopped. Applied: {applied}, Failed: {len(failed)}, Skipped: {skipped}"
                    )
                    return applied, failed
                else:
                    log_error("Invalid choice. Please enter 1, 2, or 3.")

        if not patch_path.exists():
            log_warning(f"  Patch not found: {display_name}")
            failed.append(display_name)
            continue

        # Apply the patch
        success, error = apply_single_patch(
            patch_path, chromium_src, dry_run, patches_dir, reset_to
        )

        if success:
            applied += 1
        else:
            failed.append(display_name)

            if interactive and not dry_run:
                # Interactive error handling
                log_error("\n" + "=" * 60)
                log_error(f"Patch {display_name} failed to apply")

                while True:
                    choice = input(
                        "\nOptions:\n  1) Continue with next patch\n  2) Abort\n  3) Fix manually and continue\nChoice (1-3): "
                    ).strip()

                    if choice == "1":
                        break  # Continue to next patch
                    elif choice == "2":
                        raise RuntimeError(f"Aborted at patch: {display_name}")
                    elif choice == "3":
                        input("Fix the issue manually, then press Enter to continue...")
                        applied += 1  # Count as applied since user fixed it
                        failed.pop()  # Remove from failed list
                        break
                    else:
                        log_error("Invalid choice.")

    return applied, failed
