#!/usr/bin/env python3
"""
Compilation modules for BrowserOS build system

This package contains different build strategies:
- standard: Single-architecture compilation
- universal: Merge per-arch builds into a macOS universal app
"""

from .standard import CompileModule, build_target
from .universal import MergeUniversalModule

__all__ = [
    'CompileModule',
    'MergeUniversalModule',
    'build_target',
]
