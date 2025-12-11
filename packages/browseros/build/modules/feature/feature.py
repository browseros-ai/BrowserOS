"""
Feature module - Manage feature-to-file mappings

Simple feature management with YAML persistence.
"""

import yaml
from typing import Dict, List, Optional, Tuple
from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ..extract.utils import get_commit_changed_files
from ...common.utils import log_info, log_error, log_success, log_warning
from .validation import validate_description, validate_feature_name, VALID_PREFIXES


def add_or_update_feature(
    ctx: Context,
    feature_name: str,
    commit: str,
    description: str,
) -> Tuple[bool, str]:
    """Add or update a feature with files from a commit.

    If feature exists, merges files (appends new ones).
    If feature is new, creates it.

    Args:
        ctx: Build context
        feature_name: Feature key name (e.g., 'llm-chat')
        commit: Git commit reference
        description: Feature description with prefix (e.g., 'feat: LLM chat')

    Returns:
        Tuple of (success, error_message)
    """
    # Validate inputs
    valid, error = validate_feature_name(feature_name)
    if not valid:
        return False, error

    valid, error = validate_description(description)
    if not valid:
        return False, error

    features_file = ctx.get_features_yaml_path()

    # Get changed files from commit
    changed_files = get_commit_changed_files(commit, ctx.chromium_src)
    if not changed_files:
        return False, f"No changed files found in commit {commit}"

    # Load existing features
    features: Dict = {"version": "1.0", "features": {}}
    if features_file.exists():
        with open(features_file, "r") as f:
            content = yaml.safe_load(f)
            if content:
                features = content
                if "features" not in features:
                    features["features"] = {}

    existing_feature = features["features"].get(feature_name)

    if existing_feature:
        # Update existing feature - merge files
        existing_files = set(existing_feature.get("files", []))
        new_files = set(changed_files)

        added_files = new_files - existing_files
        already_present = new_files & existing_files
        merged_files = existing_files | new_files

        log_info(f"Updating existing feature '{feature_name}'")
        log_info(f"  Current files: {len(existing_files)}")
        log_info(f"  Files from commit: {len(new_files)}")

        if added_files:
            log_success(f"  Adding {len(added_files)} new file(s):")
            for f in sorted(added_files)[:10]:
                log_info(f"    + {f}")
            if len(added_files) > 10:
                log_info(f"    ... and {len(added_files) - 10} more")

        if already_present:
            log_warning(f"  Skipping {len(already_present)} file(s) already in feature")

        features["features"][feature_name]["files"] = sorted(merged_files)
        # Update description if provided (allows updating description)
        features["features"][feature_name]["description"] = description

    else:
        # Create new feature
        log_info(f"Creating new feature '{feature_name}'")
        log_info(f"  Files from commit: {len(changed_files)}")

        features["features"][feature_name] = {
            "description": description,
            "files": sorted(changed_files),
        }

    # Save to file
    with open(features_file, "w") as f:
        yaml.safe_dump(features, f, sort_keys=False, default_flow_style=False)

    total_files = len(features["features"][feature_name]["files"])
    if existing_feature:
        log_success(f"✓ Updated feature '{feature_name}' - now has {total_files} files")
    else:
        log_success(f"✓ Created feature '{feature_name}' with {total_files} files")

    return True, ""


# Keep old function name for backwards compatibility but mark deprecated
def add_feature(ctx: Context, feature_name: str, commit: str, description: Optional[str] = None) -> bool:
    """Deprecated: Use add_or_update_feature instead."""
    if description is None:
        log_error("Description is required and must have a valid prefix")
        return False
    success, error = add_or_update_feature(ctx, feature_name, commit, description)
    if not success:
        log_error(error)
    return success


def list_features(ctx: Context):
    """List all defined features"""
    features_file = ctx.get_features_yaml_path()
    if not features_file.exists():
        log_warning("No features.yaml found")
        return

    with open(features_file, "r") as f:
        content = yaml.safe_load(f)
        if not content or "features" not in content:
            log_warning("No features defined")
            return

    features = content["features"]
    log_info(f"Features ({len(features)}):")
    log_info("-" * 60)

    for name, config in features.items():
        file_count = len(config.get("files", []))
        description = config.get("description", "")
        log_info(f"  {name}: {file_count} files - {description}")


def show_feature(ctx: Context, feature_name: str):
    """Show details of a specific feature"""
    features_file = ctx.get_features_yaml_path()
    if not features_file.exists():
        log_error("No features.yaml found")
        return

    with open(features_file, "r") as f:
        content = yaml.safe_load(f)
        if not content or "features" not in content:
            log_error("No features defined")
            return

    features = content["features"]
    if feature_name not in features:
        log_error(f"Feature '{feature_name}' not found")
        log_info("Available features:")
        for name in features.keys():
            log_info(f"  - {name}")
        return

    feature = features[feature_name]
    log_info(f"Feature: {feature_name}")
    log_info("-" * 60)
    log_info(f"Description: {feature.get('description', '')}")
    log_info(f"Commit: {feature.get('commit', 'Unknown')}")
    log_info(f"Files ({len(feature.get('files', []))}):")
    for file_path in feature.get("files", []):
        log_info(f"  - {file_path}")


# CommandModule wrappers for dev CLI

class ListFeaturesModule(CommandModule):
    """List all defined features"""
    produces = []
    requires = []
    description = "List all defined features"

    def validate(self, ctx: Context) -> None:
        """No validation needed - will show warning if no features exist"""
        pass

    def execute(self, ctx: Context, **kwargs) -> None:
        list_features(ctx)


class ShowFeatureModule(CommandModule):
    """Show details of a specific feature"""
    produces = []
    requires = []
    description = "Show details of a specific feature"

    def validate(self, ctx: Context) -> None:
        """Validation happens in execute (feature existence check)"""
        pass

    def execute(self, ctx: Context, feature_name: str, **kwargs) -> None:
        show_feature(ctx, feature_name)


class AddUpdateFeatureModule(CommandModule):
    """Add or update a feature with files from a commit"""
    produces = []
    requires = []
    description = "Add or update a feature with files from a commit"

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
        name: str,
        commit: str,
        description: str,
        **kwargs,
    ) -> None:
        success, error = add_or_update_feature(ctx, name, commit, description)
        if not success:
            raise RuntimeError(error)


# Backwards compatibility alias
AddFeatureModule = AddUpdateFeatureModule


class ClassifyFeaturesModule(CommandModule):
    """Classify unclassified patch files into features"""
    produces = []
    requires = []
    description = "Classify unclassified patch files into features"

    def validate(self, ctx: Context) -> None:
        """Validate patches directory exists"""
        patches_dir = ctx.get_patches_dir()
        if not patches_dir.exists():
            raise ValidationError(f"Patches directory not found: {patches_dir}")

    def execute(self, ctx: Context, **kwargs) -> None:
        from .select import classify_files, get_unclassified_files

        # Show summary first
        unclassified = get_unclassified_files(ctx)
        if not unclassified:
            log_success("All patch files are already classified!")
            return

        log_info(f"Found {len(unclassified)} unclassified patch file(s)")
        log_info("")

        # Run classification
        classified, skipped = classify_files(ctx)