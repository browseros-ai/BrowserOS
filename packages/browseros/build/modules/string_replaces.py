#!/usr/bin/env python3
"""
String replacement module for BlockBrowser build system
"""

import re
from pathlib import Path
from context import BuildContext
from utils import log_info, log_success, log_error, log_warning


# Strings we want to replace but that we also replace automatically
# for XTB files
branding_replacements = [
    (
        r"The Chromium Authors. All rights reserved.",
        r"The BlockBrowser Authors. All rights reserved.",
    ),
    (
        r"Google LLC. All rights reserved.",
        r"The BlockBrowser Authors. All rights reserved.",
    ),
    (r"The Chromium Authors", r"BlockBrowser Software Inc"),
    (r"Google Chrome", r"BlockBrowser"),
    (r"(Google)(?! Play)", r"BlockBrowser"),
    (r"Chromium", r"BlockBrowser"),
    (r"Chrome", r"BlockBrowser"),
]

# List of files to apply replacements to
target_files = [
    "chrome/app/chromium_strings.grd",
    "chrome/app/settings_chromium_strings.grdp",
]


def apply_string_replacements(ctx: BuildContext) -> bool:
    """Apply string replacements to specified files"""
    log_info("\n🔤 Applying string replacements...")

    success = True

    for file_path in target_files:
        full_path = ctx.chromium_src / file_path

        if not full_path.exists():
            log_warning(f"  ⚠️  File not found: {file_path}")
            continue

        log_info(f"  • Processing: {file_path}")

        try:
            # Read the file content
            with open(full_path, "r", encoding="utf-8") as f:
                content = f.read()

            original_content = content
            replacement_count = 0

            # Apply each replacement
            for pattern, replacement in branding_replacements:
                matches = len(re.findall(pattern, content))
                if matches > 0:
                    content = re.sub(pattern, replacement, content)
                    replacement_count += matches
                    log_info(f"    ✓ Replaced {matches} occurrences of '{pattern}'")

            # Write back if changes were made
            if content != original_content:
                with open(full_path, "w", encoding="utf-8") as f:
                    f.write(content)
                log_success(f"    Updated with {replacement_count} total replacements")
            else:
                log_info(f"    No replacements needed")

        except Exception as e:
            log_error(f"    Error processing {file_path}: {e}")
            success = False

    if success:
        log_success("String replacements completed")
    else:
        log_error("String replacements failed")

    return success
