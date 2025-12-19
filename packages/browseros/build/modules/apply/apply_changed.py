"""
Apply Changed - Apply patches that changed in specific commits of the browseros repo.

This module enables selective patch application based on git commits,
useful for testing changes on different machines without full rebuild.
"""

from pathlib import Path
from typing import List, Tuple, Optional
from enum import Enum
from dataclasses import dataclass

from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_error, log_success, log_warning
from .common import apply_single_patch
from .utils import (
    run_git_command,
    file_exists_in_commit,
    reset_file_to_commit,
    validate_git_repository,
    validate_commit_exists,
)


def get_git_root(repo_path: Path) -> Path:
    """Get the root directory of the git repository."""
    result = run_git_command(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=repo_path,
    )
    if result.returncode != 0:
        raise RuntimeError(f"Failed to get git root: {result.stderr}")
    return Path(result.stdout.strip())


class ChangeType(Enum):
    """Types of changes to patch files"""
    ADDED = "A"
    MODIFIED = "M"
    DELETED = "D"
    RENAMED = "R"
    COPIED = "C"


@dataclass
class PatchChange:
    """Represents a changed patch file"""
    patch_path: str  # Path relative to browseros repo (e.g., chromium_patches/chrome/foo.cc)
    chromium_path: str  # Path in chromium (e.g., chrome/foo.cc)
    change_type: ChangeType
    old_path: Optional[str] = None  # For renames


def get_changed_files_in_commit(commit: str, repo_path: Path) -> List[Tuple[str, str]]:
    """Get list of changed files in a single commit.

    Returns:
        List of (status, file_path) tuples where status is A/M/D/R/C
    """
    result = run_git_command(
        ["git", "diff-tree", "--no-commit-id", "--name-status", "-r", commit],
        cwd=repo_path,
    )

    if result.returncode != 0:
        raise RuntimeError(f"Failed to get changed files for commit {commit}: {result.stderr}")

    changes = []
    for line in result.stdout.strip().split("\n"):
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) >= 2:
            status = parts[0][0]  # First char (R100 -> R, M -> M, etc.)
            file_path = parts[-1]  # Last element is the new path
            changes.append((status, file_path))

    return changes


def get_changed_files_in_range(start: str, end: str, repo_path: Path) -> List[Tuple[str, str]]:
    """Get list of changed files in a commit range.

    Args:
        start: Start commit (exclusive)
        end: End commit (inclusive)
        repo_path: Path to the repository

    Returns:
        List of (status, file_path) tuples
    """
    result = run_git_command(
        ["git", "diff", "--name-status", f"{start}..{end}"],
        cwd=repo_path,
    )

    if result.returncode != 0:
        raise RuntimeError(f"Failed to get changed files for range {start}..{end}: {result.stderr}")

    changes = []
    for line in result.stdout.strip().split("\n"):
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) >= 2:
            status = parts[0][0]
            file_path = parts[-1]
            changes.append((status, file_path))

    return changes


def filter_patch_changes(
    changes: List[Tuple[str, str]],
    patches_prefix: str = "chromium_patches/"
) -> List[PatchChange]:
    """Filter changes to only chromium_patches/ files and map to chromium paths.

    Args:
        changes: List of (status, file_path) tuples
        patches_prefix: Prefix to filter and strip

    Returns:
        List of PatchChange objects
    """
    patch_changes = []

    for status, file_path in changes:
        if not file_path.startswith(patches_prefix):
            continue

        # Map to chromium path by stripping prefix
        chromium_path = file_path[len(patches_prefix):]

        # Skip empty paths
        if not chromium_path:
            continue

        try:
            change_type = ChangeType(status)
        except ValueError:
            # Unknown status, treat as modified
            change_type = ChangeType.MODIFIED

        patch_changes.append(PatchChange(
            patch_path=file_path,
            chromium_path=chromium_path,
            change_type=change_type,
        ))

    return patch_changes


def format_confirmation_prompt(patch_changes: List[PatchChange]) -> str:
    """Format a confirmation prompt showing all changes grouped by type."""
    lines = []
    lines.append(f"\nFound {len(patch_changes)} changed patch(es):\n")

    # Group by change type
    added = [p for p in patch_changes if p.change_type == ChangeType.ADDED]
    modified = [p for p in patch_changes if p.change_type == ChangeType.MODIFIED]
    deleted = [p for p in patch_changes if p.change_type == ChangeType.DELETED]
    other = [p for p in patch_changes if p.change_type not in (ChangeType.ADDED, ChangeType.MODIFIED, ChangeType.DELETED)]

    if added:
        lines.append(f"  Added ({len(added)}):")
        for p in added:
            lines.append(f"    + {p.chromium_path}")

    if modified:
        lines.append(f"  Modified ({len(modified)}):")
        for p in modified:
            lines.append(f"    ~ {p.chromium_path}")

    if deleted:
        lines.append(f"  Deleted ({len(deleted)}):")
        for p in deleted:
            lines.append(f"    - {p.chromium_path}")

    if other:
        lines.append(f"  Other ({len(other)}):")
        for p in other:
            lines.append(f"    ? {p.chromium_path}")

    return "\n".join(lines)


def apply_changed_patches(
    ctx: Context,
    patch_changes: List[PatchChange],
    reset_to: str,
    dry_run: bool = False,
) -> Tuple[int, int, List[str]]:
    """Apply changed patches to chromium.

    Args:
        ctx: Build context
        patch_changes: List of patch changes to apply
        reset_to: Commit to reset files to before applying
        dry_run: If True, only show what would be done

    Returns:
        Tuple of (applied_count, reset_only_count, failed_list)
    """
    applied = 0
    reset_only = 0
    failed = []

    patches_dir = ctx.get_patches_dir()
    chromium_src = ctx.chromium_src

    for change in patch_changes:
        chromium_path = change.chromium_path
        patch_path = patches_dir / change.chromium_path

        if change.change_type == ChangeType.DELETED:
            # Patch was deleted - just reset file to base (restore original)
            if dry_run:
                log_info(f"  Would reset (patch deleted): {chromium_path}")
                reset_only += 1
            else:
                log_info(f"  Resetting (patch deleted): {chromium_path}")
                if file_exists_in_commit(chromium_path, reset_to, chromium_src):
                    if reset_file_to_commit(chromium_path, reset_to, chromium_src):
                        log_success(f"    ✓ Restored to {reset_to[:8]}: {chromium_path}")
                        reset_only += 1
                    else:
                        log_error(f"    ✗ Failed to reset: {chromium_path}")
                        failed.append(chromium_path)
                else:
                    # File doesn't exist in base - delete it
                    target_file = chromium_src / chromium_path
                    if target_file.exists():
                        target_file.unlink()
                        log_success(f"    ✓ Deleted (not in {reset_to[:8]}): {chromium_path}")
                        reset_only += 1
                    else:
                        log_info(f"    Already absent: {chromium_path}")
                        reset_only += 1
        else:
            # Added or modified - reset and apply patch
            if not patch_path.exists():
                log_error(f"  Patch file not found: {patch_path}")
                failed.append(chromium_path)
                continue

            success, error = apply_single_patch(
                patch_path,
                chromium_src,
                dry_run=dry_run,
                relative_to=patches_dir,
                reset_to=reset_to,
            )

            if success:
                applied += 1
            else:
                failed.append(chromium_path)

    return applied, reset_only, failed


class ApplyChangedModule(CommandModule):
    """Apply patches that changed in specific commits"""

    produces = []
    requires = []
    description = "Apply patches changed in specific commits"

    def validate(self, ctx: Context) -> None:
        """Validate git is available and repos exist"""
        import shutil

        if not shutil.which("git"):
            raise ValidationError("Git is not available in PATH")
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")
        if not validate_git_repository(ctx.root_dir):
            raise ValidationError(f"Not a git repository: {ctx.root_dir}")
        if not validate_git_repository(ctx.chromium_src):
            raise ValidationError(f"Not a git repository: {ctx.chromium_src}")

    def execute(
        self,
        ctx: Context,
        reset_to: str,
        commit: Optional[str] = None,
        range_start: Optional[str] = None,
        range_end: Optional[str] = None,
        dry_run: bool = False,
    ) -> None:
        """Execute apply changed patches.

        Args:
            reset_to: Commit to reset chromium files to before applying (required)
            commit: Single commit hash to get changes from
            range_start: Start of commit range (exclusive)
            range_end: End of commit range (inclusive)
            dry_run: If True, only show what would be done
        """
        # Validate we have either commit or range
        if commit and (range_start or range_end):
            raise RuntimeError("Cannot specify both --commit and --range")
        if not commit and not (range_start and range_end):
            raise RuntimeError("Must specify either --commit or --range")
        if (range_start and not range_end) or (range_end and not range_start):
            raise RuntimeError("--range requires both start and end commits")

        # Get git root and compute the prefix for chromium_patches relative to git root
        git_root = get_git_root(ctx.root_dir)
        # ctx.root_dir might be a subdir of git_root (e.g., packages/browseros)
        # We need to compute the path prefix from git root to chromium_patches
        try:
            relative_root = ctx.root_dir.relative_to(git_root)
            patches_prefix = str(relative_root / "chromium_patches") + "/"
        except ValueError:
            # root_dir is not under git_root, use simple prefix
            patches_prefix = "chromium_patches/"

        log_info(f"Looking for changes in: {patches_prefix}")

        # Validate commits exist in browseros repo
        if commit:
            if not validate_commit_exists(commit, ctx.root_dir):
                raise RuntimeError(f"Commit not found in browseros repo: {commit}")
            log_info(f"Getting changes from commit: {commit}")
            changes = get_changed_files_in_commit(commit, ctx.root_dir)
        else:
            if not validate_commit_exists(range_start, ctx.root_dir):
                raise RuntimeError(f"Start commit not found: {range_start}")
            if not validate_commit_exists(range_end, ctx.root_dir):
                raise RuntimeError(f"End commit not found: {range_end}")
            log_info(f"Getting changes from range: {range_start}..{range_end}")
            changes = get_changed_files_in_range(range_start, range_end, ctx.root_dir)

        # Validate reset_to exists in chromium repo
        if not validate_commit_exists(reset_to, ctx.chromium_src):
            raise RuntimeError(f"Reset commit not found in chromium repo: {reset_to}")

        # Filter to chromium_patches/ only (using computed prefix)
        patch_changes = filter_patch_changes(changes, patches_prefix)

        if not patch_changes:
            log_warning("No chromium_patches/ files changed in the specified commit(s)")
            return

        # Show confirmation prompt
        prompt_text = format_confirmation_prompt(patch_changes)
        log_info(prompt_text)
        log_info(f"\nWill reset files to: {reset_to}")

        if dry_run:
            log_info("\n[DRY RUN - No changes will be made]\n")

        # Ask for confirmation
        response = input("\nProceed? [y/N]: ").strip().lower()
        if response not in ("y", "yes"):
            log_warning("Aborted by user")
            return

        # Apply changes
        log_info("\nApplying changes...")
        applied, reset_only, failed = apply_changed_patches(
            ctx, patch_changes, reset_to, dry_run
        )

        # Summary
        log_info("\n" + "=" * 50)
        log_info("Summary:")
        log_info(f"  Patches applied: {applied}")
        log_info(f"  Files reset only (patch deleted): {reset_only}")
        if failed:
            log_error(f"  Failed: {len(failed)}")
            for f in failed:
                log_error(f"    - {f}")
        log_info("=" * 50)

        if failed:
            raise RuntimeError(f"Failed to apply {len(failed)} patch(es)")
