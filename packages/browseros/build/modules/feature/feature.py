"""
Feature module - Manage feature-to-file mappings

Simple feature management with YAML persistence.
"""

import yaml
from typing import Dict, Optional
from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ..extract.utils import get_commit_changed_files
from ...common.utils import log_info, log_error, log_success, log_warning


def add_feature(ctx: Context, feature_name: str, commit: str, description: Optional[str] = None) -> bool:
    """Add files from a commit to a feature

    Examples:
      dev feature add my-feature HEAD
      dev feature add llm-chat HEAD~3 --description "LLM chat integration"
    """
    features_file = ctx.get_features_yaml_path()

    # Get changed files from commit
    changed_files = get_commit_changed_files(ctx, commit)
    if not changed_files:
        log_error(f"No changed files found in commit {commit}")
        return False

    # Load existing features
    features: Dict = {"features": {}}
    if features_file.exists():
        with open(features_file, "r") as f:
            content = yaml.safe_load(f)
            if content and "features" in content:
                features = content

    # Add or update feature
    features["features"][feature_name] = {
        "description": description or f"Feature: {feature_name}",
        "files": sorted(changed_files),
        "commit": commit,
    }

    # Save to file
    with open(features_file, "w") as f:
        yaml.safe_dump(features, f, sort_keys=False, default_flow_style=False)

    log_success(f"✓ Added feature '{feature_name}' with {len(changed_files)} files")
    return True


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


class AddFeatureModule(CommandModule):
    """Add files from a commit to a feature"""
    produces = []
    requires = []
    description = "Add files from a commit to a feature"

    def validate(self, ctx: Context) -> None:
        """Validate git is available"""
        import shutil
        if not shutil.which("git"):
            raise ValidationError("Git is not available in PATH")
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

    def execute(self, ctx: Context, feature_name: str, commit: str, description: Optional[str] = None, **kwargs) -> None:
        success = add_feature(ctx, feature_name, commit, description)
        if not success:
            raise RuntimeError(f"Failed to add feature '{feature_name}'")


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