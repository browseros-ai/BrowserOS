"""
Feature module - Manage feature-to-file mappings.

Provides commands for managing features:
- add_or_update_feature: Add or update a feature with files from a commit
- list_features: List all defined features
- show_feature: Show details of a specific feature
- prompt_feature_selection: Interactive feature selection for extract commands
- add_files_to_feature: Add files to a feature (with duplicate handling)
- classify_files: Classify unclassified patch files into features
- validate_description: Validate description has required prefix
- validate_feature_name: Validate feature name format
"""

from .validation import (
    validate_description,
    validate_feature_name,
    VALID_PREFIXES,
)
from .feature import (
    add_feature,
    add_or_update_feature,
    AddFeatureModule,
    AddUpdateFeatureModule,
    list_features,
    ListFeaturesModule,
    show_feature,
    ShowFeatureModule,
    ClassifyFeaturesModule,
)
from .select import (
    prompt_feature_selection,
    add_files_to_feature,
    classify_files,
    get_unclassified_files,
)

__all__ = [
    "add_feature",
    "add_or_update_feature",
    "validate_description",
    "validate_feature_name",
    "VALID_PREFIXES",
    "AddFeatureModule",
    "AddUpdateFeatureModule",
    "list_features",
    "ListFeaturesModule",
    "show_feature",
    "ShowFeatureModule",
    "ClassifyFeaturesModule",
    "prompt_feature_selection",
    "add_files_to_feature",
    "classify_files",
    "get_unclassified_files",
]
