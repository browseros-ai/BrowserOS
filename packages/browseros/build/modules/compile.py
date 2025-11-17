#!/usr/bin/env python3
"""
Build execution module for BlockBrowser build system
"""

import os
import tempfile
import shutil
import multiprocessing
from pathlib import Path
from context import BuildContext
from utils import (
    run_command,
    log_info,
    log_success,
    log_warning,
    join_paths,
    IS_WINDOWS,
    IS_MACOS,
)


def build(ctx: BuildContext) -> bool:
    """Run the actual build"""
    log_info("\n🔨 Building BlockBrowser (this will take a while)...")

    # Create VERSION file with blockbrowser_chromium_version
    if ctx.blockbrowser_chromium_version:
        # Parse the blockbrowser_chromium_version back into components
        parts = ctx.blockbrowser_chromium_version.split(".")
        if len(parts) == 4:
            version_content = f"MAJOR={parts[0]}\nMINOR={parts[1]}\nBUILD={parts[2]}\nPATCH={parts[3]}"

            # Create temporary VERSION file
            with tempfile.NamedTemporaryFile(mode="w", delete=False) as temp_file:
                temp_file.write(version_content)
                temp_path = temp_file.name

            # Copy VERSION file to chrome/VERSION
            chrome_version_path = join_paths(ctx.chromium_src, "chrome", "VERSION")
            shutil.copy2(temp_path, chrome_version_path)

            # Clean up temp file
            os.unlink(temp_path)

            log_info(
                f"Created VERSION file with blockbrowser_chromium_version: {ctx.blockbrowser_chromium_version}"
            )
    else:
        log_warning("No blockbrowser_chromium_version set. Not building")

    os.chdir(ctx.chromium_src)

    # Use default autoninja parallelism (it handles this automatically)
    autoninja_cmd = "autoninja.bat" if IS_WINDOWS else "autoninja"
    log_info("Using default autoninja parallelism")

    # Build chrome and chromedriver on Windows
    run_command([autoninja_cmd, "-C", ctx.out_dir, "chrome", "chromedriver"])

    # Rename Chromium.app to BlockBrowser.app
    app_path = ctx.get_chromium_app_path()
    new_path = ctx.get_app_path()

    if app_path.exists() and not new_path.exists():
        shutil.move(str(app_path), str(new_path))

    log_success("Build complete!")
    return True
