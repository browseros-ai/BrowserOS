#!/usr/bin/env python3
"""Build CLI - Modular build system for BrowserOS"""

import os
import sys
import time
from pathlib import Path
from typing import Optional

import typer

# Import common modules
from ..common.context import Context
from ..common.config import load_config, validate_required_envs
from ..common.pipeline import validate_pipeline, show_available_modules
from ..common.resolver import resolve_config, resolve_pipeline
from ..common.notify import (
    notify_pipeline_start,
    notify_pipeline_end,
    notify_pipeline_error,
    notify_module_start,
    notify_module_completion,
    set_build_context,
)
from ..common.module import ValidationError
from ..common.utils import (
    log_error,
    log_info,
    log_success,
    log_warning,
    IS_MACOS,
    IS_WINDOWS,
    IS_LINUX,
)

# Import all module classes
from ..modules.setup.clean import CleanModule
from ..modules.setup.git import GitSetupModule, SparkleSetupModule
from ..modules.setup.configure import ConfigureModule
from ..modules.compile import CompileModule, UniversalBuildModule
from ..modules.patches.patches import PatchesModule
from ..modules.patches.series_patches import SeriesPatchesModule
from ..modules.resources.chromium_replace import ChromiumReplaceModule
from ..modules.resources.string_replaces import StringReplacesModule
from ..modules.resources.resources import ResourcesModule
from ..modules.upload import UploadModule

# Platform-specific modules (imported unconditionally - validation handles platform checks)
from ..modules.sign.macos import MacOSSignModule
from ..modules.sign.windows import WindowsSignModule
from ..modules.sign.linux import LinuxSignModule
from ..modules.sign.sparkle import SparkleSignModule
from ..modules.package.macos import MacOSPackageModule
from ..modules.package.windows import WindowsPackageModule
from ..modules.package.linux import LinuxPackageModule

AVAILABLE_MODULES = {
    # Setup & Environment
    "clean": CleanModule,
    "git_setup": GitSetupModule,
    "sparkle_setup": SparkleSetupModule,
    "configure": ConfigureModule,
    # Patches & Resources
    "patches": PatchesModule,
    "series_patches": SeriesPatchesModule,
    "chromium_replace": ChromiumReplaceModule,
    "string_replaces": StringReplacesModule,
    "resources": ResourcesModule,
    # Build
    "compile": CompileModule,
    "universal_build": UniversalBuildModule,  # macOS universal binary (arm64 + x64)
    # Sign (platform-specific, validated at runtime)
    "sign_macos": MacOSSignModule,
    "sign_windows": WindowsSignModule,
    "sign_linux": LinuxSignModule,
    "sparkle_sign": SparkleSignModule,  # macOS Sparkle signing for auto-update
    # Package (platform-specific, validated at runtime)
    "package_macos": MacOSPackageModule,
    "package_windows": WindowsPackageModule,
    "package_linux": LinuxPackageModule,
    # Upload
    "upload": UploadModule,
}


def _get_sign_module():
    """Get platform-specific sign module name"""
    if IS_MACOS():
        return "sign_macos"
    elif IS_WINDOWS():
        return "sign_windows"
    elif IS_LINUX():
        return "sign_linux"
    else:
        log_error("Unsupported platform for packaging")
        sys.exit(1)


def _get_package_module():
    """Get platform-specific package module name"""
    if IS_MACOS():
        return "package_macos"
    elif IS_WINDOWS():
        return "package_windows"
    elif IS_LINUX():
        return "package_linux"
    else:
        log_error("Unsupported platform for packaging")
        sys.exit(1)


# Fixed execution order - flags enable/disable phases, order is always the same
EXECUTION_ORDER = [
    # Phase 1: Setup & Clean
    ("setup", ["clean", "git_setup", "sparkle_setup"]),
    # Phase 2: Patches & Resources
    (
        "prep",
        [
            "resources",
            "chromium_replace",
            "string_replaces",
            "patches",
            "configure",
        ],
    ),
    # Phase 3: Build
    ("build", ["compile"]),
    # Phase 4: Code Signing (platform-aware)
    ("sign", [_get_sign_module()]),
    # Phase 5: Packaging (platform-aware)
    ("package", [_get_package_module()]),
    # Phase 6: Upload
    ("upload", ["upload"]),
]

# Modules that trigger Slack notifications (to reduce verbosity)
NOTIFY_MODULES = [
    "compile",
    "sign_macos",
    "sign_windows",
    "sign_linux",
    "package_macos",
    "package_windows",
    "package_linux",
    "upload",
]


def execute_pipeline(
    ctx: Context,
    pipeline: list[str],
    available_modules: dict,
    pipeline_name: str = "build",
) -> None:
    """Execute a build pipeline by running modules sequentially.

    Args:
        ctx: Build context with paths and configuration
        pipeline: List of module names to execute in order
        available_modules: Dictionary mapping module names to module classes
        pipeline_name: Name of pipeline for notifications (default: "build")

    Raises:
        typer.Exit: On module validation failure, execution failure, or interrupt

    Design:
        - Executes modules sequentially in pipeline order
        - Validates each module before execution (fail fast)
        - Tracks timing for each module and total pipeline
        - Sends notifications at key lifecycle events
        - Handles interrupts (Ctrl+C) gracefully with cleanup
    """
    start_time = time.time()
    notify_pipeline_start(pipeline_name, pipeline)

    try:
        for module_name in pipeline:
            log_info(f"\n{'='*70}")
            log_info(f"🔧 Running module: {module_name}")
            log_info(f"{'='*70}")

            # Instantiate module
            module_class = available_modules[module_name]
            module = module_class()

            # Notify module start and track timing (only for key modules)
            if module_name in NOTIFY_MODULES:
                notify_module_start(module_name)
            module_start = time.time()

            # Validate right before executing (fail fast)
            try:
                module.validate(ctx)
            except ValidationError as e:
                log_error(f"Validation failed for {module_name}: {e}")
                notify_pipeline_error(
                    pipeline_name, f"{module_name} validation failed: {e}"
                )
                raise typer.Exit(1)

            # Execute module
            try:
                module.execute(ctx)
                module_duration = time.time() - module_start
                if module_name in NOTIFY_MODULES:
                    notify_module_completion(module_name, module_duration)
                log_success(f"Module {module_name} completed in {module_duration:.1f}s")
            except Exception as e:
                log_error(f"Module {module_name} failed: {e}")
                notify_pipeline_error(pipeline_name, f"{module_name} failed: {e}")
                raise typer.Exit(1)

        # Pipeline completed successfully
        duration = time.time() - start_time
        mins = int(duration / 60)
        secs = int(duration % 60)

        log_info("\n" + "=" * 70)
        log_success(f"✅ Pipeline completed successfully in {mins}m {secs}s")
        log_info("=" * 70)

        notify_pipeline_end(pipeline_name, duration)

    except KeyboardInterrupt:
        log_error("\n❌ Pipeline interrupted")
        notify_pipeline_error(pipeline_name, "Interrupted by user")
        raise typer.Exit(130)
    except typer.Exit:
        # Re-raise typer.Exit (from validation/execution failures)
        raise
    except Exception as e:
        log_error(f"\n❌ Pipeline failed: {e}")
        notify_pipeline_error(pipeline_name, str(e))
        raise typer.Exit(1)


def main(
    config: Optional[Path] = typer.Option(
        None,
        "--config",
        "-c",
        help="Load configuration from YAML file",
        exists=True,
    ),
    modules: Optional[str] = typer.Option(
        None,
        "--modules",
        "-m",
        help="Comma-separated list of modules to run",
    ),
    list_modules: bool = typer.Option(
        False,
        "--list",
        "-l",
        help="List all available modules and exit",
    ),
    # Pipeline phase flags (auto-ordered execution)
    setup: bool = typer.Option(
        False,
        "--setup",
        help="Run setup phase (clean, git_setup, sparkle_setup)",
    ),
    prep: bool = typer.Option(
        False,
        "--prep",
        help="Run prep phase (resources, chromium_replace, string_replaces, patches, configure)",
    ),
    build: bool = typer.Option(
        False,
        "--build",
        help="Run build phase (compile)",
    ),
    sign: bool = typer.Option(
        False,
        "--sign",
        help="Run sign phase (platform-specific: sign_macos/windows/linux)",
    ),
    package: bool = typer.Option(
        False,
        "--package",
        help="Run package phase (platform-specific: package_macos/windows/linux)",
    ),
    upload: bool = typer.Option(
        False,
        "--upload",
        help="Run upload phase (upload artifacts)",
    ),
    # Global options that override config
    arch: Optional[str] = typer.Option(
        None,
        "--arch",
        "-a",
        help="Architecture (arm64, x64, universal)",
    ),
    build_type: Optional[str] = typer.Option(
        None,
        "--build-type",
        "-t",
        help="Build type (debug or release)",
    ),
    chromium_src: Optional[Path] = typer.Option(
        None,
        "--chromium-src",
        "-S",
        help="Path to Chromium source directory",
    ),
):
    """BrowserOS Build System - Modular pipeline executor

    Build BrowserOS using phase flags (auto-ordered), explicit modules, or configs.

    \b
    Phase Flags (Recommended - Auto-Ordered):
      browseros build --setup --build --sign --package
      browseros build --build --sign           # Skip setup
      browseros build --package --sign         # Flags work in any order!

    \b
    Explicit Modules (Power Users):
      browseros build --modules clean,compile,sign_macos

    \b
    Config Files (CI/CD):
      browseros build --config release.yaml --arch arm64

    \b
    List Available:
      browseros build --list                   # Show all modules and phases

    Note: Phase flags always execute in correct order regardless of how you write them.
          --sign and --package auto-select platform (macos/windows/linux)
    """

    # Handle --list flag
    if list_modules:
        show_available_modules(AVAILABLE_MODULES)
        return

    # Check for mutually exclusive options
    has_config = config is not None
    has_modules = modules is not None
    has_flags = any([setup, prep, build, sign, package, upload])

    options_provided = sum([has_config, has_modules, has_flags])

    if options_provided == 0:
        typer.echo(
            "Error: Specify --config, --modules, or phase flags (--setup, --build, etc.)\n"
        )
        typer.echo("Use --help for usage information")
        typer.echo("Use --list to see available modules")
        raise typer.Exit(1)

    if options_provided > 1:
        log_error("Specify only ONE of: --config, --modules, or phase flags")
        log_error("Examples:")
        log_error("  browseros build --setup --build --sign")
        log_error("  browseros build --modules clean,compile")
        log_error("  browseros build --config release.yaml")
        raise typer.Exit(1)

    # CONFIG MODE validation: YAML controls everything, CLI build flags not allowed
    if has_config:
        conflicting_flags = []
        if arch is not None:
            conflicting_flags.append("--arch")
        if build_type is not None:
            conflicting_flags.append("--build-type")

        if conflicting_flags:
            log_error(
                f"CONFIG MODE: Cannot use {', '.join(conflicting_flags)} with --config"
            )
            log_error("When using --config, ALL build parameters come from YAML")
            log_error("Remove the conflicting flags or don't use --config")
            raise typer.Exit(1)

    log_info("🚀 BrowserOS Build System")
    log_info("=" * 70)

    # Load YAML config if provided
    config_data = load_config(config) if config else None

    # Build CLI arguments dictionary for resolver
    root_dir = Path(__file__).parent.parent.parent
    cli_args = {
        "chromium_src": chromium_src,
        "arch": arch,
        "build_type": build_type,
        "modules": modules,
        "setup": setup,
        "prep": prep,
        "build": build,
        "sign": sign,
        "package": package,
        "upload": upload,
    }

    # Resolve build context (CONFIG mode or DIRECT mode)
    try:
        ctx = resolve_config(cli_args, config_data)
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)

    # Resolve pipeline (CONFIG mode or DIRECT mode)
    try:
        pipeline = resolve_pipeline(
            cli_args,
            config_data,
            execution_order=EXECUTION_ORDER,
        )
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)

    # Show execution plan for flag-based mode
    if has_flags:
        log_info("\n📋 Execution Plan (auto-ordered):")
        log_info("-" * 70)
        phase_names = []
        if setup:
            phase_names.append("setup")
        if prep:
            phase_names.append("prep")
            log_warning("⚠️  --prep does NOT apply series_patches. Run 'browseros build -m series_patches' separately if needed.")
        if build:
            phase_names.append("build")
        if sign:
            phase_names.append(f"sign (→ {_get_sign_module()})")
        if package:
            phase_names.append(f"package (→ {_get_package_module()})")
        if upload:
            phase_names.append("upload")

        for phase_name in phase_names:
            log_info(f"  ✓ {phase_name}")

        log_info(f"\n  Pipeline: {' → '.join(pipeline)}")
        log_info("-" * 70)

    # Validate required environment variables (YAML-specific)
    if config_data:
        required_envs = config_data.get("required_envs", [])
        if required_envs:
            validate_required_envs(required_envs)

    # Validate pipeline modules exist
    validate_pipeline(pipeline, AVAILABLE_MODULES)

    # Set Windows-specific environment
    if IS_WINDOWS():
        os.environ["DEPOT_TOOLS_WIN_TOOLCHAIN"] = "0"
        log_info("Set DEPOT_TOOLS_WIN_TOOLCHAIN=0 for Windows build")

    log_info(f"📍 Root: {root_dir}")
    log_info(f"📍 Chromium: {ctx.chromium_src}")
    log_info(f"📍 Architecture: {ctx.architecture}")
    log_info(f"📍 Build type: {ctx.build_type}")
    log_info(f"📍 Output: {ctx.out_dir}")
    log_info(f"📍 Semantic version: {ctx.semantic_version}")
    log_info(f"📍 Chromium version: {ctx.chromium_version}")
    log_info(f"📍 Build offset: {ctx.browseros_build_offset}")
    log_info(f"📍 Pipeline: {' → '.join(pipeline)}")
    log_info("=" * 70)

    # Set notification context for OS and architecture
    os_name = "macOS" if IS_MACOS() else "Windows" if IS_WINDOWS() else "Linux"
    set_build_context(os_name, ctx.architecture)

    # Execute pipeline
    execute_pipeline(ctx, pipeline, AVAILABLE_MODULES, pipeline_name="build")
