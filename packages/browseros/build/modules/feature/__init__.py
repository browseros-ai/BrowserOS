"""
Feature module - Manage feature-to-file mappings.

Provides commands for managing features:
- add_feature: Add files from a commit to a feature
- list_features: List all defined features
- show_feature: Show details of a specific feature
- prompt_feature_selection: Interactive feature selection for extract commands
- add_files_to_feature: Add files to a feature (with duplicate handling)
- classify_files: Classify unclassified patch files into features
"""

from .feature import (
    add_feature,
    AddFeatureModule,
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
    "AddFeatureModule",
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
