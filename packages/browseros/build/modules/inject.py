#!/usr/bin/env python3
"""
Version injection module for manifest.json files
Injects blockbrowser browser version into extension manifests
"""

import json
from pathlib import Path
from typing import List, Dict, Any
from context import BuildContext
from utils import log_info, log_error, log_success, join_paths


def inject_version(ctx: BuildContext) -> bool:
    """Inject browser version into manifest.json files"""
    log_info("\n💉 Injecting browser version into extension manifests...")

    # Hardcoded paths to manifest files
    manifest_paths = [
        join_paths(
            ctx.root_dir, "resources", "files", "ai_side_panel", "manifest.json"
        ),
        join_paths(ctx.root_dir, "resources", "files", "bug_reporter", "manifest.json"),
    ]

    success = True
    for manifest_path in manifest_paths:
        if not inject_version_to_manifest(
            manifest_path,
            ctx.get_blockbrowser_chromium_version(),
            ctx.get_blockbrowser_version(),
        ):
            success = False

    if success:
        log_success("Browser version injected into all manifests")
    else:
        log_error("Failed to inject version into some manifests")

    return success


def inject_version_to_manifest(
    manifest_path: Path, browser_version: str, blockbrowser_version: str
) -> bool:
    """Inject browser version and increment version into a single manifest.json file"""
    try:
        if not manifest_path.exists():
            log_error(f"Manifest not found: {manifest_path}")
            return False

        # Read existing manifest
        with open(manifest_path, "r", encoding="utf-8") as f:
            manifest_data = json.load(f)

        # Set version to NXTSCAPE_VERSION formatted as X.0.0.0
        if "version" in manifest_data:
            current_version = manifest_data["version"]
            # Format version as X.0.0.0
            formatted_version = f"{blockbrowser_version}.0.0.0"
            manifest_data["version"] = formatted_version
            log_info(
                f"  Manifest version updated: {current_version} → {formatted_version}"
            )

        # Add browser_version field
        manifest_data["browser_version"] = browser_version

        # Write back with proper formatting
        with open(manifest_path, "w", encoding="utf-8") as f:
            json.dump(manifest_data, f, indent=2, ensure_ascii=False)
            f.write("\n")  # Add trailing newline

        # Validate the written JSON
        if validate_json_file(manifest_path):
            log_success(f"✓ Updated: {manifest_path.name}")
            return True
        else:
            log_error(f"✗ Invalid JSON after injection: {manifest_path.name}")
            return False

    except json.JSONDecodeError as e:
        log_error(f"Failed to parse JSON in {manifest_path}: {e}")
        return False
    except Exception as e:
        log_error(f"Failed to inject version into {manifest_path}: {e}")
        return False


def increment_version(version: str) -> str:
    """Increment version string by 1 in the last component"""
    parts = version.split(".")
    if not parts:
        return "0.0.1"

    # Try to increment the last numeric part
    for i in range(len(parts) - 1, -1, -1):
        try:
            # Convert to int, increment, and convert back
            incremented = int(parts[i]) + 1
            parts[i] = str(incremented)
            return ".".join(parts)
        except ValueError:
            # If this part is not numeric, continue to the previous part
            continue

    # If no numeric part found, append .1
    return version + ".1"


def validate_json_file(file_path: Path) -> bool:
    """Validate that a file contains valid JSON"""
    try:
        with open(file_path, "r", encoding="utf-8") as f:
            json.load(f)
        return True
    except json.JSONDecodeError:
        return False
    except Exception:
        return False


def remove_browser_version(ctx: BuildContext) -> bool:
    """Remove browser version from manifest.json files (for cleanup)"""
    log_info("\n🧹 Removing browser version from extension manifests...")

    # Hardcoded paths to manifest files
    manifest_paths = [
        join_paths(
            ctx.root_dir, "resources", "files", "ai_side_panel", "manifest.json"
        ),
        join_paths(ctx.root_dir, "resources", "files", "bug_reporter", "manifest.json"),
    ]

    success = True
    for manifest_path in manifest_paths:
        if not remove_version_from_manifest(manifest_path):
            success = False

    return success


def remove_version_from_manifest(manifest_path: Path) -> bool:
    """Remove browser version from a single manifest.json file"""
    try:
        if not manifest_path.exists():
            return True  # Nothing to remove

        # Read existing manifest
        with open(manifest_path, "r", encoding="utf-8") as f:
            manifest_data = json.load(f)

        # Remove browser_version field if it exists
        if "browser_version" in manifest_data:
            del manifest_data["browser_version"]

            # Write back with proper formatting
            with open(manifest_path, "w", encoding="utf-8") as f:
                json.dump(manifest_data, f, indent=2, ensure_ascii=False)
                f.write("\n")  # Add trailing newline

            log_info(f"Removed browser_version from: {manifest_path.name}")

        return True

    except Exception as e:
        log_error(f"Failed to remove version from {manifest_path}: {e}")
        return False


def get_manifest_version(manifest_path: Path) -> str:
    """Get the current version from a manifest.json file"""
    try:
        with open(manifest_path, "r", encoding="utf-8") as f:
            manifest_data = json.load(f)
            return manifest_data.get("version", "unknown")
    except Exception:
        return "unknown"
