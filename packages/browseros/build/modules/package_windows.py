#!/usr/bin/env python3
"""
Windows packaging module for BlockBrowser Browser
Based on ungoogled-chromium-windows packaging approach
"""

import os
import sys
import shutil
import zipfile
from pathlib import Path
from typing import Optional, List
from context import BuildContext
from utils import (
    run_command,
    log_info,
    log_error,
    log_success,
    log_warning,
    join_paths,
    IS_WINDOWS,
)

# BlockBrowser Server binaries packaged alongside Chrome that must be signed prior to
# building the installer. Extend this list when new server-side executables are added.
BROWSEROS_SERVER_BINARIES: List[str] = [
    "blockbrowser_server.exe",
    "codex.exe",
]


def get_blockbrowser_server_binary_paths(build_output_dir: Path) -> List[Path]:
    """Return absolute paths to BlockBrowser Server binaries for signing."""
    server_dir = build_output_dir / "BlockBrowserServer" / "default" / "resources" / "bin"
    return [server_dir / binary for binary in BROWSEROS_SERVER_BINARIES]


def package(ctx: BuildContext) -> bool:
    """Create Windows packages (installer and portable zip)"""
    log_info("\n📦 Creating Windows packages...")

    # First, ensure mini_installer is built
    # if not build_mini_installer(ctx):
    #     log_error("Failed to build mini_installer")
    #     return False

    # Create both installer and portable zip
    success = True

    if create_installer(ctx):
        log_success("Installer created successfully")
    else:
        log_error("Failed to create installer")
        success = False

    if create_portable_zip(ctx):
        log_success("Portable ZIP created successfully")
    else:
        log_error("Failed to create portable ZIP")
        success = False

    return success


def build_mini_installer(ctx: BuildContext) -> bool:
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
        # Use autoninja.bat on Windows
        autoninja_cmd = "autoninja.bat" if IS_WINDOWS else "autoninja"

        # Build the mini_installer target
        cmd = [
            autoninja_cmd,
            "-C",
            ctx.out_dir,  # Use relative path like in compile.py
            "setup",
            "mini_installer",
        ]

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


def create_installer(ctx: BuildContext) -> bool:
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
    installer_name = f"{ctx.get_app_base_name()}_{ctx.get_blockbrowser_chromium_version()}_{ctx.architecture}_installer.exe"
    installer_path = output_dir / installer_name

    # Copy mini_installer to final location
    try:
        shutil.copy2(mini_installer_path, installer_path)
        log_success(f"Installer created: {installer_name}")
        return True
    except Exception as e:
        log_error(f"Failed to create installer: {e}")
        return False


def create_portable_zip(ctx: BuildContext) -> bool:
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
    zip_name = f"{ctx.get_app_base_name()}_{ctx.get_blockbrowser_chromium_version()}_{ctx.architecture}_installer.zip"
    zip_path = output_dir / zip_name

    # Create ZIP file containing just the installer
    try:
        with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as zipf:
            # Add mini_installer.exe to the zip
            installer_name = f"{ctx.get_app_base_name()}_{ctx.get_blockbrowser_version()}_{ctx.architecture}_installer.exe"
            zipf.write(mini_installer_path, installer_name)

            # Get file size for logging
            file_size = mini_installer_path.stat().st_size
            log_info(f"Added installer to ZIP ({file_size // (1024*1024)} MB)")

        log_success(f"Installer ZIP created: {zip_name}")
        return True
    except Exception as e:
        log_error(f"Failed to create installer ZIP: {e}")
        return False


def sign_binaries(ctx: BuildContext, certificate_name: Optional[str] = None) -> bool:
    """Sign Windows binaries using SSL.com CodeSignTool"""
    log_info("\n🔏 Signing Windows binaries...")

    # Get paths to sign
    build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)

    # STEP 1: Sign chrome.exe and BlockBrowser Server binaries BEFORE building mini_installer
    log_info("\nStep 1/3: Signing executables before packaging...")
    binaries_to_sign_first = [build_output_dir / "chrome.exe"]
    binaries_to_sign_first.extend(get_blockbrowser_server_binary_paths(build_output_dir))

    # Check which binaries exist
    existing_binaries = []
    for binary in binaries_to_sign_first:
        if binary.exists():
            existing_binaries.append(binary)
            log_info(f"Found binary to sign: {binary.name}")
        else:
            log_warning(f"Binary not found: {binary}")

    if not existing_binaries:
        log_error("No binaries found to sign")
        return False

    # Sign the executables
    if not sign_with_codesigntool(existing_binaries):
        log_error("Failed to sign executables")
        return False

    # STEP 2: Build mini_installer to package the signed binaries
    log_info("\nStep 2/3: Building mini_installer with signed binaries...")
    if not build_mini_installer(ctx):
        log_error("Failed to build mini_installer")
        return False

    # STEP 3: Sign the mini_installer.exe
    log_info("\nStep 3/3: Signing mini_installer.exe...")
    mini_installer_path = build_output_dir / "mini_installer.exe"
    if not mini_installer_path.exists():
        log_error(f"mini_installer.exe not found at: {mini_installer_path}")
        return False

    if not sign_with_codesigntool([mini_installer_path]):
        log_error("Failed to sign mini_installer.exe")
        return False

    log_success("✅ All binaries signed successfully!")
    return True


def sign_with_codesigntool(binaries: List[Path]) -> bool:
    """Sign binaries using SSL.com CodeSignTool"""
    log_info("Using SSL.com CodeSignTool for signing...")

    # Get CodeSignTool directory from environment
    codesigntool_dir = os.environ.get("CODE_SIGN_TOOL_PATH")
    if not codesigntool_dir:
        log_error("CODE_SIGN_TOOL_PATH not set in .env file")
        log_error("Set CODE_SIGN_TOOL_PATH=C:/src/CodeSignTool-v1.3.2-windows")
        return False

    # Construct path to CodeSignTool.bat
    codesigntool_path = Path(codesigntool_dir) / "CodeSignTool.bat"
    if not codesigntool_path.exists():
        log_error(f"CodeSignTool.bat not found at: {codesigntool_path}")
        log_error(f"Make sure CODE_SIGN_TOOL_PATH points to the CodeSignTool directory")
        return False

    # Check for required environment variables
    username = os.environ.get("ESIGNER_USERNAME")
    password = os.environ.get("ESIGNER_PASSWORD")
    totp_secret = os.environ.get("ESIGNER_TOTP_SECRET")
    credential_id = os.environ.get("ESIGNER_CREDENTIAL_ID")

    if not all([username, password, totp_secret]):
        log_error("Missing required eSigner environment variables in .env:")
        log_error("  ESIGNER_USERNAME=your-email")
        log_error("  ESIGNER_PASSWORD=your-password")
        log_error("  ESIGNER_TOTP_SECRET=your-totp-secret")
        if not credential_id:
            log_warning("  ESIGNER_CREDENTIAL_ID is recommended but optional")
        return False

    all_success = True
    for binary in binaries:
        try:
            log_info(f"Signing {binary.name}...")

            # Build command
            # Create a temp output directory to avoid source/dest conflict
            temp_output_dir = binary.parent / "signed_temp"
            temp_output_dir.mkdir(exist_ok=True)

            cmd = [
                str(codesigntool_path),
                "sign",
                "-username",
                username,
                "-password",
                f'"{password}"',  # Always quote the password for shell
            ]

            # Add credential_id BEFORE totp_secret (order matters!)
            if credential_id:
                cmd.extend(["-credential_id", credential_id])

            cmd.extend(
                [
                    "-totp_secret",
                    totp_secret,
                    "-input_file_path",
                    str(binary),
                    "-output_dir_path",
                    str(temp_output_dir),
                    "-override",  # Add this back
                ]
            )

            # Note: Timestamp server is configured on SSL.com side automatically

            # CodeSignTool needs to be run as a shell command for proper quote handling
            cmd_str = " ".join(cmd)
            log_info(f"Running: {cmd_str}")

            import subprocess

            result = subprocess.run(
                cmd_str,
                shell=True,
                capture_output=True,
                text=True,
                cwd=str(codesigntool_path.parent),
            )

            # Print output for debugging
            if result.stdout:
                for line in result.stdout.split("\n"):
                    if line.strip():
                        log_info(line.strip())
            if result.stderr:
                for line in result.stderr.split("\n"):
                    if line.strip() and "WARNING" not in line:
                        log_error(line.strip())

            # Check if signing actually succeeded by looking for error messages
            # CodeSignTool returns 0 even on auth errors, so we need to check output
            if result.stdout and "Error:" in result.stdout:
                log_error(
                    f"✗ Failed to sign {binary.name} - Authentication or signing error"
                )
                all_success = False
                continue

            # Move the signed file back to original location
            signed_file = temp_output_dir / binary.name
            if signed_file.exists():
                import shutil

                shutil.move(str(signed_file), str(binary))
                log_info(f"Moved signed {binary.name} to original location")

            # Clean up temp directory
            try:
                temp_output_dir.rmdir()
            except:
                pass  # Directory might not be empty

            # Verify the file is actually signed (Windows only)
            verify_cmd = [
                "powershell",
                "-Command",
                f"(Get-AuthenticodeSignature '{binary}').Status",
            ]
            try:
                import subprocess

                verify_result = subprocess.run(
                    verify_cmd, capture_output=True, text=True
                )
                if "Valid" in verify_result.stdout:
                    log_success(f"✓ {binary.name} signed and verified successfully")
                else:
                    log_error(
                        f"✗ {binary.name} signing verification failed - Status: {verify_result.stdout.strip()}"
                    )
                    all_success = False
            except:
                log_warning(f"Could not verify signature for {binary.name}")

        except Exception as e:
            log_error(f"Failed to sign {binary.name}: {e}")
            all_success = False

    return all_success


def package_universal(contexts: List[BuildContext]) -> bool:
    """Windows doesn't support universal binaries like macOS"""
    log_warning("Universal binaries are not supported on Windows")
    log_info("Consider creating separate packages for each architecture")
    return True


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


def create_files_cfg_package(ctx: BuildContext) -> bool:
    """Create package using Chromium's FILES.cfg approach (alternative method)"""
    log_info("\n📦 Creating FILES.cfg-based package...")

    build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
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
