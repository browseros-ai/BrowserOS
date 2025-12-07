"""
Feature selection utilities for interactive feature assignment.

Provides functions to prompt users to select or create features
and add files to them.
"""

import yaml
from pathlib import Path
from typing import List, Optional, Dict, Tuple

from ...common.context import Context
from ...common.utils import log_info, log_success, log_warning


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

    try:
        # Get feature name
        feature_name = input("Feature name: ").strip()
        if not feature_name:
            log_warning("Cancelled - no feature name provided")
            return None

        # Sanitize feature name (lowercase, hyphens instead of spaces)
        feature_name = feature_name.lower().replace(" ", "-")

        # Get description
        if default_description:
            desc_prompt = f"Description [{default_description}]: "
        else:
            desc_prompt = "Description: "

        description = input(desc_prompt).strip()
        if not description and default_description:
            description = default_description

        if not description:
            description = f"Feature: {feature_name}"

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
