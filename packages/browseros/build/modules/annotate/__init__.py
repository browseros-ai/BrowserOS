"""
Annotate module - Create git commits organized by features.

Provides commands for creating feature-based commits:
- annotate_features: Create commits for all features with modified files
- annotate_single_feature: Create a commit for a specific feature
"""

from .annotate import annotate_features, annotate_single_feature, AnnotateModule

__all__ = [
    "annotate_features",
    "annotate_single_feature",
    "AnnotateModule",
]
