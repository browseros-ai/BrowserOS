#!/usr/bin/env python3
"""Pipeline validation for BrowserOS build system"""

from typing import Dict, List, Type
from .step import PHASES, Step
from ..lib.utils import log_error, log_info


def validate_pipeline(pipeline: List[str], available_modules: Dict[str, Type[Step]]) -> None:
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


def show_available_modules(available_modules: Dict[str, Type[Step]]) -> None:
    """Display all available modules with descriptions, grouped by phase"""

    log_info("\n" + "=" * 70)
    log_info("Available Build Modules")
    log_info("=" * 70)

    for phase in PHASES:
        phase_modules = [
            (name, cls)
            for name, cls in available_modules.items()
            if cls.phase == phase
        ]
        if not phase_modules:
            continue

        log_info(f"\n{phase.capitalize()}:")
        log_info("-" * 70)

        for module_name, module_class in phase_modules:
            extras = []
            if module_class.platforms is not None:
                extras.append("/".join(module_class.platforms))
            if module_class.optional:
                extras.append("optional")
            suffix = f"  [{', '.join(extras)}]" if extras else ""
            log_info(f"  {module_name:20} {module_class.description}{suffix}")

    ungrouped = sorted(
        name for name, cls in available_modules.items() if not cls.phase
    )
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
    log_info("  browseros build --preset release --product browseros --arch arm64")
    log_info("=" * 70 + "\n")
