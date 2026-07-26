#!/usr/bin/env python3
"""Merge two architecture builds into one universal .app via the vendored universalizer."""

import sys
import shutil
from pathlib import Path
from ...lib.utils import run_command, log_info, log_error, log_success


def merge_architectures(
    arch1_path: Path,
    arch2_path: Path,
    output_path: Path,
    universalizer_script: Path | None = None,
) -> bool:
    """
    Merge two architecture builds into a universal binary

    Args:
        arch1_path: Path to first architecture .app bundle
        arch2_path: Path to second architecture .app bundle
        output_path: Path where universal .app bundle should be created
        universalizer_script: Path to universalizer script (optional)

    Returns:
        True if successful, False otherwise
    """
    log_info("🔄 Merging architecture builds into universal binary...")

    # Validate input paths
    if not arch1_path.exists():
        log_error(f"Architecture 1 app not found: {arch1_path}")
        return False

    if not arch2_path.exists():
        log_error(f"Architecture 2 app not found: {arch2_path}")
        return False

    log_info(f"📱 Input 1: {arch1_path}")
    log_info(f"📱 Input 2: {arch2_path}")
    log_info(f"🎯 Output: {output_path}")

    # Find universalizer script
    if universalizer_script is None:
        # Try to find it in the same package module directory
        current_dir = Path(__file__).parent
        universalizer_script = current_dir / "universalizer_patched.py"

    if not universalizer_script.exists():
        log_error(f"Universalizer script not found: {universalizer_script}")
        return False

    # Create output directory if needed
    output_path.parent.mkdir(parents=True, exist_ok=True)

    # Remove existing output if present
    if output_path.exists():
        log_info(f"Removing existing output: {output_path}")
        shutil.rmtree(output_path)

    try:
        # Run universalizer
        cmd = [
            sys.executable,
            str(universalizer_script),
            str(arch1_path),
            str(arch2_path),
            str(output_path),
        ]

        log_info("Running universalizer...")
        log_info(f"Command: {' '.join(cmd)}")
        run_command(cmd)

        if output_path.exists():
            log_success(f"Universal binary created: {output_path}")
            return True
        else:
            log_error("Universal binary creation failed - output not found")
            return False

    except Exception as e:
        log_error(f"Failed to create universal binary: {e}")
        return False
