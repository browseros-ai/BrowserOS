#!/usr/bin/env python3
"""
DMG creation and packaging module for BlockBrowser
NOTE: This module is macOS-specific. Windows packaging would require different tools (MSI/NSIS)
"""

import sys
import shutil
from pathlib import Path
from typing import Optional, List
from context import BuildContext
from utils import run_command, log_info, log_error, log_success


def package(ctx: BuildContext) -> bool:
    """Create DMG package (only if not done by signing)"""
    if ctx.sign_package:
        # Already handled by signing process
        return True

    log_info("\n📀 Creating DMG package...")

    app_path = ctx.get_app_path()
    dmg_dir = ctx.get_dist_dir()
    dmg_name = ctx.get_dmg_name()
    dmg_path = dmg_dir / dmg_name

    # Use Chromium's pkg-dmg tool
    pkg_dmg_path = ctx.get_pkg_dmg_path()

    if create_dmg(app_path, dmg_path, "BlockBrowser", pkg_dmg_path):
        log_success(f"Created {dmg_name}")
        return True
    else:
        log_error("Failed to create DMG")
        raise RuntimeError("Failed to create DMG")


def create_dmg(
    app_path: Path,
    dmg_path: Path,
    volume_name: str = "BlockBrowser",
    pkg_dmg_path: Optional[Path] = None,
) -> bool:
    """Create a DMG package from an app bundle"""
    log_info(f"\n📀 Creating DMG package: {dmg_path.name}")

    # Verify app exists
    if not app_path.exists():
        log_error(f"App not found at: {app_path}")
        return False

    # Create DMG directory if needed
    dmg_path.parent.mkdir(parents=True, exist_ok=True)

    # Remove existing DMG if present
    if dmg_path.exists():
        log_info(f"  Removing existing DMG: {dmg_path.name}")
        dmg_path.unlink()

    # Build command
    cmd = []

    if pkg_dmg_path and pkg_dmg_path.exists():
        # Use Chromium's pkg-dmg tool if available
        cmd = [str(pkg_dmg_path)]
    else:
        # Fallback to system pkg-dmg if available
        pkg_dmg_system = shutil.which("pkg-dmg")
        if pkg_dmg_system:
            cmd = [pkg_dmg_system]
        else:
            log_error("No pkg-dmg tool found")
            return False

    cmd.extend(
        [
            "--sourcefile",
            "--source",
            str(app_path),
            "--target",
            str(dmg_path),
            "--volname",
            volume_name,
            "--symlink",
            "/Applications:/Applications",
            "--format",
            "UDBZ",
        ]
    )

    # Add verbosity for Chromium's pkg-dmg
    if pkg_dmg_path:
        cmd.extend(["--verbosity", "2"])

    try:
        run_command(cmd)
        log_success(f"DMG created: {dmg_path}")
        return True
    except Exception as e:
        log_error(f"Failed to create DMG: {e}")
        return False


def sign_dmg(dmg_path: Path, certificate_name: str) -> bool:
    """Sign a DMG file"""
    log_info(f"\n🔏 Signing DMG: {dmg_path.name}")

    if not dmg_path.exists():
        log_error(f"DMG not found at: {dmg_path}")
        return False

    try:
        run_command(
            [
                "codesign",
                "--sign",
                certificate_name,
                "--force",
                "--timestamp",
                str(dmg_path),
            ]
        )

        # Verify signature
        log_info("🔍 Verifying DMG signature...")
        run_command(["codesign", "-vvv", str(dmg_path)])

        log_success("DMG signed successfully")
        return True
    except Exception as e:
        log_error(f"Failed to sign DMG: {e}")
        return False


def notarize_dmg(dmg_path: Path, keychain_profile: str = "notarytool-profile") -> bool:
    """Notarize a DMG file"""
    log_info(f"\n📤 Notarizing DMG: {dmg_path.name}")

    if not dmg_path.exists():
        log_error(f"DMG not found at: {dmg_path}")
        return False

    try:
        # Submit for notarization
        log_info("📤 Submitting DMG for notarization (this may take a while)...")
        result = run_command(
            [
                "xcrun",
                "notarytool",
                "submit",
                str(dmg_path),
                "--keychain-profile",
                keychain_profile,
                "--wait",
            ],
            check=False,
        )

        log_info(result.stdout)
        if result.stderr:
            log_error(result.stderr)

        if result.returncode != 0:
            log_error("DMG notarization submission failed")
            return False

        # Check if accepted
        if "status: Accepted" not in result.stdout:
            log_error("DMG notarization failed - status was not 'Accepted'")
            # Try to extract submission ID for debugging
            for line in result.stdout.split("\n"):
                if "id:" in line:
                    submission_id = line.split("id:")[1].strip().split()[0]
                    log_info(
                        f'Get detailed logs with: xcrun notarytool log {submission_id} --keychain-profile "{keychain_profile}"'
                    )
                    break
            return False

        log_success("DMG notarization successful - status: Accepted")

        # Staple the ticket
        log_info("📎 Stapling notarization ticket to DMG...")
        result = run_command(["xcrun", "stapler", "staple", str(dmg_path)], check=False)

        if result.returncode != 0:
            log_error("Failed to staple notarization ticket to DMG")
            return False

        log_success("DMG notarization ticket stapled successfully")

        # Verify stapling
        log_info("🔍 Verifying DMG stapling...")
        result = run_command(
            ["xcrun", "stapler", "validate", str(dmg_path)], check=False
        )

        if result.returncode != 0:
            log_error("DMG stapling verification failed")
            return False

        log_success("DMG stapling verification successful")

        # Final security assessment
        log_info("🔍 Performing final security assessment...")
        result = run_command(
            [
                "spctl",
                "-a",
                "-vvv",
                "-t",
                "open",
                "--context",
                "context:primary-signature",
                str(dmg_path),
            ],
            check=False,
        )

        if result.returncode != 0:
            log_error("Final security assessment failed")
            return False

        log_success("Final security assessment passed")
        return True

    except Exception as e:
        log_error(f"Unexpected error during DMG notarization: {e}")
        return False


def create_signed_notarized_dmg(
    app_path: Path,
    dmg_path: Path,
    certificate_name: str,
    volume_name: str = "BlockBrowser",
    pkg_dmg_path: Optional[Path] = None,
    keychain_profile: str = "notarytool-profile",
) -> bool:
    """Create, sign, and notarize a DMG in one go"""
    log_info("=" * 70)
    log_info("📦 Creating signed and notarized DMG package")
    log_info("=" * 70)

    # Create DMG
    if not create_dmg(app_path, dmg_path, volume_name, pkg_dmg_path):
        return False

    # Sign DMG
    if not sign_dmg(dmg_path, certificate_name):
        return False

    # Notarize DMG
    if not notarize_dmg(dmg_path, keychain_profile):
        return False

    log_info("=" * 70)
    log_success(f"DMG package ready: {dmg_path}")
    log_info("=" * 70)
    return True


def package_universal(contexts: List[BuildContext]) -> bool:
    """Create DMG package for universal binary"""
    log_info("=" * 70)
    log_info("📦 Creating universal DMG package...")
    log_info("=" * 70)

    if len(contexts) < 2:
        log_error("Universal packaging requires at least 2 architectures")
        return False

    # Use the universal app path
    universal_dir = contexts[0].chromium_src / "out/Default_universal"
    universal_app_path = universal_dir / contexts[0].NXTSCAPE_APP_NAME

    if not universal_app_path.exists():
        log_error(f"Universal app not found: {universal_app_path}")
        return False

    # Create a temporary universal context for DMG naming
    universal_ctx = BuildContext(
        root_dir=contexts[0].root_dir,
        chromium_src=contexts[0].chromium_src,
        architecture="universal",
        build_type=contexts[0].build_type,
        apply_patches=False,
        sign_package=contexts[0].sign_package,
        package=False,
        build=False,
    )

    # Create DMG in dist/<version> directory
    dmg_dir = universal_ctx.get_dist_dir()
    dmg_dir.mkdir(parents=True, exist_ok=True)

    # Use context's DMG naming
    dmg_name = universal_ctx.get_dmg_name()
    dmg_path = dmg_dir / dmg_name

    # Get pkg-dmg tool
    pkg_dmg_path = contexts[0].get_pkg_dmg_path()

    # Create the universal DMG
    if create_dmg(universal_app_path, dmg_path, "BlockBrowser", pkg_dmg_path):
        log_success(f"Universal DMG created: {dmg_name}")
        return True
    else:
        log_error("Failed to create universal DMG")
        return False
