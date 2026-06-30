#!/usr/bin/env python3
"""String replacement module for BrowserOS build system"""

import re
from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import log_info, log_success, log_error, log_warning


class StringReplacesModule(CommandModule):
    produces = []
    requires = []
    description = "Apply branding string replacements in Chromium"

    def validate(self, ctx: Context) -> None:
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

    def execute(self, ctx: Context) -> None:
        log_info("\n🔤 Applying string replacements...")
        if not apply_string_replacements_impl(ctx):
            raise RuntimeError("Failed to apply string replacements")


# List of files to apply replacements to
target_files = [
    "chrome/app/chromium_strings.grd",
    "chrome/app/settings_chromium_strings.grdp",
]


def apply_string_replacements_impl(ctx: Context) -> bool:
    """Internal implementation for applying string replacements"""

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
            for pattern, replacement in ctx.product.string_replacements:
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
                log_info("    No replacements needed")

        except Exception as e:
            log_error(f"    Error processing {file_path}: {e}")
            success = False

    if success:
        log_success("String replacements completed")
    else:
        log_error("String replacements failed")

    return success
