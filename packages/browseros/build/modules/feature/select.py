"""
Feature selection utilities for interactive feature assignment.

Provides functions to prompt users to select or create features
and add files to them.
"""

import yaml
from pathlib import Path
from typing import List, Optional, Dict, Tuple, Set

from ...common.context import Context
from ...common.utils import log_info, log_success, log_warning, log_error
from .validation import validate_feature_name, validate_description, VALID_PREFIXES


def load_features_yaml(features_file: Path) -> Dict:
    """Load features from YAML file."""
    if not features_file.exists():
        return {"version": "1.0", "features": {}}

    with open(features_file, "r") as f:
        content = yaml.safe_load(f)
        if not content:
            return {"version": "1.0", "features": {}}
        return content


def save_features_yaml(features_file: Path, data: Dict) -> None:
    """Save features to YAML file."""
    with open(features_file, "w") as f:
        yaml.safe_dump(data, f, sort_keys=False, default_flow_style=False)


def prompt_feature_selection(
    ctx: Context,
    commit_hash: Optional[str] = None,
    commit_message: Optional[str] = None,
) -> Optional[Tuple[str, str]]:
    """Prompt user to select an existing feature or create a new one.

    Args:
        ctx: Build context
        commit_hash: Optional commit hash for display
        commit_message: Optional commit message for display/defaults

    Returns:
        Tuple of (feature_name, description) or None if cancelled
    """
    features_file = ctx.get_features_yaml_path()
    data = load_features_yaml(features_file)
    features = data.get("features", {})

    # Display commit info if available
    if commit_hash or commit_message:
        log_info("")
        log_info("=" * 60)
        if commit_hash:
            log_info(f"Commit: {commit_hash[:12]}")
        if commit_message:
            log_info(f"Message: {commit_message}")
        log_info("=" * 60)

    # Display numbered list of features
    log_info("")
    log_info("Select a feature to add files to:")
    log_info("-" * 40)

    feature_list = list(features.keys())
    for i, name in enumerate(feature_list, 1):
        desc = features[name].get("description", name)
        file_count = len(features[name].get("files", []))
        log_info(f"  {i}) {desc} ({file_count} files)")

    # Add "new feature" option
    new_option = len(feature_list) + 1
    log_info(f"  {new_option}) [Add new feature]")
    log_info("")

    # Get user selection
    while True:
        try:
            choice = input(f"Enter choice (1-{new_option}): ").strip()
            if not choice:
                log_warning("Cancelled")
                return None

            choice_num = int(choice)
            if choice_num < 1 or choice_num > new_option:
                log_warning(f"Please enter a number between 1 and {new_option}")
                continue

            break
        except ValueError:
            log_warning("Please enter a valid number")
            continue
        except (KeyboardInterrupt, EOFError):
            log_warning("\nCancelled")
            return None

    # Handle selection
    if choice_num == new_option:
        # Create new feature
        return prompt_new_feature(commit_message)
    else:
        # Selected existing feature
        feature_name = feature_list[choice_num - 1]
        description = features[feature_name].get("description", "")
        return (feature_name, description)


def prompt_new_feature(default_description: Optional[str] = None) -> Optional[Tuple[str, str]]:
    """Prompt user to create a new feature.

    Args:
        default_description: Optional default description (e.g., from commit message)

    Returns:
        Tuple of (feature_name, description) or None if cancelled
    """
    log_info("")
    log_info("Creating new feature:")
    log_info("-" * 40)
    log_info(f"  Valid prefixes: {', '.join(VALID_PREFIXES)}")
    log_info("")

    try:
        # Get and validate feature name
        while True:
            feature_name = input("Feature name (kebab-case): ").strip()
            if not feature_name:
                log_warning("Cancelled - no feature name provided")
                return None

            # Sanitize feature name (lowercase, hyphens instead of spaces)
            feature_name = feature_name.lower().replace(" ", "-")

            # Validate
            valid, error = validate_feature_name(feature_name)
            if valid:
                break
            log_warning(f"Invalid name: {error}")

        # Get and validate description
        while True:
            if default_description:
                # Check if default already has valid prefix
                valid, _ = validate_description(default_description)
                if valid:
                    desc_prompt = f"Description [{default_description}]: "
                else:
                    desc_prompt = f"Description (e.g., feat: {default_description}): "
            else:
                desc_prompt = "Description (e.g., feat: Add feature): "

            description = input(desc_prompt).strip()
            if not description and default_description:
                # Check if default is valid
                valid, _ = validate_description(default_description)
                if valid:
                    description = default_description
                else:
                    log_warning(f"Default description needs prefix. Valid: {', '.join(VALID_PREFIXES)}")
                    continue

            if not description:
                log_warning(f"Description required. Must start with: {', '.join(VALID_PREFIXES)}")
                continue

            # Validate
            valid, error = validate_description(description)
            if valid:
                break
            log_warning(f"Invalid description: {error}")

        return (feature_name, description)

    except (KeyboardInterrupt, EOFError):
        log_warning("\nCancelled")
        return None


def add_files_to_feature(
    ctx: Context,
    feature_name: str,
    description: str,
    files: List[str],
) -> int:
    """Add files to a feature in features.yaml, avoiding duplicates.

    Args:
        ctx: Build context
        feature_name: Name of the feature
        description: Feature description
        files: List of file paths to add

    Returns:
        Number of new files added (excludes duplicates)
    """
    features_file = ctx.get_features_yaml_path()
    data = load_features_yaml(features_file)

    if "features" not in data:
        data["features"] = {}

    features = data["features"]

    # Get or create feature entry
    if feature_name in features:
        existing_files = set(features[feature_name].get("files", []))
        # Keep existing description if present
        if not features[feature_name].get("description"):
            features[feature_name]["description"] = description
    else:
        existing_files = set()
        features[feature_name] = {
            "description": description,
            "files": [],
        }

    # Add new files, avoiding duplicates
    new_files = []
    duplicate_files = []

    for file_path in files:
        if file_path in existing_files:
            duplicate_files.append(file_path)
        else:
            new_files.append(file_path)
            existing_files.add(file_path)

    # Update feature with merged file list
    features[feature_name]["files"] = sorted(existing_files)

    # Save to file
    save_features_yaml(features_file, data)

    # Log results
    if new_files:
        log_success(f"Added {len(new_files)} file(s) to feature '{feature_name}'")
        for f in new_files[:5]:
            log_info(f"  + {f}")
        if len(new_files) > 5:
            log_info(f"  ... and {len(new_files) - 5} more")

    if duplicate_files:
        log_warning(f"Skipped {len(duplicate_files)} duplicate file(s)")
        for f in duplicate_files[:3]:
            log_info(f"  ~ {f}")
        if len(duplicate_files) > 3:
            log_info(f"  ... and {len(duplicate_files) - 3} more")

    return len(new_files)


def get_all_patch_files(ctx: Context) -> List[str]:
    """Get all patch files from chromium_patches/ directory.

    Returns:
        List of file paths (relative to chromium_patches/)
    """
    patches_dir = ctx.get_patches_dir()
    if not patches_dir.exists():
        return []

    patch_files = []
    for patch_path in patches_dir.rglob("*"):
        if patch_path.is_file():
            # Get relative path from patches_dir
            rel_path = str(patch_path.relative_to(patches_dir))
            patch_files.append(rel_path)

    return sorted(patch_files)


def get_all_classified_files(ctx: Context) -> Set[str]:
    """Get all files that are already classified in features.yaml.

    Returns:
        Set of file paths
    """
    features_file = ctx.get_features_yaml_path()
    data = load_features_yaml(features_file)
    features = data.get("features", {})

    classified = set()
    for feature_data in features.values():
        files = feature_data.get("files", [])
        classified.update(files)

    return classified


def get_unclassified_files(ctx: Context) -> List[str]:
    """Get list of patch files not in any feature.

    Returns:
        List of unclassified file paths
    """
    all_patches = set(get_all_patch_files(ctx))
    classified = get_all_classified_files(ctx)
    unclassified = all_patches - classified
    return sorted(unclassified)


def classify_files(ctx: Context) -> Tuple[int, int]:
    """Interactively classify unclassified patch files into features.

    Goes through each unclassified file one-by-one and prompts user
    to select a feature or create a new one.

    Returns:
        Tuple of (files_classified, files_skipped)
    """
    unclassified = get_unclassified_files(ctx)

    if not unclassified:
        log_success("All patch files are already classified!")
        return 0, 0

    log_info(f"Found {len(unclassified)} unclassified file(s)")
    log_info("=" * 60)
    log_info("Press Ctrl+C to stop at any time")
    log_info("")

    classified_count = 0
    skipped_count = 0

    for i, file_path in enumerate(unclassified, 1):
        log_info(f"\n[{i}/{len(unclassified)}] {file_path}")
        log_info("-" * 40)

        try:
            # Prompt for feature selection (no commit context for classify)
            result = prompt_feature_selection_for_file(ctx, file_path)

            if result is None:
                log_warning("Skipped")
                skipped_count += 1
                continue

            feature_name, description = result
            add_files_to_feature(ctx, feature_name, description, [file_path])
            classified_count += 1

        except KeyboardInterrupt:
            log_info("\n\nStopped by user")
            break

    log_info("")
    log_info("=" * 60)
    log_success(f"Classified {classified_count} file(s)")
    if skipped_count > 0:
        log_info(f"Skipped {skipped_count} file(s)")
    remaining = len(unclassified) - classified_count - skipped_count
    if remaining > 0:
        log_info(f"Remaining: {remaining} file(s)")

    return classified_count, skipped_count


def prompt_feature_selection_for_file(
    ctx: Context,
    file_path: str,
) -> Optional[Tuple[str, str]]:
    """Prompt user to select a feature for a single file.

    Simplified version of prompt_feature_selection for classify workflow.

    Args:
        ctx: Build context
        file_path: The file being classified

    Returns:
        Tuple of (feature_name, description) or None if skipped
    """
    features_file = ctx.get_features_yaml_path()
    data = load_features_yaml(features_file)
    features = data.get("features", {})

    if not features:
        log_info("No features defined yet. Create a new one:")
        return prompt_new_feature()

    # Display numbered list of features
    feature_list = list(features.keys())
    for i, name in enumerate(feature_list, 1):
        desc = features[name].get("description", name)
        file_count = len(features[name].get("files", []))
        log_info(f"  {i}) {desc} ({file_count} files)")

    # Add options
    new_option = len(feature_list) + 1
    skip_option = len(feature_list) + 2
    log_info(f"  {new_option}) [Add new feature]")
    log_info(f"  {skip_option}) [Skip this file]")

    # Get user selection
    while True:
        try:
            choice = input(f"Choice (1-{skip_option}): ").strip()
            if not choice:
                return None

            choice_num = int(choice)
            if choice_num < 1 or choice_num > skip_option:
                log_warning(f"Please enter 1-{skip_option}")
                continue

            break
        except ValueError:
            log_warning("Enter a valid number")
            continue
        except (KeyboardInterrupt, EOFError):
            raise KeyboardInterrupt

    # Handle selection
    if choice_num == skip_option:
        return None
    elif choice_num == new_option:
        return prompt_new_feature()
    else:
        feature_name = feature_list[choice_num - 1]
        description = features[feature_name].get("description", "")
        return (feature_name, description)
