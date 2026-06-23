#!/usr/bin/env python3
"""Pipeline validation for BrowserOS build system"""

from typing import Dict, List, Type
from .module import CommandModule
from .utils import log_error, log_info


def validate_pipeline(pipeline: List[str], available_modules: Dict[str, Type[CommandModule]]) -> None:
    """Validate that all modules in pipeline exist in available_modules
    
    Raises SystemExit if validation fails
    """
    invalid_modules = []
    
    for module_name in pipeline:
        if module_name not in available_modules:
            invalid_modules.append(module_name)
    
    if invalid_modules:
        log_error("Invalid module names in pipeline:")
        for module_name in invalid_modules:
            log_error(f"  - {module_name}")
        
        log_error("\nAvailable modules:")
        for module_name in sorted(available_modules.keys()):
            module_class = available_modules[module_name]
            log_info(f"  - {module_name}: {module_class.description}")
        
        raise SystemExit(1)


def show_available_modules(available_modules: Dict[str, Type[CommandModule]]) -> None:
    """Display all available modules with descriptions, grouped by category"""

    # Group modules by prefix
    groups = {
        "Setup & Environment": ["clean", "git_setup", "sparkle_setup", "winsparkle_setup", "configure"],
        "Patches & Resources": ["patches", "chromium_replace", "string_replaces", "resources"],
        "Build": ["compile"],
        "Code Signing": ["sign_macos", "sign_windows", "sign_linux"],
        "Packaging": ["package_macos", "package_windows", "package_linux"],
        "Upload": ["upload"],
    }

    log_info("\n" + "=" * 70)
    log_info("Available Build Modules")
    log_info("=" * 70)

    for group_name, module_names in groups.items():
        # Only show group if it has modules
        group_modules = [m for m in module_names if m in available_modules]
        if not group_modules:
            continue

        log_info(f"\n{group_name}:")
        log_info("-" * 70)

        for module_name in group_modules:
            module_class = available_modules[module_name]
            log_info(f"  {module_name:20} {module_class.description}")

    # Show any modules not in groups (for extensibility)
    all_grouped = set(m for group in groups.values() for m in group)
    ungrouped = sorted(set(available_modules.keys()) - all_grouped)

    if ungrouped:
        log_info("\nOther:")
        log_info("-" * 70)
        for module_name in ungrouped:
            module_class = available_modules[module_name]
            log_info(f"  {module_name:20} {module_class.description}")

    log_info("\n" + "=" * 70)
    log_info("Example Usage:")
    log_info("=" * 70)
    log_info("  browseros build --modules clean,git_setup,configure,compile")
    log_info("  browseros build --modules compile,sign_macos,package_macos")
    log_info("  browseros build --config release.yaml")
    log_info("=" * 70 + "\n")
