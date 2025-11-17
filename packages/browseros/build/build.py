#!/usr/bin/env python3
"""
Main build orchestrator for BlockBrowser Browser
"""

import os
import sys
import time
import click
from pathlib import Path
from typing import Optional


# Load .env file if it exists
def load_env_file():
    env_file = Path(__file__).parent.parent / ".env"
    if env_file.exists():
        with open(env_file, "r") as f:
            for line in f:
                line = line.strip()
                if line and not line.startswith("#"):
                    if "=" in line:
                        key, value = line.split("=", 1)
                        os.environ[key.strip()] = value.strip()
        print(f"✓ Loaded environment from .env file")


# Load .env file on import
load_env_file()

# Import shared components
from context import BuildContext
from utils import (
    load_config,
    log_info,
    log_warning,
    log_error,
    log_success,
    IS_MACOS,
    IS_WINDOWS,
    IS_LINUX,
)

# Import modules
from modules.clean import clean
from modules.git import setup_git, setup_sparkle
from modules.patches import apply_patches
from modules.resources import copy_resources
from modules.chromium_replace import replace_chromium_files, add_file_to_replacements
from modules.string_replaces import apply_string_replacements
from modules.inject import inject_version
from modules.configure import configure
from modules.compile import build
from modules.gcs import upload_package_artifacts, upload_signed_artifacts, handle_upload_dist

# Platform-specific imports
if IS_MACOS:
    from modules.sign import sign, sign_universal, check_signing_environment
    from modules.package import package, package_universal
    from modules.postbuild import run_postbuild
elif IS_WINDOWS:
    from modules.package_windows import (
        package,
        package_universal,
        sign_binaries as sign,
    )

    # Windows doesn't have universal signing
    def sign_universal(contexts: list[BuildContext]) -> bool:
        log_warning("Universal signing is not supported on Windows")
        return True

    def run_postbuild(ctx: BuildContext) -> None:
        log_warning("Post-build tasks are not implemented for Windows yet")

elif IS_LINUX:
    from modules.package_linux import package, package_universal, sign_binaries as sign

    # Linux doesn't have universal signing
    def sign_universal(contexts: list[BuildContext]) -> bool:
        log_warning("Universal signing is not supported on Linux")
        return True

    def run_postbuild(ctx: BuildContext) -> None:
        log_warning("Post-build tasks are not implemented for Linux yet")

else:
    # Stub functions for other platforms
    def sign(ctx: BuildContext) -> bool:
        log_warning("Signing is not implemented for this platform")
        return True

    def sign_universal(contexts: list[BuildContext]) -> bool:
        log_warning("Universal signing is not implemented for this platform")
        return True

    def package(ctx: BuildContext) -> bool:
        log_warning("Packaging is not implemented for this platform")
        return True

    def package_universal(contexts: list[BuildContext]) -> bool:
        log_warning("Universal packaging is not implemented for this platform")
        return True

    def run_postbuild(ctx: BuildContext) -> None:
        log_warning("Post-build tasks are not implemented for this platform")


from modules.slack import (
    notify_build_started,
    notify_build_step,
    notify_build_success,
    notify_build_failure,
    notify_build_interrupted,
    notify_gcs_upload,
)


def build_main(
    config_file: Optional[Path] = None,
    clean_flag: bool = False,
    git_setup_flag: bool = False,
    apply_patches_flag: bool = False,
    sign_flag: bool = False,
    package_flag: bool = False,
    build_flag: bool = False,
    arch: str = "",  # Will use platform default if not specified
    build_type: str = "debug",
    chromium_src_dir: Optional[Path] = None,
    slack_notifications: bool = False,
    patch_interactive: bool = False,
    patch_commit: bool = False,
    upload_gcs: bool = True,  # Default to uploading to GCS
):
    """Main build orchestration"""
    log_info("🚀 BlockBrowser Build System")
    log_info("=" * 50)

    # Check if sign flag is enabled and required environment variables are set
    if sign_flag and IS_MACOS:
        if not check_signing_environment():
            sys.exit(1)

    # Set Windows-specific environment variables
    if IS_WINDOWS:
        os.environ["DEPOT_TOOLS_WIN_TOOLCHAIN"] = "0"
        log_info("🔧 Set DEPOT_TOOLS_WIN_TOOLCHAIN=0 for Windows build")

    # Setup context
    root_dir = Path(__file__).parent.parent

    # Initialize chromium_src as None - will be set from CLI or config
    chromium_src = None

    # Load config if provided
    config = None
    gn_flags_file = None
    architectures = [arch] if arch else []  # Empty list if no arch specified
    universal = False
    certificate_name = None  # For Windows signing
    if config_file:
        config = load_config(config_file)
        log_info(f"📄 Loaded config from: {config_file}")

        # Override parameters from config
        if "build" in config:
            build_type = config["build"].get("type", build_type)
            arch = config["build"].get("architecture", arch)
            # Check for multi-architecture builds
            if "architectures" in config["build"]:
                architectures = config["build"]["architectures"]
            universal = config["build"].get("universal", False)

        if "steps" in config:
            clean_flag = config["steps"].get("clean", clean_flag)
            git_setup_flag = config["steps"].get("git_setup", git_setup_flag)
            apply_patches_flag = config["steps"].get(
                "apply_patches", apply_patches_flag
            )
            build_flag = config["steps"].get("build", build_flag)
            sign_flag = config["steps"].get("sign", sign_flag)
            package_flag = config["steps"].get("package", package_flag)

        # Override slack notifications from config if not explicitly set via CLI
        if "notifications" in config:
            slack_notifications = config["notifications"].get(
                "slack", slack_notifications
            )

        if "gn_flags" in config and "file" in config["gn_flags"]:
            gn_flags_file = Path(config["gn_flags"]["file"])

        # Get chromium_src from config (only if not provided via CLI)
        if (
            not chromium_src_dir
            and "paths" in config
            and "chromium_src" in config["paths"]
        ):
            config_chromium_src = Path(config["paths"]["chromium_src"])
            chromium_src = config_chromium_src
            log_info(f"📁 Using Chromium source from config: {chromium_src}")

        # Get Windows signing certificate name from config
        if (
            IS_WINDOWS
            and "signing" in config
            and "certificate_name" in config["signing"]
        ):
            certificate_name = config["signing"]["certificate_name"]
            log_info(f"🔏 Using certificate for signing: {certificate_name}")

    # CLI takes precedence over config
    if chromium_src_dir:
        chromium_src = chromium_src_dir
        log_info(f"📁 Using Chromium source from CLI: {chromium_src}")

    # Enforce chromium_src requirement
    if not chromium_src:
        log_error("Chromium source directory is required!")
        log_error(
            "Provide it via --chromium-src CLI option or paths.chromium_src in config YAML"
        )
        log_error("Example: python build.py --chromium-src /path/to/chromium/src")
        raise ValueError("chromium_src is required but not provided")

    # Validate chromium_src path exists
    if not chromium_src.exists():
        log_error(f"Chromium source directory does not exist: {chromium_src}")
        log_error("Please provide a valid chromium source path")
        raise FileNotFoundError(f"Chromium source directory not found: {chromium_src}")

    # If no architectures specified, use platform default
    if not architectures:
        from utils import get_platform_arch

        architectures = [get_platform_arch()]
        log_info(f"📍 Using platform default architecture: {architectures[0]}")

    # Display build configuration
    log_info(f"📍 Root: {root_dir}")
    log_info(f"📍 Chromium source: {chromium_src}")
    log_info(f"📍 Architectures: {architectures}")
    log_info(f"📍 Universal build: {universal}")
    log_info(f"📍 Build type: {build_type}")

    # Start time for overall build
    start_time = time.time()

    # Notify build started (if enabled)
    if slack_notifications:
        notify_build_started(build_type, str(architectures))

    # Run build steps
    try:
        built_contexts = []
        all_gcs_uris = []  # Track all uploaded GCS URIs

        # Build each architecture separately
        for arch_name in architectures:
            log_info(f"\n{'='*60}")
            log_info(f"🏗️  Building for architecture: {arch_name}")
            log_info(f"{'='*60}")

            ctx = BuildContext(
                root_dir=root_dir,
                chromium_src=chromium_src,
                architecture=arch_name,
                build_type=build_type,
                apply_patches=apply_patches_flag,
                sign_package=sign_flag,
                package=package_flag,
                build=build_flag,
            )

            log_info(f"📍 Chromium: {ctx.chromium_version}")
            log_info(f"📍 BlockBrowser: {ctx.blockbrowser_version}")
            log_info(f"📍 Output directory: {ctx.out_dir}")

            # Clean (only for first architecture to avoid conflicts)
            if clean_flag and arch_name == architectures[0]:
                clean(ctx)
                if slack_notifications:
                    notify_build_step("Completed cleaning build artifacts")

            # Git setup (only once for first architecture)
            if git_setup_flag and arch_name == architectures[0]:
                setup_git(ctx)
                if slack_notifications:
                    notify_build_step("Completed Git setup and Chromium source")

            # Apply patches (only once for first architecture)
            if apply_patches_flag and arch_name == architectures[0]:
                # First do chromium file replacements
                replace_chromium_files(ctx)

                # Then apply string replacements
                apply_string_replacements(ctx)

                # Setup sparkle (macOS only)
                if IS_MACOS:
                    setup_sparkle(ctx)
                else:
                    log_info("Skipping Sparkle setup (macOS only)")

                # Apply patches
                apply_patches(
                    ctx, interactive=patch_interactive, commit_each=patch_commit
                )

                if slack_notifications:
                    notify_build_step("Completed applying patches")

            # Copy resources for each architecture (YAML filters by arch)
            if apply_patches_flag:
                copy_resources(ctx, commit_each=patch_commit)

                if slack_notifications:
                    notify_build_step(
                        f"Completed copying resources for {arch_name}"
                    )

            # Build for this architecture
            if build_flag:
                if slack_notifications:
                    notify_build_step(f"Started building for {arch_name}")
                configure(ctx, gn_flags_file)
                build(ctx)

                # Run post-build tasks
                # run_postbuild(ctx)

                if slack_notifications:
                    notify_build_step(f"Completed building for {arch_name}")

            # Sign and package immediately after building each architecture
            if sign_flag:
                log_info(f"\n🔏 Signing {ctx.architecture} build...")
                if slack_notifications:
                    notify_build_step(f"[{ctx.architecture}] Started signing")
                # Pass certificate_name for Windows signing
                if IS_WINDOWS:
                    sign(ctx, certificate_name)
                else:
                    sign(ctx)
                if slack_notifications:
                    notify_build_step(f"[{ctx.architecture}] Completed signing")

            if package_flag:
                log_info(f"\n📦 Packaging {ctx.architecture} build...")
                if slack_notifications:
                    package_type = (
                        "DMG" if IS_MACOS else "installer" if IS_WINDOWS else "AppImage"
                    )
                    notify_build_step(
                        f"[{ctx.architecture}] Started {package_type} creation"
                    )
                package(ctx)
                if slack_notifications:
                    package_type = (
                        "DMG" if IS_MACOS else "installer" if IS_WINDOWS else "AppImage"
                    )
                    notify_build_step(
                        f"[{ctx.architecture}] Completed {package_type} creation"
                    )

                # Upload to GCS after packaging
                gcs_uris = []
                if upload_gcs:
                    success, gcs_uris = upload_package_artifacts(ctx)
                    if not success:
                        log_warning("Failed to upload package artifacts to GCS")
                    elif gcs_uris and slack_notifications:
                        notify_gcs_upload(ctx.architecture, gcs_uris)
                        all_gcs_uris.extend(gcs_uris)

            built_contexts.append(ctx)

        # Handle universal build if requested
        if len(architectures) > 1 and universal:
            # Universal build: merge, sign and package
            log_info(f"\n{'='*60}")
            log_info("🔄 Creating universal binary...")
            log_info(f"{'='*60}")

            # Import merge function
            from modules.merge import merge_architectures

            # Get paths for the built apps
            arch1_app = built_contexts[0].get_app_path()
            arch2_app = built_contexts[1].get_app_path()

            # Clean up old universal output directory if it exists
            universal_dir = built_contexts[0].chromium_src / "out/Default_universal"
            if universal_dir.exists():
                log_info("🧹 Cleaning up old universal output directory...")
                from utils import safe_rmtree

                safe_rmtree(universal_dir)

            # Create fresh universal output path
            universal_dir.mkdir(parents=True, exist_ok=True)
            universal_app_path = universal_dir / built_contexts[0].NXTSCAPE_APP_NAME

            # Find universalizer script
            universalizer_script = root_dir / "build" / "universalizer_patched.py"

            # Merge the architectures
            if not merge_architectures(
                arch1_app, arch2_app, universal_app_path, universalizer_script
            ):
                raise RuntimeError(
                    "Failed to merge architectures into universal binary"
                )

            if slack_notifications:
                notify_build_step(
                    "Completed merging architectures into universal binary"
                )

            if sign_flag:
                if slack_notifications:
                    notify_build_step("[Universal] Started signing and notarization")
                sign_universal(built_contexts)
                if slack_notifications:
                    notify_build_step("[Universal] Completed signing and notarization")

            if package_flag:
                if slack_notifications:
                    package_type = (
                        "DMG" if IS_MACOS else "installer" if IS_WINDOWS else "AppImage"
                    )
                    notify_build_step(f"[Universal] Started {package_type} creation")
                package_universal(built_contexts)
                if slack_notifications:
                    package_type = (
                        "DMG" if IS_MACOS else "installer" if IS_WINDOWS else "AppImage"
                    )
                    notify_build_step(f"[Universal] Completed {package_type} creation")

                # Upload universal package to GCS
                universal_gcs_uris = []
                if upload_gcs:
                    # Use the first context with universal architecture override
                    universal_ctx = built_contexts[0]
                    original_arch = universal_ctx.architecture
                    universal_ctx.architecture = "universal"
                    success, universal_gcs_uris = upload_package_artifacts(
                        universal_ctx
                    )
                    if not success:
                        log_warning(
                            "Failed to upload universal package artifacts to GCS"
                        )
                    elif universal_gcs_uris and slack_notifications:
                        notify_gcs_upload("universal", universal_gcs_uris)
                        all_gcs_uris.extend(universal_gcs_uris)
                    universal_ctx.architecture = original_arch

        # Summary
        elapsed = time.time() - start_time
        mins = int(elapsed / 60)
        secs = int(elapsed % 60)

        log_info("\n" + "=" * 60)
        log_success(
            f"Build completed for {len(architectures)} architecture(s) in {mins}m {secs}s"
        )
        if universal and len(architectures) > 1:
            log_success("Universal binary created successfully!")
        log_info("=" * 60)

        # Notify build success (if enabled)
        if slack_notifications:
            notify_build_success(mins, secs, gcs_uris=all_gcs_uris)

    except KeyboardInterrupt:
        log_warning("\nBuild interrupted")
        if slack_notifications:
            notify_build_interrupted()
        sys.exit(130)
    except Exception as e:
        log_error(f"\nBuild failed: {e}")
        if slack_notifications:
            notify_build_failure(str(e))
        sys.exit(1)


@click.command()
@click.option(
    "--config",
    "-c",
    type=click.Path(exists=True, path_type=Path),
    help="Load configuration from YAML file",
)
@click.option("--clean", "-C", is_flag=True, default=False, help="Clean before build")
@click.option("--git-setup", "-g", is_flag=True, default=False, help="Git setup")
@click.option(
    "--apply-patches", "-p", is_flag=True, default=False, help="Apply patches"
)
@click.option(
    "--sign", "-s", is_flag=True, default=False, help="Sign and notarize the app"
)
@click.option(
    "--arch",
    "-a",
    type=click.Choice(["arm64", "x64"]),
    default=None,
    help="Architecture (defaults to platform-specific)",
)
@click.option(
    "--build-type",
    "-t",
    type=click.Choice(["debug", "release"]),
    default="debug",
    help="Build type",
)
@click.option(
    "--package",
    "-P",
    is_flag=True,
    default=False,
    help="Create package (DMG/AppImage/Installer)",
)
@click.option("--build", "-b", is_flag=True, default=False, help="Build")
@click.option(
    "--chromium-src",
    "-S",
    type=click.Path(exists=False, path_type=Path),
    help="Path to Chromium source directory",
)
@click.option(
    "--slack-notifications",
    "-n",
    is_flag=True,
    default=False,
    help="Enable Slack notifications",
)
@click.option(
    "--merge",
    nargs=2,
    type=click.Path(path_type=Path),
    metavar="ARCH1_APP ARCH2_APP",
    help="Merge two architecture builds: --merge path/to/arch1.app path/to/arch2.app",
)
@click.option(
    "--add-replace",
    type=click.Path(exists=True, path_type=Path),
    help="Add a file to chromium_src replacement directory: --add-replace /path/to/chromium/src/file --chromium-src /path/to/chromium/src",
)
@click.option(
    "--string-replace",
    is_flag=True,
    default=False,
    help="Apply string replacements to chromium files",
)
@click.option(
    "--patch-interactive",
    "-i",
    is_flag=True,
    default=False,
    help="Ask for confirmation before applying each patch",
)
@click.option(
    "--patch-commit",
    is_flag=True,
    default=False,
    help="Create a git commit after applying each patch",
)
@click.option(
    "--no-gcs-upload",
    is_flag=True,
    default=False,
    help="Skip uploading artifacts to Google Cloud Storage",
)
@click.option(
    "--upload-dist",
    type=click.Path(exists=True, path_type=Path),
    help="Upload pre-built artifacts from dist/<version> directory to GCS: --upload-dist dist/61",
)
@click.option(
    "--platform",
    type=click.Choice(["macos", "linux", "win"]),
    default=None,
    help="Override platform for GCS upload (auto-detected if not specified)",
)
def main(
    config,
    clean,
    git_setup,
    apply_patches,
    sign,
    arch,
    build_type,
    package,
    build,
    chromium_src,
    slack_notifications,
    merge,
    add_replace,
    string_replace,
    patch_interactive,
    patch_commit,
    no_gcs_upload,
    upload_dist,
    platform,
):
    """Simple build system for BlockBrowser Browser"""

    # Validate chromium-src for commands that need it
    if add_replace or merge or string_replace or (not config and chromium_src is None):
        if not chromium_src:
            if add_replace:
                log_error("--add-replace requires --chromium-src to be specified")
                log_error(
                    "Example: python build.py --add-replace /path/to/chromium/src/chrome/file.cc --chromium-src /path/to/chromium/src"
                )
            elif merge:
                log_error("--merge requires --chromium-src to be specified")
                log_error(
                    "Example: python build.py --merge app1.app app2.app --chromium-src /path/to/chromium/src"
                )
            elif string_replace:
                log_error("--string-replace requires --chromium-src to be specified")
                log_error(
                    "Example: python build.py --string-replace --chromium-src /path/to/chromium/src"
                )
            else:
                log_error("--chromium-src is required when not using a config file")
                log_error(
                    "Example: python build.py --chromium-src /path/to/chromium/src"
                )
            sys.exit(1)

        # Validate chromium_src path exists
        if not chromium_src.exists():
            log_error(f"Chromium source directory does not exist: {chromium_src}")
            sys.exit(1)

    # Handle string-replace command
    if string_replace:
        # Get root directory
        root_dir = Path(__file__).parent.parent

        # Create a minimal context for string replacements
        from context import BuildContext

        ctx = BuildContext(
            root_dir=root_dir,
            chromium_src=chromium_src,
            architecture="",  # Use platform default
            build_type="debug",  # Not used for string replacements
        )

        # Apply string replacements
        if apply_string_replacements(ctx):
            sys.exit(0)
        else:
            sys.exit(1)

    # Handle add-replace command
    if add_replace:
        # Get root directory
        root_dir = Path(__file__).parent.parent

        # Call the function from chromium_replace module
        if add_file_to_replacements(add_replace, chromium_src, root_dir):
            sys.exit(0)
        else:
            sys.exit(1)

    # Handle merge command
    if merge:
        from modules.merge import handle_merge_command

        arch1_path, arch2_path = merge

        if handle_merge_command(arch1_path, arch2_path, chromium_src, sign, package):
            sys.exit(0)
        else:
            sys.exit(1)

    # Handle upload-dist command
    if upload_dist:
        # Get root directory
        root_dir = Path(__file__).parent.parent

        # Call the upload handler from gcs module
        if handle_upload_dist(upload_dist, root_dir, platform_override=platform):
            sys.exit(0)
        else:
            sys.exit(1)

    # Regular build workflow
    build_main(
        config_file=config,
        clean_flag=clean,
        git_setup_flag=git_setup,
        apply_patches_flag=apply_patches,
        sign_flag=sign,
        package_flag=package,
        build_flag=build,
        arch=arch or "",  # Pass empty string to use platform default
        build_type=build_type,
        chromium_src_dir=chromium_src,
        slack_notifications=slack_notifications,
        patch_interactive=patch_interactive,
        patch_commit=patch_commit,
        upload_gcs=not no_gcs_upload,  # Invert the flag
    )


if __name__ == "__main__":
    main.main(standalone_mode=False)
