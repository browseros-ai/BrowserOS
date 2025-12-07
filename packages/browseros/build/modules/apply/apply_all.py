"""
Apply All - Apply all patches from patches directory.
"""

from typing import List, Tuple, Optional

from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_error, log_warning, log_success
from .common import find_patch_files, process_patch_list


def apply_all_patches(
    build_ctx: Context,
    dry_run: bool = False,
    interactive: bool = False,
    reset_to: Optional[str] = None,
) -> Tuple[int, List[str]]:
    """Apply all patches from patches directory.

    Args:
        build_ctx: Build context
        dry_run: Only check if patches would apply
        interactive: Ask for confirmation before each patch
        reset_to: Commit to reset files to before applying (optional)

    Returns:
        Tuple of (applied_count, failed_list)
    """
    patches_dir = build_ctx.get_patches_dir()

    if not patches_dir.exists():
        log_warning(f"Patches directory does not exist: {patches_dir}")
        return 0, []

    # Find all patch files
    patch_files = find_patch_files(patches_dir)

    if not patch_files:
        log_warning("No patch files found")
        return 0, []

    log_info(f"Found {len(patch_files)} patches")

    if dry_run:
        log_info("DRY RUN - No changes will be made")

    # Create patch list with display names
    patch_list = [(p, p.relative_to(patches_dir)) for p in patch_files]

    # Process patches
    applied, failed = process_patch_list(
        patch_list,
        build_ctx.chromium_src,
        patches_dir,
        dry_run,
        interactive,
        reset_to=reset_to,
    )

    # Summary
    log_info(f"\nSummary: {applied} applied, {len(failed)} failed")

    if failed:
        log_error("Failed patches:")
        for p in failed:
            log_error(f"  - {p}")

    return applied, failed


class ApplyAllModule(CommandModule):
    """Apply all patches from chromium_patches/"""

    produces = []
    requires = []
    description = "Apply all patches from chromium_patches/"

    def validate(self, ctx: Context) -> None:
        """Validate git is available"""
        import shutil

        if not shutil.which("git"):
            raise ValidationError("Git is not available in PATH")
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

    def execute(
        self,
        ctx: Context,
        interactive: bool = True,
        reset_to: Optional[str] = None,
        annotate: bool = False,
        **kwargs,
    ) -> None:
        """Execute apply all patches

        Args:
            interactive: Interactive mode (ask before each patch)
            reset_to: Commit to reset files to before applying (optional)
            annotate: Create git commits per feature after applying
        """
        applied, failed = apply_all_patches(
            ctx,
            dry_run=False,
            interactive=interactive,
            reset_to=reset_to,
        )
        if failed:
            raise RuntimeError(f"Failed to apply {len(failed)} patches")

        # Run annotate if requested
        if annotate:
            from ..annotate import annotate_features

            log_info("\n" + "=" * 60)
            log_info("🏗️  Creating feature-based commits...")
            commits, skipped = annotate_features(ctx)
            if commits > 0:
                log_success(f"✓ Created {commits} commit(s)")
            else:
                log_info("No commits created (no modified files found)")
