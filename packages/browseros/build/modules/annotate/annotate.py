"""
Annotate - Create git commits organized by features from features.yaml

For each feature, checks which files have modifications and creates a commit
with the feature name and description.
"""

import yaml
from pathlib import Path
from typing import List, Tuple, Optional, Dict

from ..apply.utils import run_git_command
from ...common.context import Context
from ...common.module import CommandModule, ValidationError
from ...common.utils import log_info, log_error, log_success, log_warning


def load_features(features_file: Path) -> Dict:
    """Load features from YAML file."""
    try:
        with open(features_file, "r") as f:
            data = yaml.safe_load(f)
            return data.get("features", {})
    except Exception as e:
        log_error(f"Failed to load features file: {e}")
        return {}


def get_modified_files(chromium_src: Path, files: List[str]) -> List[str]:
    """Get list of files that have modifications or are untracked.

    Args:
        chromium_src: Chromium source directory
        files: List of file paths to check

    Returns:
        List of file paths that have modifications
    """
    modified = []

    for file_path in files:
        full_path = chromium_src / file_path

        if not full_path.exists():
            continue

        result = run_git_command(
            ["git", "status", "--porcelain", str(file_path)],
            cwd=chromium_src,
        )

        if result.returncode == 0 and result.stdout.strip():
            modified.append(file_path)

    return modified


def git_add_and_commit(
    chromium_src: Path, files: List[str], commit_message: str
) -> bool:
    """Add files and create commit.

    Args:
        chromium_src: Chromium source directory
        files: List of file paths to add
        commit_message: Commit message

    Returns:
        True if commit was created successfully
    """
    # Add all specified files
    for file_path in files:
        result = run_git_command(
            ["git", "add", str(file_path)],
            cwd=chromium_src,
        )
        if result.returncode != 0:
            log_error(f"Failed to add file: {file_path}")
            return False

    # Create commit
    result = run_git_command(
        ["git", "commit", "-m", commit_message],
        cwd=chromium_src,
    )

    if result.returncode != 0:
        stderr = result.stderr or ""
        if "nothing to commit" in stderr or "nothing added to commit" in stderr:
            return False
        log_error(f"Failed to commit: {stderr}")
        return False

    return True


def annotate_features(
    ctx: Context,
    feature_filter: Optional[str] = None,
) -> Tuple[int, int]:
    """Create commits for features with modified files.

    Iterates through features.yaml and creates a commit for each feature
    that has modified files in the working tree.

    Args:
        ctx: Build context
        feature_filter: If specified, only process this feature

    Returns:
        Tuple of (commits_created, features_skipped)
    """
    features_file = ctx.get_features_yaml_path()

    if not features_file.exists():
        log_error(f"Features file not found: {features_file}")
        return 0, 0

    features = load_features(features_file)
    if not features:
        log_error("No features found in features.yaml")
        return 0, 0

    # Filter to specific feature if requested
    if feature_filter:
        if feature_filter not in features:
            log_error(f"Feature '{feature_filter}' not found in features.yaml")
            return 0, 0
        features = {feature_filter: features[feature_filter]}

    log_info(f"📋 Processing {len(features)} feature(s)")
    log_info("=" * 60)

    commits_created = 0
    features_skipped = 0

    for feature_name, feature_data in features.items():
        description = feature_data.get("description", feature_name)
        files = feature_data.get("files", [])

        log_info(f"\n🔧 {feature_name}")
        log_info(f"   {description}")

        if not files:
            log_warning("   No files specified, skipping")
            features_skipped += 1
            continue

        # Find files with modifications
        modified_files = get_modified_files(ctx.chromium_src, files)

        if not modified_files:
            log_warning(f"   No modified files ({len(files)} files checked)")
            features_skipped += 1
            continue

        log_info(f"   Found {len(modified_files)} modified file(s)")

        # Create commit
        commit_message = f"{feature_name}: {description}"

        if git_add_and_commit(ctx.chromium_src, modified_files, commit_message):
            log_success(f"   ✓ Committed {len(modified_files)} file(s)")
            commits_created += 1
        else:
            log_warning("   No changes staged, skipping commit")
            features_skipped += 1

    return commits_created, features_skipped


def annotate_single_feature(
    ctx: Context,
    feature_name: str,
) -> bool:
    """Create a commit for a single feature.

    Args:
        ctx: Build context
        feature_name: Name of the feature to commit

    Returns:
        True if commit was created successfully
    """
    commits, _ = annotate_features(ctx, feature_filter=feature_name)
    return commits > 0


class AnnotateModule(CommandModule):
    """Create git commits organized by features from features.yaml"""

    produces = []
    requires = []
    description = "Create git commits organized by features"

    def validate(self, ctx: Context) -> None:
        """Validate git is available and chromium_src exists."""
        import shutil

        if not shutil.which("git"):
            raise ValidationError("Git is not available in PATH")
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

        # Check if it's a git repository
        git_dir = ctx.chromium_src / ".git"
        if not git_dir.exists():
            raise ValidationError(f"Not a git repository: {ctx.chromium_src}")

    def execute(
        self,
        ctx: Context,
        feature_name: Optional[str] = None,
        **kwargs,
    ) -> None:
        """Execute annotate.

        Args:
            ctx: Build context
            feature_name: If specified, only annotate this feature
        """
        log_info("🏗️  Annotate Features")
        log_info("=" * 60)
        log_info(f"📁 Chromium source: {ctx.chromium_src}")
        log_info(f"📄 Features file: {ctx.get_features_yaml_path()}")

        commits_created, features_skipped = annotate_features(
            ctx, feature_filter=feature_name
        )

        log_info("\n" + "=" * 60)
        if commits_created > 0:
            log_success(f"✓ Created {commits_created} commit(s)")
        else:
            log_info("No commits created (no modified files found)")

        if features_skipped > 0:
            log_info(f"  Skipped {features_skipped} feature(s) with no changes")
        log_info("=" * 60)
