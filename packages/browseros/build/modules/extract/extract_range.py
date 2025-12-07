"""
Extract Range - Extract patches from a range of git commits.
"""

import click
from pathlib import Path
from typing import List, Optional, Tuple

from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_error, log_success, log_warning
from .utils import (
    FileOperation,
    GitError,
    run_git_command,
    validate_git_repository,
    validate_commit_exists,
    parse_diff_output,
    write_patch_file,
    create_deletion_marker,
    create_binary_marker,
    log_extraction_summary,
)
from .common import check_overwrite, extract_with_base
from .extract_commit import extract_single_commit


def extract_commit_range(
    ctx: Context,
    base_commit: str,
    head_commit: str,
    verbose: bool = False,
    force: bool = False,
    include_binary: bool = False,
    custom_base: Optional[str] = None,
) -> Tuple[int, List[str]]:
    """Extract patches from a commit range as a single cumulative diff

    Returns:
        Tuple of (count, list of extracted file paths)
    """
    # Step 1: Validate commits
    if not validate_commit_exists(base_commit, ctx.chromium_src):
        raise GitError(f"Base commit not found: {base_commit}")
    if not validate_commit_exists(head_commit, ctx.chromium_src):
        raise GitError(f"Head commit not found: {head_commit}")
    if custom_base and not validate_commit_exists(custom_base, ctx.chromium_src):
        raise GitError(f"Custom base commit not found: {custom_base}")

    # Count commits in range for progress
    result = run_git_command(
        ["git", "rev-list", "--count", f"{base_commit}..{head_commit}"],
        cwd=ctx.chromium_src,
    )
    commit_count = int(result.stdout.strip()) if result.returncode == 0 else 0

    if commit_count == 0:
        log_warning(f"No commits between {base_commit} and {head_commit}")
        return 0, []

    log_info(f"Processing {commit_count} commits")

    # Step 2: Get diff based on whether we have a custom base
    if custom_base:
        # First get list of files changed in the range
        range_files_cmd = [
            "git",
            "diff",
            "--name-only",
            f"{base_commit}..{head_commit}",
        ]
        result = run_git_command(range_files_cmd, cwd=ctx.chromium_src)

        if result.returncode != 0:
            raise GitError(f"Failed to get changed files: {result.stderr}")

        changed_files = (
            result.stdout.strip().split("\n") if result.stdout.strip() else []
        )

        if not changed_files:
            log_warning("No files changed in range")
            return 0, []

        log_info(f"Found {len(changed_files)} files changed in range")

        # Now get diff from custom base to head for these files
        diff_cmd = ["git", "diff", f"{custom_base}..{head_commit}"]
        if include_binary:
            diff_cmd.append("--binary")
        # Add the specific files to diff command
        diff_cmd.append("--")
        diff_cmd.extend(changed_files)
    else:
        # Regular diff from base_commit to head_commit
        diff_cmd = ["git", "diff", f"{base_commit}..{head_commit}"]
        if include_binary:
            diff_cmd.append("--binary")

    result = run_git_command(diff_cmd, cwd=ctx.chromium_src, timeout=120)

    if result.returncode != 0:
        raise GitError(f"Failed to get diff for range: {result.stderr}")

    # Step 3-5: Process diff
    file_patches = parse_diff_output(result.stdout)

    if not file_patches:
        log_warning("No changes found in commit range")
        return 0, []

    # Check for existing patches
    if not force and not check_overwrite(ctx, file_patches, verbose):
        return 0, []

    success_count = 0
    fail_count = 0
    skip_count = 0
    extracted_files: List[str] = []

    # Process with progress indicator
    with click.progressbar(
        file_patches.items(),
        label="Extracting patches",
        show_pos=True,
        show_percent=True,
    ) as patches_bar:
        for file_path, patch in patches_bar:
            # Handle different operations
            if patch.operation == FileOperation.DELETE:
                if create_deletion_marker(ctx, file_path):
                    success_count += 1
                    extracted_files.append(file_path)
                else:
                    fail_count += 1

            elif patch.is_binary:
                if include_binary:
                    if create_binary_marker(ctx, file_path, patch.operation):
                        success_count += 1
                        extracted_files.append(file_path)
                    else:
                        fail_count += 1
                else:
                    skip_count += 1

            elif patch.patch_content:
                if write_patch_file(ctx, file_path, patch.patch_content):
                    success_count += 1
                    extracted_files.append(file_path)
                else:
                    fail_count += 1
            else:
                skip_count += 1

    # Step 6: Log summary
    log_extraction_summary(file_patches)

    if fail_count > 0:
        log_warning(f"Failed to extract {fail_count} patches")
    if skip_count > 0:
        log_info(f"Skipped {skip_count} files")

    return success_count, extracted_files


def extract_commits_individually(
    ctx: Context,
    base_commit: str,
    head_commit: str,
    verbose: bool = False,
    force: bool = False,
    include_binary: bool = False,
    custom_base: Optional[str] = None,
) -> Tuple[int, List[str]]:
    """Extract patches from each commit in a range individually

    This preserves commit boundaries and can help with conflict resolution.

    Returns:
        Tuple of (count, list of extracted file paths)
    """
    # Validate custom base if provided
    if custom_base and not validate_commit_exists(custom_base, ctx.chromium_src):
        raise GitError(f"Custom base commit not found: {custom_base}")

    # Get list of commits in range
    result = run_git_command(
        ["git", "rev-list", "--reverse", f"{base_commit}..{head_commit}"],
        cwd=ctx.chromium_src,
    )

    if result.returncode != 0:
        raise GitError(f"Failed to list commits: {result.stderr}")

    commits = [c.strip() for c in result.stdout.strip().split("\n") if c.strip()]

    if not commits:
        log_warning(f"No commits between {base_commit} and {head_commit}")
        return 0, []

    log_info(f"Extracting patches from {len(commits)} commits individually")
    if custom_base:
        log_info(f"Using custom base: {custom_base}")

    total_extracted = 0
    all_extracted_files: List[str] = []
    failed_commits = []

    with click.progressbar(
        commits, label="Processing commits", show_pos=True, show_percent=True
    ) as commits_bar:
        for commit in commits_bar:
            try:
                if custom_base:
                    # Use extract_with_base for full diff from custom base
                    extracted, files = extract_with_base(
                        ctx,
                        commit,
                        custom_base,
                        verbose=False,
                        force=force,
                        include_binary=include_binary,
                    )
                else:
                    # Normal extraction from parent
                    extracted, files = extract_single_commit(
                        ctx,
                        commit,
                        verbose=False,
                        force=force,
                        include_binary=include_binary,
                    )
                total_extracted += extracted
                all_extracted_files.extend(files)
            except GitError as e:
                failed_commits.append((commit, str(e)))
                if verbose:
                    log_error(f"Failed to extract {commit}: {e}")

    if failed_commits:
        log_warning(f"Failed to extract {len(failed_commits)} commits:")
        for commit, error in failed_commits[:5]:
            log_warning(f"  - {commit[:8]}: {error}")
        if len(failed_commits) > 5:
            log_warning(f"  ... and {len(failed_commits) - 5} more")

    # Deduplicate files (same file may appear in multiple commits)
    unique_files = list(dict.fromkeys(all_extracted_files))
    return total_extracted, unique_files


class ExtractRangeModule(CommandModule):
    """Extract patches from a range of commits"""
    produces = []
    requires = []
    description = "Extract patches from a range of commits"

    def validate(self, ctx: Context) -> None:
        """Validate git repository"""
        import shutil
        if not shutil.which("git"):
            raise ValidationError("Git is not available in PATH")
        if not validate_git_repository(ctx.chromium_src):
            raise ValidationError(f"Not a git repository: {ctx.chromium_src}")

    def execute(
        self,
        ctx: Context,
        start: str,
        end: str,
        output: Optional[Path] = None,
        interactive: bool = True,
        verbose: bool = False,
        force: bool = False,
        include_binary: bool = False,
        squash: bool = False,
        base: Optional[str] = None,
        feature: bool = False,
    ) -> None:
        """Execute extract range

        Args:
            start: Start commit (exclusive)
            end: End commit (inclusive)
            output: Output directory (unused, kept for compatibility)
            interactive: Interactive mode (unused, kept for compatibility)
            verbose: Show detailed output
            force: Overwrite existing patches
            include_binary: Include binary files
            squash: Squash all commits into single patches
            base: Use different base for diff (full diff from base for files in range)
            feature: Prompt to add extracted files to a feature in features.yaml
        """
        try:
            if squash:
                count, extracted_files = extract_commit_range(
                    ctx,
                    base_commit=start,
                    head_commit=end,
                    verbose=verbose,
                    force=force,
                    include_binary=include_binary,
                    custom_base=base,
                )
            else:
                count, extracted_files = extract_commits_individually(
                    ctx,
                    base_commit=start,
                    head_commit=end,
                    verbose=verbose,
                    force=force,
                    include_binary=include_binary,
                    custom_base=base,
                )
            if count == 0:
                log_warning(f"No patches extracted from range {start}..{end}")
            else:
                log_success(f"Successfully extracted {count} patches from {start}..{end}")

                # Handle --feature flag
                if feature and extracted_files:
                    self._add_to_feature(ctx, end, extracted_files)

        except GitError as e:
            raise RuntimeError(f"Git error: {e}")

    def _add_to_feature(self, ctx: Context, commit: str, files: List[str]) -> None:
        """Prompt user to add extracted files to a feature."""
        from ..feature import prompt_feature_selection, add_files_to_feature
        from .utils import get_commit_info

        # Get commit info for context (use the end commit)
        commit_info = get_commit_info(commit, ctx.chromium_src)
        commit_message = commit_info.get("subject") if commit_info else None

        # Prompt for feature selection
        result = prompt_feature_selection(ctx, commit[:12], commit_message)
        if result is None:
            log_warning("Skipped adding files to feature")
            return

        feature_name, description = result
        add_files_to_feature(ctx, feature_name, description, files)
