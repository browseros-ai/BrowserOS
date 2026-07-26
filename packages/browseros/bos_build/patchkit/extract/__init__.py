"""
Extract module - Extract patches from git commits.

Provides commands for extracting patches:
- extract_commit: Extract patches from a single commit
- extract_range: Extract patches from a range of commits
- extract_patch: Extract patch for a single file
"""

from .extract_commit import extract_single_commit, ExtractCommitModule
from .extract_range import (
    extract_commit_range,
    extract_commits_individually,
    ExtractRangeModule,
)
from .extract_patch import extract_single_file_patch

__all__ = [
    "extract_single_commit",
    "ExtractCommitModule",
    "extract_commit_range",
    "extract_commits_individually",
    "ExtractRangeModule",
    "extract_single_file_patch",
]
