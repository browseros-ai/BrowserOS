"""
Apply Feature - Apply patches for a specific feature.
"""

import yaml
from typing import List, Tuple, Optional

from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_error, log_warning, log_success
from .common import process_patch_list


def apply_feature_patches(
    build_ctx: Context,
    feature_name: str,
    dry_run: bool = False,
    reset_to: Optional[str] = None,
) -> Tuple[int, List[str]]:
    """Apply patches for a specific feature.

    Args:
        build_ctx: Build context
        feature_name: Name of the feature
        dry_run: Only check if patches would apply
        reset_to: Commit to reset files to before applying (optional)

    Returns:
        Tuple of (applied_count, failed_list)
    """
    # Load features.yaml
    features_path = build_ctx.get_features_yaml_path()
    if not features_path.exists():
        log_error("No features.yaml found")
        return 0, []

    with open(features_path) as f:
        data = yaml.safe_load(f)

    features = data.get("features", {})

    if feature_name not in features:
        log_error(f"Feature '{feature_name}' not found")
        log_info("Available features:")
        for name in features:
            log_info(f"  - {name}")
        return 0, []

    file_list = features[feature_name].get("files", [])

    if not file_list:
        log_warning(f"Feature '{feature_name}' has no files")
        return 0, []

    log_info(f"Applying patches for feature '{feature_name}' ({len(file_list)} files)")

    if dry_run:
        log_info("DRY RUN - No changes will be made")

    # Create patch list
    patches_dir = build_ctx.get_patches_dir()
    patch_list = []
    for file_path in file_list:
        patch_path = build_ctx.get_patch_path_for_file(file_path)
        patch_list.append((patch_path, file_path))

    # Process patches
    applied, failed = process_patch_list(
        patch_list,
        build_ctx.chromium_src,
        patches_dir,
        dry_run,
        interactive=False,  # Feature patches don't support interactive mode
        reset_to=reset_to,
    )

    # Summary
    log_info(f"\nSummary: {applied} applied, {len(failed)} failed")

    if failed:
        log_error("Failed patches:")
        for p in failed:
            log_error(f"  - {p}")

    return applied, failed


class ApplyFeatureModule(CommandModule):
    """Apply patches for a specific feature"""

    produces = []
    requires = []
    description = "Apply patches for a specific feature"

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
        feature_name: str,
        interactive: bool = True,
        reset_to: Optional[str] = None,
        annotate: bool = False,
        **kwargs,
    ) -> None:
        """Execute apply feature patches

        Args:
            feature_name: Name of the feature to apply
            interactive: Interactive mode (ask before each patch)
            reset_to: Commit to reset files to before applying (optional)
            annotate: Create git commit for this feature after applying
        """
        applied, failed = apply_feature_patches(
            ctx,
            feature_name,
            dry_run=False,
            reset_to=reset_to,
        )
        if failed:
            raise RuntimeError(
                f"Failed to apply {len(failed)} patches for feature '{feature_name}'"
            )

        # Run annotate for this specific feature if requested
        if annotate:
            from ..annotate import annotate_single_feature

            log_info("\n" + "=" * 60)
            log_info(f"🏗️  Creating commit for feature '{feature_name}'...")
            if annotate_single_feature(ctx, feature_name):
                log_success(f"✓ Created commit for '{feature_name}'")
            else:
                log_info("No commit created (no modified files found)")
