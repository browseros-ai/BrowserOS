"""Non-interactive batch application of chromium_patches/.

The build pipeline's patch step runs this on pristine pinned trees;
patches either apply cleanly or the build fails loudly. Interactive
conflict workflows belong to the Rust bpatch tool — runners must never
need it, so this stays in Python.
"""

from pathlib import Path
from typing import TYPE_CHECKING, List, Optional, Tuple

from ..lib.utils import log_info, log_error, log_success, log_warning
from .extract.utils import run_git_command

if TYPE_CHECKING:
    from ..core.context import Context


def file_exists_in_commit(file_path: str, commit: str, chromium_src: Path) -> bool:
    """Check if file exists in a commit."""
    result = run_git_command(
        ["git", "cat-file", "-e", f"{commit}:{file_path}"],
        cwd=chromium_src,
    )
    return result.returncode == 0


def reset_file_to_commit(file_path: str, commit: str, chromium_src: Path) -> bool:
    """Reset a single file to a specific commit state."""
    result = run_git_command(
        ["git", "checkout", commit, "--", file_path],
        cwd=chromium_src,
    )
    return result.returncode == 0


# Marker files annotate a patch path (deleted/binary/renamed upstream) and
# are never applied; the doctor maps them back onto their base path.
MARKER_SUFFIXES = (".deleted", ".binary", ".rename")

METADATA_ROOT_FILES = (
    ".features.yaml",
    ".store.yaml",
    "features.yaml",
    "store.yaml",
)


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
            and not p.name.endswith(MARKER_SUFFIXES)
            and not p.name.startswith(".")
            and str(p.relative_to(patches_dir)) not in METADATA_ROOT_FILES
        ]
    )


def check_patch_applies(
    patch_path: Path, chromium_src: Path
) -> Tuple[bool, Optional[str]]:
    """Quiet dry-run of one patch (git apply --check); never modifies the tree."""
    result = run_git_command(
        ["git", "apply", "--check", "-p1", str(patch_path)], cwd=chromium_src
    )
    if result.returncode == 0:
        return True, None
    return False, result.stderr


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
        else:
            # File doesn't exist in target commit - delete it so patch can create fresh
            target_file = chromium_src / file_path
            if target_file.exists():
                log_info(f"  Deleting (not in {reset_to[:8]}): {file_path}")
                target_file.unlink()

    if dry_run:
        success, error = check_patch_applies(patch_path, chromium_src)
        if success:
            log_success(f"  ✓ Would apply: {display_path}")
            return True, None
        else:
            log_error(f"  ✗ Would fail: {display_path}")
            return False, error
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


def apply_all_patches(
    build_ctx: "Context",
    dry_run: bool = False,
    interactive: bool = False,
    reset_to: Optional[str] = None,
) -> Tuple[int, List[str]]:
    """Apply every patch under chromium_patches/ in sorted order.

    Returns (applied_count, failed_list); the pipeline step treats a
    non-empty failed list as a build failure.
    """
    patches_dir = build_ctx.get_patches_dir()

    if not patches_dir.exists():
        log_warning(f"Patches directory does not exist: {patches_dir}")
        return 0, []

    patch_files = find_patch_files(patches_dir)

    if not patch_files:
        log_warning("No patch files found")
        return 0, []

    log_info(f"Found {len(patch_files)} patches")

    if dry_run:
        log_info("DRY RUN - No changes will be made")

    patch_list = [(p, p.relative_to(patches_dir)) for p in patch_files]

    applied, failed = process_patch_list(
        patch_list,
        build_ctx.chromium_src,
        patches_dir,
        dry_run,
        interactive,
        reset_to=reset_to,
    )

    log_info(f"\nSummary: {applied} applied, {len(failed)} failed")

    if failed:
        log_error("Failed patches:")
        for p in failed:
            log_error(f"  - {p}")

    return applied, failed
