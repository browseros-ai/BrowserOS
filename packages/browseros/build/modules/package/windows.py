#!/usr/bin/env python3
"""Windows packaging module for BrowserOS"""

import shutil
import zipfile
from pathlib import Path
from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import (
    run_command,
    log_info,
    log_error,
    log_success,
    log_warning,
    join_paths,
    IS_WINDOWS,
)
from ...common.notify import get_notifier, COLOR_GREEN
from ..compile.standard import autoninja_command


class MiniInstallerModule(CommandModule):
    """Build mini_installer.exe without signing.

    The signed release flow builds mini_installer inside WindowsSignModule
    (sign binaries -> build installer -> sign installer). Unsigned CI builds
    skip sign_windows entirely, so this module provides the installer build
    step that package_windows requires.
    """

    produces = []
    requires = []
    description = "Build unsigned mini_installer.exe (CI builds without signing)"

    def validate(self, context: Context) -> None:
        if not IS_WINDOWS():
            raise ValidationError("mini_installer build requires Windows")

        args_file = context.get_gn_args_file()
        if not args_file.exists():
            raise ValidationError(
                f"Build not configured - args.gn not found: {args_file}"
            )

    def execute(self, context: Context) -> None:
        if not build_mini_installer(context):
            raise RuntimeError("Failed to build mini_installer")


class WindowsPackageModule(CommandModule):
    produces = ["installer", "installer_zip"]
    requires = []
    description = "Create Windows installer and portable ZIP"

    def validate(self, context: Context) -> None:
        if not IS_WINDOWS():
            raise ValidationError("Windows packaging requires Windows")

        build_output_dir = join_paths(context.chromium_src, context.out_dir)
        mini_installer_path = build_output_dir / "mini_installer.exe"

        if not mini_installer_path.exists():
            raise ValidationError(f"mini_installer.exe not found: {mini_installer_path}")

    def execute(self, context: Context) -> None:
        log_info("\n📦 Creating Windows packages...")

        installer_path = self._create_installer(context)
        zip_path = self._create_portable_zip(context)

        context.artifact_registry.add("installer", installer_path)
        context.artifact_registry.add("installer_zip", zip_path)

        log_success("Windows packages created successfully")

        # Send Slack notification
        notifier = get_notifier()
        notifier.notify(
            "📦 Package Created",
            "Windows packages created successfully",
            {
                "Artifacts": f"{installer_path.name}, {zip_path.name}",
                "Version": context.semantic_version,
            },
            color=COLOR_GREEN,
        )

    def _create_installer(self, ctx: Context) -> Path:
        build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
        mini_installer_path = build_output_dir / "mini_installer.exe"

        output_dir = ctx.get_dist_dir()
        output_dir.mkdir(parents=True, exist_ok=True)

        installer_name = ctx.get_artifact_name("installer")
        installer_path = output_dir / installer_name

        try:
            shutil.copy2(mini_installer_path, installer_path)
            log_success(f"Installer created: {installer_name}")
            return installer_path
        except Exception as e:
            raise RuntimeError(f"Failed to create installer: {e}")

    def _create_portable_zip(self, ctx: Context) -> Path:
        build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
        mini_installer_path = build_output_dir / "mini_installer.exe"

        output_dir = ctx.get_dist_dir()
        output_dir.mkdir(parents=True, exist_ok=True)

        zip_name = ctx.get_artifact_name("installer_zip")
        zip_path = output_dir / zip_name

        try:
            with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as zipf:
                installer_name = ctx.get_artifact_name("installer")
                zipf.write(mini_installer_path, installer_name)

                file_size = mini_installer_path.stat().st_size
                log_info(f"Added installer to ZIP ({file_size // (1024*1024)} MB)")

            log_success(f"Installer ZIP created: {zip_name}")
            return zip_path
        except Exception as e:
            raise RuntimeError(f"Failed to create installer ZIP: {e}")
def build_mini_installer(ctx: Context) -> bool:
    """Build the mini_installer target if it doesn't exist"""
    log_info("\n🔨 Checking mini_installer build...")

    # Get paths
    build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
    mini_installer_path = build_output_dir / "mini_installer.exe"
    setup_exe_path = build_output_dir / "setup.exe"

    if mini_installer_path.exists() and setup_exe_path.exists():
        log_info(
            "mini_installer.exe and setup.exe already exist; rebuilding to ensure freshness"
        )
    elif setup_exe_path.exists() and not mini_installer_path.exists():
        log_info("setup.exe exists but mini_installer.exe missing")
    elif mini_installer_path.exists() and not setup_exe_path.exists():
        log_info("mini_installer.exe exists but setup.exe missing")

    log_info("Building setup and mini_installer targets...")

    # Build mini_installer using autoninja
    try:
        cmd = autoninja_command(ctx.out_dir, ["setup", "mini_installer"])

        # Change to chromium_src directory before running (like compile.py does)
        import os

        old_cwd = os.getcwd()
        os.chdir(ctx.chromium_src)

        try:
            run_command(cmd)
        finally:
            os.chdir(old_cwd)

        # Verify the file was created
        missing_artifacts = []
        if not setup_exe_path.exists():
            missing_artifacts.append("setup.exe")
        if not mini_installer_path.exists():
            missing_artifacts.append("mini_installer.exe")

        if not missing_artifacts:
            log_success("mini_installer and setup built successfully")
            return True

        log_error(
            "Build completed but missing artifacts: "
            + ", ".join(missing_artifacts)
        )
        return False

    except Exception as e:
        log_error(f"Failed to build setup/mini_installer: {e}")
        return False


def create_installer(ctx: Context) -> bool:
    """Create Windows installer (mini_installer.exe)"""
    log_info("\n🔧 Creating Windows installer...")

    # Get paths
    build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
    mini_installer_path = build_output_dir / "mini_installer.exe"

    if not mini_installer_path.exists():
        log_warning(f"mini_installer.exe not found at: {mini_installer_path}")
        log_info(
            "To build the installer, run: autoninja -C out\\Default_x64 mini_installer"
        )
        return False

    # Create output directory
    output_dir = ctx.get_dist_dir()
    output_dir.mkdir(parents=True, exist_ok=True)

    # Generate installer filename with version and architecture
    installer_name = ctx.get_artifact_name("installer")
    installer_path = output_dir / installer_name

    # Copy mini_installer to final location
    try:
        shutil.copy2(mini_installer_path, installer_path)
        log_success(f"Installer created: {installer_name}")
        return True
    except Exception as e:
        log_error(f"Failed to create installer: {e}")
        return False


def create_portable_zip(ctx: Context) -> bool:
    """Create ZIP of just the installer for easier distribution"""
    log_info("\n📦 Creating installer ZIP package...")

    # Get paths
    build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
    mini_installer_path = build_output_dir / "mini_installer.exe"

    if not mini_installer_path.exists():
        log_warning(f"mini_installer.exe not found at: {mini_installer_path}")
        log_info(
            "To build the installer, run: autoninja -C out\\Default_x64 mini_installer"
        )
        return False

    # Create output directory
    output_dir = ctx.get_dist_dir()
    output_dir.mkdir(parents=True, exist_ok=True)

    # Generate ZIP filename with version and architecture
    zip_name = ctx.get_artifact_name("installer_zip")
    zip_path = output_dir / zip_name

    # Create ZIP file containing just the installer
    try:
        with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as zipf:
            # Add mini_installer.exe to the zip
            installer_name = ctx.get_artifact_name("installer")
            zipf.write(mini_installer_path, installer_name)

            # Get file size for logging
            file_size = mini_installer_path.stat().st_size
            log_info(f"Added installer to ZIP ({file_size // (1024*1024)} MB)")

        log_success(f"Installer ZIP created: {zip_name}")
        return True
    except Exception as e:
        log_error(f"Failed to create installer ZIP: {e}")
        return False


# Sign functions moved to sign/windows.py
# - sign_binaries()
# - sign_with_codesigntool()
# - get_browseros_server_binary_paths()
# These are now in modules/sign/windows.py


def get_target_cpu(build_output_dir: Path) -> str:
    """Get target CPU architecture from build configuration"""
    args_gn_path = build_output_dir / "args.gn"

    if not args_gn_path.exists():
        return "x64"  # Default

    try:
        args_gn_content = args_gn_path.read_text(encoding="utf-8")
        for cpu in ("x64", "x86", "arm64"):
            if f'target_cpu="{cpu}"' in args_gn_content:
                return cpu
    except Exception:
        pass

    return "x64"  # Default


def create_files_cfg_package(ctx: Context) -> bool:
    """Create package using Chromium's FILES.cfg approach (alternative method)"""
    log_info("\n📦 Creating FILES.cfg-based package...")

    files_cfg_path = (
        ctx.chromium_src / "chrome" / "tools" / "build" / "win" / "FILES.cfg"
    )

    if not files_cfg_path.exists():
        log_error(f"FILES.cfg not found at: {files_cfg_path}")
        return False

    # This would require implementing the filescfg module functionality
    # from ungoogled-chromium, which is quite complex
    log_warning("FILES.cfg packaging not yet implemented")
    return False
