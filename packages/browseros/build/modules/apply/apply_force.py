"""
Apply Force - Non-interactive apply that uses --reject for conflicts.

When a patch fails to apply cleanly, falls back to `git apply --reject`
which applies what it can and writes .rej files for failed hunks.
Always continues to the next patch regardless of failures.
"""

from pathlib import Path
from typing import List, Tuple, Optional

from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_error, log_warning, log_success
from .common import find_patch_files
from .utils import run_git_command, file_exists_in_commit, reset_file_to_commit


def apply_patch_with_reject(
    patch_path: Path,
    chromium_src: Path,
    relative_to: Optional[Path] = None,
    reset_to: Optional[str] = None,
) -> Tuple[str, Optional[str]]:
    """Apply a single patch, falling back to --reject on conflict.

    Args:
        patch_path: Path to the patch file
        chromium_src: Chromium source directory
        relative_to: Base path for displaying relative paths
        reset_to: Commit to reset file to before applying

    Returns:
        Tuple of (status, rej_files) where status is one of:
        - "applied": patch applied cleanly (rej_files is [])
        - "rejected": patch had conflicts, .rej files written (rej_files has full paths)
        - "failed": patch could not be applied at all
    """
    display_path = patch_path.relative_to(relative_to) if relative_to else patch_path

    # Reset file to base commit if requested
    if reset_to:
        file_path = str(display_path)
        if file_exists_in_commit(file_path, reset_to, chromium_src):
            reset_file_to_commit(file_path, reset_to, chromium_src)
        else:
            target_file = chromium_src / file_path
            if target_file.exists():
                target_file.unlink()

    # Try clean apply first (same strategy as common.apply_single_patch)
    result = run_git_command(
        [
            "git", "apply",
            "--ignore-whitespace", "--whitespace=nowarn",
            "-p1", str(patch_path),
        ],
        cwd=chromium_src,
    )

    if result.returncode == 0:
        log_success(f"  Applied: {display_path}")
        return "applied", []

    # Try 3-way merge
    result = run_git_command(
        [
            "git", "apply",
            "--ignore-whitespace", "--whitespace=nowarn",
            "-p1", "--3way",
            str(patch_path),
        ],
        cwd=chromium_src,
    )

    if result.returncode == 0:
        log_success(f"  Applied (3way): {display_path}")
        return "applied", []

    # Fall back to --reject: applies what it can, writes .rej for the rest
    result = run_git_command(
        [
            "git", "apply",
            "--ignore-whitespace", "--whitespace=nowarn",
            "-p1", "--reject",
            str(patch_path),
        ],
        cwd=chromium_src,
    )

    if result.returncode == 0:
        # --reject returned 0 means it applied (possibly with warnings)
        log_success(f"  Applied (reject): {display_path}")
        return "applied", []
    else:
        # Some hunks failed - .rej files were written
        log_warning(f"  Conflict: {display_path}")
        # Find .rej files created for this patch
        rej_path = chromium_src / f"{display_path}.rej"
        rej_files = []
        if rej_path.exists():
            rej_files.append(str(rej_path))
            log_warning(f"    .rej: {rej_path}")
        return "rejected", rej_files


def apply_all_force(
    build_ctx: Context,
    reset_to: Optional[str] = None,
) -> Tuple[int, int, List[str]]:
    """Apply all patches non-interactively, using --reject for conflicts.

    Args:
        build_ctx: Build context
        reset_to: Commit to reset files to before applying

    Returns:
        Tuple of (applied_count, rejected_count, failed_list)
    """
    patches_dir = build_ctx.get_patches_dir()

    if not patches_dir.exists():
        log_warning(f"Patches directory does not exist: {patches_dir}")
        return 0, 0, []

    patch_files = find_patch_files(patches_dir)

    if not patch_files:
        log_warning("No patch files found")
        return 0, 0, []

    log_info(f"Found {len(patch_files)} patches (non-interactive, --reject on conflict)")

    applied = 0
    rejected = 0
    failed = []
    all_rej_files: List[str] = []

    for patch_path in patch_files:
        display_name = str(patch_path.relative_to(patches_dir))

        if not patch_path.exists():
            log_warning(f"  Patch not found: {display_name}")
            failed.append(display_name)
            continue

        status, rej_files = apply_patch_with_reject(
            patch_path, build_ctx.chromium_src, patches_dir, reset_to
        )

        if status == "applied":
            applied += 1
        elif status == "rejected":
            rejected += 1
            failed.append(display_name)
            all_rej_files.extend(rej_files)
        else:
            failed.append(display_name)

    # Summary
    log_info(f"\nSummary: {applied} applied, {rejected} rejected (.rej), {len(failed)} total failed")

    if all_rej_files:
        log_warning("Reject files:")
        for rej in all_rej_files:
            log_warning(f"  {rej}")

    return applied, rejected, failed


class ApplyForceModule(CommandModule):
    """Non-interactive apply with --reject for conflicts"""

    produces = []
    requires = []
    description = "Apply all patches non-interactively, writing .rej files for conflicts"

    def validate(self, ctx: Context) -> None:
        import shutil

        if not shutil.which("git"):
            raise ValidationError("Git is not available in PATH")
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

    def execute(
        self,
        ctx: Context,
        reset_to: Optional[str] = None,
        **kwargs,
    ) -> None:
        applied, rejected, failed = apply_all_force(ctx, reset_to=reset_to)
        if rejected > 0:
            log_warning(
                f"{rejected} patch(es) had conflicts. Review .rej files in chromium source."
            )
