#!/usr/bin/env python3
"""
Build configuration module for BlockBrowser build system
"""

import os
import sys
from pathlib import Path
from typing import Optional
from context import BuildContext
from utils import run_command, log_info, log_error, log_success, join_paths, IS_WINDOWS


def configure(ctx: BuildContext, gn_flags_file: Optional[Path] = None) -> bool:
    """Configure the build with GN"""
    log_info(f"\n⚙️  Configuring {ctx.build_type} build for {ctx.architecture}...")

    # Create output directory
    out_path = join_paths(ctx.chromium_src, ctx.out_dir)
    out_path.mkdir(parents=True, exist_ok=True)

    # Copy build flags
    if gn_flags_file is None:
        flags_file = ctx.get_gn_flags_file()
    else:
        flags_file = join_paths(ctx.root_dir, gn_flags_file)

    if not flags_file.exists():
        log_error(f"GN flags file not found: {flags_file}")
        raise FileNotFoundError(f"GN flags file not found: {flags_file}")

    args_file = ctx.get_gn_args_file()

    args_content = flags_file.read_text()
    args_content += f'\ntarget_cpu = "{ctx.architecture}"\n'

    args_file.write_text(args_content)

    # Run gn gen
    os.chdir(ctx.chromium_src)
    gn_cmd = "gn.bat" if IS_WINDOWS else "gn"
    run_command([gn_cmd, "gen", ctx.out_dir, "--fail-on-unused-args"])

    log_success("Build configured")
    return True
