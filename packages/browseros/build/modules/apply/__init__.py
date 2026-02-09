"""
Apply module - Apply patches to Chromium source.

Provides commands for applying patches:
- apply_all: Apply all patches from patches directory
- apply_feature: Apply patches for a specific feature
- apply_patch: Apply patch for a single file
- apply_changed: Apply patches changed in specific commits
"""

from .apply_all import apply_all_patches, ApplyAllModule
from .apply_feature import apply_feature_patches, ApplyFeatureModule
from .apply_patch import apply_single_file_patch
from .apply_changed import apply_changed_patches, ApplyChangedModule
from .apply_force import apply_all_force, ApplyForceModule

__all__ = [
    "apply_all_patches",
    "ApplyAllModule",
    "apply_feature_patches",
    "ApplyFeatureModule",
    "apply_single_file_patch",
    "apply_changed_patches",
    "ApplyChangedModule",
    "apply_all_force",
    "ApplyForceModule",
]
