"""
Common functions shared across extract module commands.

Contains core extraction logic used by extract_commit and extract_range.
"""

import click
from pathlib import Path
from typing import Dict, List, Optional, Tuple

from ...common.context import Context
from ...common.utils import log_info, log_error, log_warning
from .utils import (
    FilePatch,
    FileOperation,
    run_git_command,
    parse_diff_output,
    write_patch_file,
    create_deletion_marker,
    create_binary_marker,
    log_extraction_summary,
    get_commit_changed_files,
)


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
            if create_deletion_marker(ctx, file_path):
                success_count += 1
                extracted_files.append(file_path)
            else:
                fail_count += 1

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


def extract_normal(
    ctx: Context,
    commit_hash: str,
    verbose: bool,
    force: bool,
    include_binary: bool,
) -> Tuple[int, List[str]]:
    """Extract patches normally (diff against parent).

    Returns:
        Tuple of (count, list of extracted file paths)
    """
    from .utils import GitError

    # Get diff against parent
    diff_cmd = ["git", "diff", f"{commit_hash}^..{commit_hash}"]
    if include_binary:
        diff_cmd.append("--binary")

    result = run_git_command(diff_cmd, cwd=ctx.chromium_src)

    if result.returncode != 0:
        raise GitError(f"Failed to get diff for commit {commit_hash}: {result.stderr}")

    # Parse diff into file patches
    file_patches = parse_diff_output(result.stdout)

    if not file_patches:
        log_warning("No changes found in commit")
        return 0, []

    # Check for existing patches
    if not force and not check_overwrite(ctx, file_patches, verbose):
        return 0, []

    # Write patches
    return write_patches(ctx, file_patches, verbose, include_binary)


def extract_with_base(
    ctx: Context,
    commit_hash: str,
    base: str,
    verbose: bool,
    force: bool,
    include_binary: bool,
) -> Tuple[int, List[str]]:
    """Extract patches with custom base (full diff from base for files in commit).

    Returns:
        Tuple of (count, list of extracted file paths)
    """

    # Step 1: Get list of files changed in the commit
    changed_files = get_commit_changed_files(commit_hash, ctx.chromium_src)

    if not changed_files:
        log_warning(f"No files changed in commit {commit_hash}")
        return 0, []

    if verbose:
        log_info(f"Files changed in {commit_hash}: {len(changed_files)}")

    # Step 2: For each file, get diff from base to commit
    file_patches = {}

    for file_path in changed_files:
        if verbose:
            log_info(f"  Getting diff for: {file_path}")

        # Get diff for this specific file from base to commit
        diff_cmd = ["git", "diff", f"{base}..{commit_hash}", "--", file_path]
        if include_binary:
            diff_cmd.append("--binary")

        result = run_git_command(diff_cmd, cwd=ctx.chromium_src)

        if result.returncode != 0:
            log_warning(f"Failed to get diff for {file_path}")
            continue

        if result.stdout.strip():
            # Parse this single file's diff
            patches = parse_diff_output(result.stdout)
            # Should only have one file in the result
            if patches:
                file_patches.update(patches)
        else:
            # File might have been added/deleted
            # Check if file exists in base and commit
            base_exists = (
                run_git_command(
                    ["git", "cat-file", "-e", f"{base}:{file_path}"],
                    cwd=ctx.chromium_src,
                ).returncode
                == 0
            )

            commit_exists = (
                run_git_command(
                    ["git", "cat-file", "-e", f"{commit_hash}:{file_path}"],
                    cwd=ctx.chromium_src,
                ).returncode
                == 0
            )

            if not base_exists and commit_exists:
                # File was added - get full content
                diff_cmd = ["git", "diff", f"{base}..{commit_hash}", "--", file_path]
                if include_binary:
                    diff_cmd.append("--binary")
                result = run_git_command(diff_cmd, cwd=ctx.chromium_src)
                if result.stdout.strip():
                    patches = parse_diff_output(result.stdout)
                    if patches:
                        file_patches.update(patches)
            elif base_exists and not commit_exists:
                # File was deleted
                file_patches[file_path] = FilePatch(
                    file_path=file_path,
                    operation=FileOperation.DELETE,
                    patch_content=None,
                    is_binary=False,
                )

    if not file_patches:
        log_warning("No patches to extract")
        return 0, []

    log_info(f"Extracting {len(file_patches)} patches with base {base}")

    # Check for existing patches
    if not force and not check_overwrite(ctx, file_patches, verbose):
        return 0, []

    # Write patches
    return write_patches(ctx, file_patches, verbose, include_binary)
