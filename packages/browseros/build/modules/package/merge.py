#!/usr/bin/env python3
"""
Universal binary merge module for Nxtscape Browser
Provides functions to merge two architecture builds into a universal binary
"""

import sys
import shutil
from pathlib import Path
from ...common.context import Context
from ...common.products import ProductDescriptor
from ...common.utils import run_command, log_info, log_error, log_success


def merge_architectures(
    arch1_path: Path,
    arch2_path: Path,
    output_path: Path,
    universalizer_script: Path | None = None,
) -> bool:
    """
    Merge two architecture builds into a universal binary

    Args:
        arch1_path: Path to first architecture .app bundle
        arch2_path: Path to second architecture .app bundle
        output_path: Path where universal .app bundle should be created
        universalizer_script: Path to universalizer script (optional)

    Returns:
        True if successful, False otherwise
    """
    log_info("🔄 Merging architecture builds into universal binary...")

    # Validate input paths
    if not arch1_path.exists():
        log_error(f"Architecture 1 app not found: {arch1_path}")
        return False

    if not arch2_path.exists():
        log_error(f"Architecture 2 app not found: {arch2_path}")
        return False

    log_info(f"📱 Input 1: {arch1_path}")
    log_info(f"📱 Input 2: {arch2_path}")
    log_info(f"🎯 Output: {output_path}")

    # Find universalizer script
    if universalizer_script is None:
        # Try to find it in the same package module directory
        current_dir = Path(__file__).parent
        universalizer_script = current_dir / "universalizer_patched.py"

    if not universalizer_script.exists():
        log_error(f"Universalizer script not found: {universalizer_script}")
        return False

    # Create output directory if needed
    output_path.parent.mkdir(parents=True, exist_ok=True)

    # Remove existing output if present
    if output_path.exists():
        log_info(f"Removing existing output: {output_path}")
        shutil.rmtree(output_path)

    try:
        # Run universalizer
        cmd = [
            sys.executable,
            str(universalizer_script),
            str(arch1_path),
            str(arch2_path),
            str(output_path),
        ]

        log_info("Running universalizer...")
        log_info(f"Command: {' '.join(cmd)}")
        run_command(cmd)

        if output_path.exists():
            log_success(f"Universal binary created: {output_path}")
            return True
        else:
            log_error("Universal binary creation failed - output not found")
            return False

    except Exception as e:
        log_error(f"Failed to create universal binary: {e}")
        return False


def create_minimal_context(
    app_path: Path,
    chromium_src: Path,
    root_dir: Path,
    architecture: str = "universal",
    product: ProductDescriptor | None = None,
) -> Context:
    """Create a minimal BuildContext for signing/packaging operations"""

    out_dir_path = app_path.parent

    log_info(f"Creating context from app path: {app_path}")
    log_info(f"  Out dir: {out_dir_path}")
    log_info(f"  Chromium src: {chromium_src}")
    log_info(f"  Root dir: {root_dir}")

    if product is None:
        ctx = Context(
            root_dir=root_dir,
            chromium_src=chromium_src,
            architecture=architecture,
            build_type="release",
        )
    else:
        ctx = Context(
            root_dir=root_dir,
            chromium_src=chromium_src,
            architecture=architecture,
            build_type="release",
            product=product,
        )

    # Override out_dir to match the actual location
    ctx.out_dir = out_dir_path.name

    # Override get_app_path to return the actual app path for merge operations
    def get_app_path_override():
        return app_path

    ctx.get_app_path = get_app_path_override

    log_info(f"Context created with out_dir: {ctx.out_dir}")
    log_info(f"App path: {ctx.get_app_path()}")
    log_info(f"PKG-DMG path: {ctx.get_pkg_dmg_path()}")

    return ctx


def merge_sign_package(
    arch1_path: Path,
    arch2_path: Path,
    output_path: Path,
    chromium_src: Path,
    root_dir: Path,
    sign: bool = True,
    package: bool = True,
    universalizer_script: Path | None = None,
) -> bool:
    """
    Complete workflow: merge, sign, and package universal binary

    Args:
        arch1_path: Path to first architecture .app bundle
        arch2_path: Path to second architecture .app bundle
        output_path: Path where universal .app bundle should be created
        chromium_src: Path to chromium source directory
        root_dir: Path to project root directory
        sign: Whether to sign the universal binary
        package: Whether to create DMG package
        universalizer_script: Path to universalizer script (optional)

    Returns:
        True if successful, False otherwise
    """
    log_info("=" * 70)
    log_info("🚀 Starting merge, sign, and package workflow...")
    log_info("=" * 70)

    # Step 1: Merge architectures
    if not merge_architectures(
        arch1_path, arch2_path, output_path, universalizer_script
    ):
        return False

    # Step 2: Sign (if requested)
    if sign:
        log_info("\n" + "=" * 70)
        log_info("🔏 Signing universal binary...")
        log_info("=" * 70)

        try:
            from ..sign.macos import sign_app

            ctx = create_minimal_context(output_path, chromium_src, root_dir)
            if not sign_app(ctx, create_dmg=False):
                log_error("Failed to sign universal binary")
                return False

            log_success("Universal binary signed successfully!")

        except ImportError as e:
            log_error(f"Could not import signing module: {e}")
            return False
        except Exception as e:
            log_error(f"Signing failed: {e}")
            return False

    # Step 3: Package (if requested)
    if package:
        log_info("\n" + "=" * 70)
        log_info("📦 Creating DMG package...")
        log_info("=" * 70)

        try:
            from .macos import create_dmg

            ctx = create_minimal_context(output_path, chromium_src, root_dir)

            # Create DMG in parent directory
            dmg_dir = ctx.root_dir / "dmg"
            dmg_dir.mkdir(parents=True, exist_ok=True)

            dmg_name = ctx.get_artifact_name("dmg")

            dmg_path = dmg_dir / dmg_name
            pkg_dmg_path = ctx.get_pkg_dmg_path()

            # pkg-dmg should now be available since we enforce chromium-src path
            if not pkg_dmg_path.exists():
                log_error(f"Chromium pkg-dmg not found at: {pkg_dmg_path}")
                log_error("Make sure you provided the correct --chromium-src path")
                return False

            if create_dmg(
                output_path, dmg_path, ctx.product.mac.dmg_volume_name, pkg_dmg_path
            ):
                log_success(f"DMG created: {dmg_name}")
            else:
                log_error("Failed to create DMG")
                return False

        except ImportError as e:
            log_error(f"Could not import packaging module: {e}")
            return False
        except Exception as e:
            log_error(f"Packaging failed: {e}")
            return False

    log_info("\n" + "=" * 70)
    log_success("Merge, sign, and package workflow completed successfully!")
    log_info("=" * 70)

    return True


def handle_merge_command(
    arch1_path: Path,
    arch2_path: Path,
    chromium_src: Path,
    sign: bool = False,
    package: bool = False,
) -> bool:
    """
    Handle the merge command from CLI

    Args:
        arch1_path: Path to first architecture .app bundle
        arch2_path: Path to second architecture .app bundle
        chromium_src: Path to chromium source directory
        sign: Whether to sign the universal binary
        package: Whether to create DMG package

    Returns:
        True if successful, False otherwise
    """
    log_info("🔄 Running merge command...")
    log_info(f"  Arch 1: {arch1_path}")
    log_info(f"  Arch 2: {arch2_path}")
    log_info(f"  Sign: {sign}")
    log_info(f"  Package: {package}")
    log_info(f"📁 Using Chromium source: {chromium_src}")

    # Validate input paths exist
    if not arch1_path.exists():
        log_error(f"Architecture 1 app not found: {arch1_path}")
        return False

    if not arch2_path.exists():
        log_error(f"Architecture 2 app not found: {arch2_path}")
        return False

    # Auto-generate output path in chromium source
    # Get the app name from BuildContext
    from ...common.context import Context
    from ...common.paths import get_package_root

    root_dir = get_package_root()
    log_info(f"📂 Using root directory: {root_dir}")

    temp_ctx = Context(
        chromium_src=chromium_src,
        architecture="universal",
        build_type="release",
    )
    output_path = chromium_src / temp_ctx.out_dir / temp_ctx.BROWSEROS_APP_NAME
    log_info(f"  Output: {output_path} (auto-generated)")

    try:
        success = merge_sign_package(
            arch1_path=arch1_path,
            arch2_path=arch2_path,
            output_path=output_path,
            chromium_src=chromium_src,
            root_dir=root_dir,
            sign=sign,
            package=package,
        )

        if success:
            log_success("Merge command completed successfully!")
        else:
            log_error("Merge command failed!")

        return success
    except Exception as e:
        log_error(f"Merge command failed with exception: {e}")
        import traceback

        traceback.print_exc()
        return False
