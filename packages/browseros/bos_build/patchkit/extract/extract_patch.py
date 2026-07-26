"""
Extract Patch - Extract patch for a single chromium file.
"""

from typing import Optional, Tuple

from ...core.context import Context
from ...lib.utils import log_info
from .utils import (
    run_git_command,
    parse_diff_output,
    write_patch_file,
    create_deletion_marker,
    validate_commit_exists,
    FileOperation,
    GitError,
)
from .common import resolve_base_commit


def extract_single_file_patch(
    build_ctx: Context,
    chromium_path: str,
    base: Optional[str] = None,
    force: bool = False,
) -> Tuple[bool, Optional[str]]:
    """Extract patch for a single chromium file.

    Extracts the diff from base commit to current working directory
    (including unstaged changes) for the specified file.

    Args:
        build_ctx: Build context
        chromium_path: Path to file in chromium (e.g., chrome/common/foo.h)
        base: Base commit to diff against. Defaults to BASE_COMMIT.
        force: If True, overwrite existing patch without prompting

    Returns:
        Tuple of (success: bool, error_message: Optional[str])
    """
    try:
        base_commit = resolve_base_commit(build_ctx, base)
    except GitError as e:
        return False, str(e)

    if not validate_commit_exists(base_commit, build_ctx.chromium_src):
        return False, f"Base commit not found: {base_commit}"

    log_info(f"Extracting patch for: {chromium_path}")
    log_info(f"  Base: {base_commit[:12]}")

    # Get diff from base to working directory for this file
    diff_cmd = ["git", "diff", base_commit, "--", chromium_path]
    result = run_git_command(diff_cmd, cwd=build_ctx.chromium_src)

    if result.returncode != 0:
        return False, f"Failed to get diff: {result.stderr}"

    if not result.stdout.strip():
        # No diff - check if file exists in base vs working directory
        base_exists = (
            run_git_command(
                ["git", "cat-file", "-e", f"{base_commit}:{chromium_path}"],
                cwd=build_ctx.chromium_src,
            ).returncode
            == 0
        )

        working_file = build_ctx.chromium_src / chromium_path
        working_exists = working_file.exists()

        if not base_exists and not working_exists:
            return (
                False,
                f"File does not exist in base or working directory: {chromium_path}",
            )

        if base_exists and working_exists:
            return False, f"No changes found for: {chromium_path}"

        if not base_exists and working_exists:
            # New file - get full content as diff
            diff_cmd = ["git", "diff", "--no-index", "/dev/null", chromium_path]
            result = run_git_command(diff_cmd, cwd=build_ctx.chromium_src)
            # --no-index returns 1 when files differ, which is expected
            if not result.stdout.strip():
                return False, f"Failed to generate diff for new file: {chromium_path}"

    # Parse the diff
    file_patches = parse_diff_output(result.stdout)

    if not file_patches:
        return False, f"Failed to parse diff for: {chromium_path}"

    if chromium_path not in file_patches:
        # The file might be in the patches under a different key
        if len(file_patches) == 1:
            patch = list(file_patches.values())[0]
        else:
            return False, f"Unexpected diff output for: {chromium_path}"
    else:
        patch = file_patches[chromium_path]

    # Check for existing patch
    patch_path = build_ctx.get_patch_path_for_file(chromium_path)
    if patch_path.exists() and not force:
        import click

        if not click.confirm(
            f"Patch already exists: {chromium_path}. Overwrite?", default=False
        ):
            log_info("Extraction cancelled")
            return False, "Cancelled by user"

    # Handle different operations
    if patch.operation == FileOperation.DELETE:
        if create_deletion_marker(build_ctx, chromium_path):
            return True, None
        return False, f"Failed to create deletion marker for: {chromium_path}"

    if patch.is_binary:
        return False, f"Binary files not supported: {chromium_path}"

    if not patch.patch_content:
        return False, f"No patch content for: {chromium_path}"

    # Write the patch
    if write_patch_file(build_ctx, chromium_path, patch.patch_content):
        return True, None

    return False, f"Failed to write patch for: {chromium_path}"
