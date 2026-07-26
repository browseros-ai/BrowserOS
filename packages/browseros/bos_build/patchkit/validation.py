"""
Feature validation utilities.

Shared validation functions for feature names and descriptions.
"""

import re
from typing import Tuple


# Valid description prefixes (conventional commits style)
VALID_PREFIXES = ("feat:", "fix:", "build:", "chore:", "series:")


def validate_description(description: str) -> Tuple[bool, str]:
    """Validate description has required prefix.

    Returns:
        Tuple of (is_valid, error_message)
    """
    description = description.strip()
    if not description:
        return False, "Description cannot be empty"

    if not any(description.startswith(prefix) for prefix in VALID_PREFIXES):
        return False, f"Description must start with one of: {', '.join(VALID_PREFIXES)}"

    return True, ""


def validate_feature_name(name: str) -> Tuple[bool, str]:
    """Validate feature name format.

    Feature names should be lowercase kebab-case identifiers.

    Returns:
        Tuple of (is_valid, error_message)
    """
    if not name:
        return False, "Feature name cannot be empty"

    if " " in name:
        return False, "Feature name cannot contain spaces (use hyphens instead)"

    if ":" in name:
        return False, "Feature name cannot contain ':' (did you pass a description as the name?)"

    if name != name.lower():
        return False, f"Feature name must be lowercase (got '{name}', use '{name.lower()}')"

    # Check for valid characters (alphanumeric, hyphens, underscores)
    if not re.match(r'^[a-z0-9][a-z0-9_-]*$', name):
        return False, "Feature name must start with a letter/number and contain only lowercase letters, numbers, hyphens, and underscores"

    return True, ""
