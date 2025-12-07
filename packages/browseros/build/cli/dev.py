#!/usr/bin/env python3
"""
Dev CLI - Chromium patch management tool

A git-like patch management system for maintaining patches against Chromium.
Enables extracting, applying, and managing patches across Chromium upgrades.
"""

import yaml
from pathlib import Path
from typing import Optional

import typer
from typer import Typer, Option, Argument

# Import from common and utils
from ..common.context import Context
from ..common.utils import log_info, log_error, log_success, log_warning


def create_build_context(chromium_src: Optional[Path] = None) -> Optional[Context]:
    """Create BuildContext for dev CLI operations"""
    try:
        if not chromium_src:
            log_error("Chromium source directory not specified")
            log_info(
                "Use --chromium-src option to specify the Chromium source directory"
            )
            return None

        if not chromium_src.exists():
            log_error(f"Chromium source directory does not exist: {chromium_src}")
            return None

        ctx = Context(
            chromium_src=chromium_src,
            architecture="",  # Not needed for patch operations
            build_type="debug",  # Not needed for patch operations
        )

        return ctx
    except Exception as e:
        log_error(f"Failed to create build context: {e}")
        return None


# Create the Typer app
app = Typer(
    name="dev",
    help="BrowserOS dev CLI",
    no_args_is_help=True,
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


# State class to hold global options
class State:
    def __init__(self):
        self.chromium_src: Optional[Path] = None
        self.verbose: bool = False
        self.quiet: bool = False


state = State()


@app.callback()
def main(
    chromium_src: Optional[Path] = Option(
        None,
        "--chromium-src",
        "-S",
        help="Path to Chromium source directory",
        exists=True,
    ),
    verbose: bool = Option(False, "--verbose", "-v", help="Enable verbose output"),
    quiet: bool = Option(False, "--quiet", "-q", help="Suppress non-essential output"),
):
    """
    Dev CLI - Chromium patch management tool

    This tool provides git-like commands for managing patches against Chromium:

    Extract patches from commits:
      browseros dev extract commit HEAD
      browseros dev extract range HEAD~5 HEAD

    Apply patches:
      browseros dev apply all
      browseros dev apply feature llm-chat

    Manage features:
      browseros dev feature list
      browseros dev feature add my-feature HEAD
      browseros dev feature show my-feature
    """
    state.chromium_src = chromium_src
    state.verbose = verbose
    state.quiet = quiet


@app.command()
def status():
    """Show dev CLI status"""
    log_info("Dev CLI Status")
    log_info("-" * 40)

    build_ctx = create_build_context(state.chromium_src)
    if build_ctx:
        log_success(f"Chromium source: {build_ctx.chromium_src}")

        # Check for patches directory
        patches_dir = build_ctx.root_dir / "chromium_patches"
        if patches_dir.exists():
            patch_count = len(list(patches_dir.rglob("*.patch")))
            log_info(f"Individual patches: {patch_count}")
        else:
            log_warning("No patches directory found")

        # Check for features.yaml
        features_file = build_ctx.root_dir / "features.yaml"
        if features_file.exists():
            with open(features_file) as f:
                features = yaml.safe_load(f)
                feature_count = len(features.get("features", {}))
                log_info(f"Features defined: {feature_count}")
        else:
            log_warning("No features.yaml found")
    else:
        log_error("Failed to create build context")


# Create sub-apps for extract, apply, and feature commands
extract_app = Typer(
    name="extract",
    help="Extract patches from commits",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)
apply_app = Typer(
    name="apply",
    help="Apply patches to Chromium",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)
feature_app = Typer(
    name="feature",
    help="Manage features",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)

# Add sub-apps to main app
app.add_typer(extract_app, name="extract")
app.add_typer(apply_app, name="apply")
app.add_typer(feature_app, name="feature")


# Extract commands
@extract_app.command(name="commit")
def extract_commit(
    commit: str = Argument(..., help="Git commit reference (e.g., HEAD)"),
    output: Optional[Path] = Option(None, "--output", "-o", help="Output directory"),
    interactive: bool = Option(
        True, "--interactive/--no-interactive", "-i/-n", help="Interactive mode"
    ),
    force: bool = Option(False, "--force", "-f", help="Overwrite existing patches"),
    include_binary: bool = Option(False, "--include-binary", help="Include binary files"),
    base: Optional[str] = Option(
        None, "--base", help="Extract full diff from base commit for files in COMMIT"
    ),
    feature: bool = Option(
        False, "--feature", help="Add extracted files to a feature in features.yaml"
    ),
):
    """Extract patches from a single commit"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.extract import ExtractCommitModule

    module = ExtractCommitModule()
    try:
        module.validate(ctx)
        module.execute(
            ctx,
            commit=commit,
            output=output,
            interactive=interactive,
            verbose=state.verbose,
            force=force,
            include_binary=include_binary,
            base=base,
            feature=feature,
        )
    except Exception as e:
        log_error(f"Failed to extract commit: {e}")
        raise typer.Exit(1)


@extract_app.command(name="patch")
def extract_patch_cmd(
    chromium_path: str = Argument(..., help="Chromium file path (e.g., chrome/common/foo.h)"),
    base: str = Option(..., "--base", "-b", help="Base commit to diff against"),
    force: bool = Option(False, "--force", "-f", help="Overwrite existing patch without prompting"),
    feature: bool = Option(
        False, "--feature", help="Add extracted file to a feature in features.yaml"
    ),
):
    """Extract patch for a specific file"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.extract import extract_single_file_patch

    success, error = extract_single_file_patch(ctx, chromium_path, base, force)
    if not success:
        log_error(error or "Unknown error")
        raise typer.Exit(1)
    log_success(f"Successfully extracted patch for: {chromium_path}")

    # Handle --feature flag
    if feature:
        from ..modules.feature import prompt_feature_selection, add_files_to_feature

        result = prompt_feature_selection(ctx, base[:12], None)
        if result is None:
            log_warning("Skipped adding file to feature")
        else:
            feature_name, description = result
            add_files_to_feature(ctx, feature_name, description, [chromium_path])


@extract_app.command(name="range")
def extract_range(
    start: str = Argument(..., help="Start commit (exclusive)"),
    end: str = Argument(..., help="End commit (inclusive)"),
    output: Optional[Path] = Option(None, "--output", "-o", help="Output directory"),
    interactive: bool = Option(
        True, "--interactive/--no-interactive", "-i/-n", help="Interactive mode"
    ),
    force: bool = Option(False, "--force", "-f", help="Overwrite existing patches"),
    include_binary: bool = Option(False, "--include-binary", help="Include binary files"),
    squash: bool = Option(False, "--squash", help="Squash all commits into single patches"),
    base: Optional[str] = Option(
        None,
        "--base",
        help="Use different base for diff (full diff from base for files in range)",
    ),
    feature: bool = Option(
        False, "--feature", help="Add extracted files to a feature in features.yaml"
    ),
):
    """Extract patches from a range of commits"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.extract import ExtractRangeModule

    module = ExtractRangeModule()
    try:
        module.validate(ctx)
        module.execute(
            ctx,
            start=start,
            end=end,
            output=output,
            interactive=interactive,
            verbose=state.verbose,
            force=force,
            include_binary=include_binary,
            squash=squash,
            base=base,
            feature=feature,
        )
    except Exception as e:
        log_error(f"Failed to extract range: {e}")
        raise typer.Exit(1)


# Apply commands
@apply_app.command(name="all")
def apply_all(
    interactive: bool = Option(
        True, "--interactive/--no-interactive", "-i/-n", help="Interactive mode"
    ),
    reset_to: Optional[str] = Option(
        None, "--reset-to", "-r", help="Reset files to this commit before applying patches"
    ),
    annotate: bool = Option(
        False, "--annotate", "-a", help="Create git commits per feature after applying"
    ),
):
    """Apply all patches from chromium_patches/"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.apply import ApplyAllModule

    module = ApplyAllModule()
    try:
        module.validate(ctx)
        module.execute(ctx, interactive=interactive, reset_to=reset_to, annotate=annotate)
    except Exception as e:
        log_error(f"Failed to apply patches: {e}")
        raise typer.Exit(1)


@apply_app.command(name="feature")
def apply_feature(
    feature_name: str = Argument(..., help="Feature name to apply"),
    interactive: bool = Option(
        True, "--interactive/--no-interactive", "-i/-n", help="Interactive mode"
    ),
    reset_to: Optional[str] = Option(
        None, "--reset-to", "-r", help="Reset files to this commit before applying patches"
    ),
    annotate: bool = Option(
        False, "--annotate", "-a", help="Create git commit for this feature after applying"
    ),
):
    """Apply patches for a specific feature"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.apply import ApplyFeatureModule

    module = ApplyFeatureModule()
    try:
        module.validate(ctx)
        module.execute(
            ctx, feature_name=feature_name, interactive=interactive, reset_to=reset_to, annotate=annotate
        )
    except Exception as e:
        log_error(f"Failed to apply feature: {e}")
        raise typer.Exit(1)


@apply_app.command(name="patch")
def apply_patch_cmd(
    chromium_path: str = Argument(..., help="Chromium file path (e.g., chrome/common/foo.h)"),
    reset_to: Optional[str] = Option(
        None, "--reset-to", "-r", help="Reset file to this commit before applying patch"
    ),
    dry_run: bool = Option(False, "--dry-run", help="Test without applying"),
):
    """Apply patch for a specific file"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.apply import apply_single_file_patch

    success, error = apply_single_file_patch(ctx, chromium_path, reset_to, dry_run)
    if not success:
        log_error(error or "Unknown error")
        raise typer.Exit(1)
    log_success(f"Successfully applied patch for: {chromium_path}")


# Feature commands
@feature_app.command(name="list")
def feature_list():
    """List all defined features"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.feature import ListFeaturesModule

    module = ListFeaturesModule()
    try:
        module.validate(ctx)
        module.execute(ctx)
    except Exception as e:
        log_error(f"Failed to list features: {e}")
        raise typer.Exit(1)


@feature_app.command(name="show")
def feature_show(
    feature_name: str = Argument(..., help="Feature name to show"),
):
    """Show details of a specific feature"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.feature import ShowFeatureModule

    module = ShowFeatureModule()
    try:
        module.validate(ctx)
        module.execute(ctx, feature_name=feature_name)
    except Exception as e:
        log_error(f"Failed to show feature: {e}")
        raise typer.Exit(1)


@feature_app.command(name="add")
def feature_add(
    feature_name: str = Argument(..., help="Feature name to add"),
    commit: str = Argument(..., help="Git commit reference"),
    description: Optional[str] = Option(
        None, "--description", "-d", help="Feature description"
    ),
):
    """Add a new feature from a commit"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.feature import AddFeatureModule

    module = AddFeatureModule()
    try:
        module.validate(ctx)
        module.execute(
            ctx, feature_name=feature_name, commit=commit, description=description
        )
    except Exception as e:
        log_error(f"Failed to add feature: {e}")
        raise typer.Exit(1)


@feature_app.command(name="classify")
def feature_classify():
    """Classify unclassified patch files into features

    Lists all patches in chromium_patches/ that are not in any feature,
    then prompts one-by-one to assign each to a feature.

    Examples:
        browseros dev feature classify
    """
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.feature import ClassifyFeaturesModule

    module = ClassifyFeaturesModule()
    try:
        module.validate(ctx)
        module.execute(ctx)
    except Exception as e:
        log_error(f"Failed to classify features: {e}")
        raise typer.Exit(1)


# Annotate command
@app.command(name="annotate")
def annotate_cmd(
    feature_name: Optional[str] = Argument(
        None, help="Optional: specific feature to annotate (default: all features)"
    ),
):
    """Create git commits organized by features from features.yaml

    For each feature with modified files, creates a commit with the format:
    "{feature_name}: {description}"

    Examples:
        browseros dev annotate -S /path/to/chromium
        browseros dev annotate llm-chat -S /path/to/chromium
    """
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..modules.annotate import AnnotateModule

    module = AnnotateModule()
    try:
        module.validate(ctx)
        module.execute(ctx, feature_name=feature_name)
    except Exception as e:
        log_error(f"Failed to annotate: {e}")
        raise typer.Exit(1)


if __name__ == "__main__":
    app()
