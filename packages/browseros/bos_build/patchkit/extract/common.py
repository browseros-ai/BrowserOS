"""
Common functions shared across extract module commands.

Contains core extraction logic used by extract_commit and extract_range.
"""

import click
from typing import Dict, List, Optional, Tuple

from ...core.context import Context
from ...lib.utils import log_info, log_error, log_warning
from .utils import (
    FilePatch,
    FileOperation,
    GitError,
    run_git_command,
    parse_diff_output,
    write_patch_file,
    create_deletion_marker,
    create_binary_marker,
    log_extraction_summary,
    get_commit_changed_files_with_status,
)


def resolve_base_commit(ctx: Context, base: Optional[str]) -> str:
    """Return an explicit base or the package BASE_COMMIT used for Chromium patches."""
    if base:
        return base

    base_path = ctx.root_dir / "BASE_COMMIT"
    try:
        resolved = base_path.read_text(encoding="utf-8").strip()
    except FileNotFoundError as exc:
        raise GitError(f"BASE_COMMIT not found: {base_path}") from exc

    if not resolved:
        raise GitError(f"BASE_COMMIT is empty: {base_path}")
    return resolved


def check_overwrite(ctx: Context, file_patches: Dict, verbose: bool) -> bool:
    """Check for existing patches and prompt for overwrite"""
    existing_patches = []
    for file_path in file_patches.keys():
        patch_path = ctx.get_patch_path_for_file(file_path)
        if patch_path.exists():
            existing_patches.append(file_path)

    if existing_patches:
        log_warning(f"Found {len(existing_patches)} existing patches")
        if verbose:
            for path in existing_patches[:5]:
                log_warning(f"  - {path}")
            if len(existing_patches) > 5:
                log_warning(f"  ... and {len(existing_patches) - 5} more")

        if not click.confirm("Overwrite existing patches?", default=False):
            log_info("Extraction cancelled")
            return False
    return True


def write_patches(
    ctx: Context,
    file_patches: Dict[str, FilePatch],
    verbose: bool,
    include_binary: bool,
) -> Tuple[int, List[str]]:
    """Write patches to disk.

    Returns:
        Tuple of (success_count, list of successfully extracted file paths)
    """
    success_count = 0
    fail_count = 0
    skip_count = 0
    extracted_files: List[str] = []

    for file_path, patch in file_patches.items():
        if verbose:
            op_str = patch.operation.value.capitalize()
            log_info(f"Processing ({op_str}): {file_path}")

        # Handle different operations
        if patch.operation == FileOperation.DELETE:
            # Create deletion marker
            result = create_deletion_marker(ctx, file_path)
            if result is True:
                success_count += 1
                extracted_files.append(file_path)
            elif result is False:
                fail_count += 1
            else:  # None = user skipped
                skip_count += 1

        elif patch.is_binary:
            if include_binary:
                # Create binary marker
                if create_binary_marker(ctx, file_path, patch.operation):
                    success_count += 1
                    extracted_files.append(file_path)
                else:
                    fail_count += 1
            else:
                log_warning(f"  Skipping binary file: {file_path}")
                skip_count += 1

        elif patch.operation == FileOperation.RENAME:
            # Write patch with rename info
            if patch.patch_content:
                # If there are changes beyond the rename
                if write_patch_file(ctx, file_path, patch.patch_content):
                    success_count += 1
                    extracted_files.append(file_path)
                else:
                    fail_count += 1
            else:
                # Pure rename - create marker
                marker_path = ctx.get_patches_dir() / file_path
                marker_path = marker_path.with_suffix(marker_path.suffix + ".rename")
                marker_path.parent.mkdir(parents=True, exist_ok=True)
                try:
                    marker_content = f"Renamed from: {patch.old_path}\nSimilarity: {patch.similarity}%\n"
                    marker_path.write_text(marker_content)
                    log_info(f"  Rename marked: {file_path}")
                    success_count += 1
                    extracted_files.append(file_path)
                except Exception as e:
                    log_error(f"  Failed to mark rename: {e}")
                    fail_count += 1

        else:
            # Normal patch (ADD, MODIFY, COPY)
            if patch.patch_content:
                if write_patch_file(ctx, file_path, patch.patch_content):
                    success_count += 1
                    extracted_files.append(file_path)
                else:
                    fail_count += 1
            else:
                log_warning(f"  No patch content for: {file_path}")
                skip_count += 1

    # Log summary
    log_extraction_summary(file_patches)

    if fail_count > 0:
        log_warning(f"Failed to extract {fail_count} patches")
    if skip_count > 0:
        log_info(f"Skipped {skip_count} files")

    return success_count, extracted_files


def extract_with_base(
    ctx: Context,
    commit_hash: str,
    base: str,
    verbose: bool,
    force: bool,
    include_binary: bool,
) -> Tuple[int, List[str]]:
    """Extract patches with custom base (full diff from base for files in commit).

    Uses git's --name-status to get accurate operation types, avoiding inference
    bugs with edge cases like files that were added after base and then deleted.

    Returns:
        Tuple of (count, list of extracted file paths)
    """

    # Step 1: Get files changed in commit WITH their status (A/M/D/R/C)
    changed_files = get_commit_changed_files_with_status(commit_hash, ctx.chromium_src)

    if not changed_files:
        log_warning(f"No files changed in commit {commit_hash}")
        return 0, []

    if verbose:
        log_info(f"Files changed in {commit_hash}: {len(changed_files)}")

    # Step 2: Process each file based on its status
    file_patches = {}

    for file_path, status in changed_files.items():
        if verbose:
            log_info(f"  Processing ({status}): {file_path}")

        # Handle deletions directly - trust git's status, no inference needed
        if status == "D":
            file_patches[file_path] = FilePatch(
                file_path=file_path,
                operation=FileOperation.DELETE,
                patch_content=None,
                is_binary=False,
            )
            continue

        # For A/M/R/C: get diff from base to commit
        diff_cmd = ["git", "diff", f"{base}..{commit_hash}", "--", file_path]
        if include_binary:
            diff_cmd.append("--binary")

        result = run_git_command(diff_cmd, cwd=ctx.chromium_src)

        if result.returncode != 0:
            log_warning(f"Failed to get diff for {file_path}")
            continue

        if result.stdout.strip():
            patches = parse_diff_output(result.stdout)
            if patches:
                file_patches.update(patches)
        elif status == "A":
            # Added file with no diff from base - file may not exist in base
            # Try getting full content as new file
            show_cmd = ["git", "show", f"{commit_hash}:{file_path}"]
            show_result = run_git_command(show_cmd, cwd=ctx.chromium_src)
            if show_result.returncode == 0 and show_result.stdout:
                # Create a synthetic add patch
                file_patches[file_path] = FilePatch(
                    file_path=file_path,
                    operation=FileOperation.ADD,
                    patch_content=None,  # Will be handled specially
                    is_binary=False,
                )
                log_warning(f"  Added file needs manual handling: {file_path}")

    if not file_patches:
        log_warning("No patches to extract")
        return 0, []

    log_info(f"Extracting {len(file_patches)} patches with base {base}")

    # Check for existing patches
    if not force and not check_overwrite(ctx, file_patches, verbose):
        return 0, []

    # Write patches
    return write_patches(ctx, file_patches, verbose, include_binary)
