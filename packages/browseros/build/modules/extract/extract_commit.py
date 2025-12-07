"""
Extract Commit - Extract patches from a single git commit.
"""

from pathlib import Path
from typing import List, Optional, Tuple

from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_success, log_warning
from .utils import (
    GitError,
    validate_git_repository,
    validate_commit_exists,
    get_commit_info,
)
from .common import extract_normal, extract_with_base


def extract_single_commit(
    ctx: Context,
    commit_hash: str,
    verbose: bool = False,
    force: bool = False,
    include_binary: bool = False,
    base: Optional[str] = None,
) -> Tuple[int, List[str]]:
    """Extract patches from a single commit

    Args:
        ctx: Build context
        commit_hash: Commit to extract
        verbose: Show detailed output
        force: Overwrite existing patches
        include_binary: Include binary files
        base: If provided, extract full diff from base for files in commit

    Returns:
        Tuple of (count, list of extracted file paths)
    """
    # Step 1: Validate commit
    if not validate_commit_exists(commit_hash, ctx.chromium_src):
        raise GitError(f"Commit not found: {commit_hash}")

    # Get commit info for logging
    commit_info = get_commit_info(commit_hash, ctx.chromium_src)
    if commit_info and verbose:
        log_info(
            f"  Author: {commit_info['author_name']} <{commit_info['author_email']}>"
        )
        log_info(f"  Subject: {commit_info['subject']}")

    if base:
        # With --base: Get files from commit, but diff from base
        return extract_with_base(ctx, commit_hash, base, verbose, force, include_binary)
    else:
        # Normal behavior: diff against parent
        return extract_normal(ctx, commit_hash, verbose, force, include_binary)


class ExtractCommitModule(CommandModule):
    """Extract patches from a single commit"""
    produces = []
    requires = []
    description = "Extract patches from a single commit"

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
        commit: str,
        output: Optional[Path] = None,
        interactive: bool = True,
        verbose: bool = False,
        force: bool = False,
        include_binary: bool = False,
        base: Optional[str] = None,
        feature: bool = False,
    ) -> None:
        """Execute extract commit

        Args:
            commit: Git commit reference (e.g., HEAD)
            output: Output directory (unused, kept for compatibility)
            interactive: Interactive mode (unused, kept for compatibility)
            verbose: Show detailed output
            force: Overwrite existing patches
            include_binary: Include binary files
            base: Extract full diff from base commit for files in COMMIT
            feature: Prompt to add extracted files to a feature in features.yaml
        """
        try:
            count, extracted_files = extract_single_commit(
                ctx,
                commit_hash=commit,
                verbose=verbose,
                force=force,
                include_binary=include_binary,
                base=base,
            )
            if count == 0:
                log_warning(f"No patches extracted from {commit}")
            else:
                log_success(f"Successfully extracted {count} patches from {commit}")

                # Handle --feature flag
                if feature and extracted_files:
                    self._add_to_feature(ctx, commit, extracted_files)

        except GitError as e:
            raise RuntimeError(f"Git error: {e}")

    def _add_to_feature(self, ctx: Context, commit: str, files: List[str]) -> None:
        """Prompt user to add extracted files to a feature."""
        from ..feature import prompt_feature_selection, add_files_to_feature
        from .utils import get_commit_info

        # Get commit info for context
        commit_info = get_commit_info(commit, ctx.chromium_src)
        commit_message = commit_info.get("subject") if commit_info else None

        # Prompt for feature selection
        result = prompt_feature_selection(ctx, commit[:12], commit_message)
        if result is None:
            log_warning("Skipped adding files to feature")
            return

        feature_name, description = result
        add_files_to_feature(ctx, feature_name, description, files)
