#!/usr/bin/env python3
"""
Universal Build Module - Build, sign, package, and upload universal binary for macOS

This module orchestrates building both architectures (arm64 + x64), signing each,
packaging each into DMGs, uploading each, then merging into a universal binary
and signing/packaging/uploading that as well.

Design:
    For each arch (arm64, x64):
        1. resources -> configure -> compile
        2. sign -> package -> upload

    Then:
        3. Merge arm64 + x64 into universal
        4. sign universal -> package -> upload

    Output: 3 DMGs uploaded:
        - BrowserOS_{version}_arm64_signed.dmg
        - BrowserOS_{version}_x64_signed.dmg
        - BrowserOS_{version}_universal_signed.dmg

Prerequisites (must run BEFORE this module):
    - clean (optional)
    - git_setup
    - sparkle_setup (macOS)
    - chromium_replace
    - string_replaces
    - patches

This module internally runs (for EACH architecture):
    - resources (arch-specific binaries)
    - configure (GN configuration)
    - compile (ninja build)
    - sign_macos (code signing + notarization)
    - package_macos (DMG creation)
    - upload (artifact upload)

Then merges and processes the universal binary.
"""

from pathlib import Path

from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import log_info, log_success, log_warning, IS_MACOS

# Architectures to build for universal binary
UNIVERSAL_ARCHITECTURES = ["arm64", "x64"]


class UniversalBuildModule(CommandModule):
    """Build, sign, package, and upload universal binary (arm64 + x64) for macOS

    This module handles the complete multi-architecture build, sign, package,
    and upload workflow. It internally creates separate contexts for arm64 and x64,
    builds each, signs each, packages each into DMGs, uploads each, then merges
    them into a universal binary and processes that as well.

    The base context passed to this module can have any architecture value -
    it will be ignored and arm64/x64 will be built explicitly.

    Output artifacts:
        - BrowserOS_{version}_arm64_signed.dmg
        - BrowserOS_{version}_x64_signed.dmg
        - BrowserOS_{version}_universal_signed.dmg
    """

    produces = ["dmg_arm64", "dmg_x64", "dmg_universal"]
    requires = []
    description = (
        "Build, sign, package, and upload universal binary (arm64 + x64) for macOS"
    )

    def validate(self, ctx: Context) -> None:
        """Validate universal build can run"""
        if not IS_MACOS():
            raise ValidationError("Universal builds only supported on macOS")

        # Check universalizer script exists
        universalizer = ctx.root_dir / "build/modules/package/universalizer_patched.py"
        if not universalizer.exists():
            raise ValidationError(f"Universalizer script not found: {universalizer}")

        # Fail fast: check signing environment is configured
        from ..sign.macos import check_signing_environment

        if not check_signing_environment():
            raise ValidationError(
                "Signing environment not configured. "
                "Required: MACOS_CERTIFICATE_NAME, notarization credentials"
            )

    def execute(self, ctx: Context) -> None:
        """Build arm64 + x64, sign/package/upload each, then merge and process universal"""

        log_info("\n" + "=" * 70)
        log_info("🔄 Universal Build Mode (Full Pipeline)")
        log_info("Building arm64 + x64, signing, packaging, uploading each...")
        log_info("Then merging into universal and processing that too.")
        log_info("=" * 70)

        # Import build modules
        from ..resources.resources import ResourcesModule
        from ..setup.configure import ConfigureModule
        from .standard import CompileModule

        # Import sign/package/upload modules
        from ..sign.macos import MacOSSignModule
        from ..package.macos import MacOSPackageModule
        from ..storage import UploadModule

        # Clean all build directories before starting
        self._clean_build_directories(ctx)

        built_apps = []

        # Build + Sign + Package + Upload each architecture
        for arch in UNIVERSAL_ARCHITECTURES:
            log_info("\n" + "=" * 70)
            log_info(f"🏗️  Processing architecture: {arch}")
            log_info("=" * 70)

            # Create architecture-specific context with fixed app path
            arch_ctx = self._create_arch_context(ctx, arch)

            log_info(f"📍 Chromium: {arch_ctx.chromium_version}")
            log_info(f"📍 BrowserOS: {arch_ctx.browseros_build_offset}")
            log_info(f"📍 Output directory: {arch_ctx.out_dir}")

            # === BUILD PHASE ===
            # Copy resources (arch-specific binaries like browseros_server, codex)
            log_info(f"\n📦 Copying resources for {arch}...")
            ResourcesModule().execute(arch_ctx)

            # Configure build (GN gen)
            log_info(f"\n🔧 Configuring {arch}...")
            ConfigureModule().execute(arch_ctx)

            # Compile (ninja)
            log_info(f"\n🏗️  Compiling {arch}...")
            CompileModule().execute(arch_ctx)

            # Get app path for this architecture
            app_path = arch_ctx.get_app_path()

            if not app_path.exists():
                raise RuntimeError(f"Build failed - app not found: {app_path}")

            log_success(f"✅ {arch} build complete: {app_path}")
            built_apps.append(app_path)

            # === SIGN PHASE ===
            log_info(f"\n🔏 Signing {arch} build...")
            MacOSSignModule().execute(arch_ctx)
            log_success(f"✅ {arch} signing complete")

            # === PACKAGE PHASE ===
            log_info(f"\n📦 Packaging {arch} build...")
            MacOSPackageModule().execute(arch_ctx)
            log_success(f"✅ {arch} packaging complete")

            # === UPLOAD PHASE ===
            log_info(f"\n☁️  Uploading {arch} artifacts...")
            try:
                UploadModule().execute(arch_ctx)
                log_success(f"✅ {arch} upload complete")
            except Exception as e:
                log_warning(f"⚠️  {arch} upload failed (non-fatal): {e}")

        # === MERGE INTO UNIVERSAL ===
        log_info("\n" + "=" * 70)
        log_info("🔄 Merging into universal binary...")
        log_info("=" * 70)

        self._merge_universal(ctx, built_apps[0], built_apps[1])

        # Verify universal binary was created
        universal_app = ctx.chromium_src / "out/Default_universal/BrowserOS.app"
        if not universal_app.exists():
            raise RuntimeError(f"Universal binary not found: {universal_app}")

        log_success(f"✅ Universal binary created: {universal_app}")

        # === SIGN + PACKAGE + UPLOAD UNIVERSAL ===
        log_info("\n" + "=" * 70)
        log_info("🔏 Processing universal binary...")
        log_info("=" * 70)

        universal_ctx = self._create_universal_context(ctx)

        # Sign universal
        log_info("\n🔏 Signing universal build...")
        MacOSSignModule().execute(universal_ctx)
        log_success("✅ Universal signing complete")

        # Package universal
        log_info("\n📦 Packaging universal build...")
        MacOSPackageModule().execute(universal_ctx)
        log_success("✅ Universal packaging complete")

        # Upload universal
        log_info("\n☁️  Uploading universal artifacts...")
        try:
            UploadModule().execute(universal_ctx)
            log_success("✅ Universal upload complete")
        except Exception as e:
            log_warning(f"⚠️  Universal upload failed (non-fatal): {e}")

        log_info("\n" + "=" * 70)
        log_success("✅ Universal build pipeline complete!")
        log_info("Artifacts created:")
        log_info(
            f"  - arm64 DMG: {ctx.get_dist_dir() / ctx.get_artifact_name('dmg').replace('universal', 'arm64')}"
        )
        log_info(
            f"  - x64 DMG: {ctx.get_dist_dir() / ctx.get_artifact_name('dmg').replace('universal', 'x64')}"
        )
        log_info(
            f"  - universal DMG: {ctx.get_dist_dir() / universal_ctx.get_artifact_name('dmg')}"
        )
        log_info("=" * 70)

    def _clean_build_directories(self, ctx: Context) -> None:
        """Clean architecture-specific and universal build directories

        Args:
            ctx: Base context
        """
        from ...common.utils import safe_rmtree

        log_info("\n🧹 Cleaning build directories...")

        # Clean architecture-specific directories
        for arch in UNIVERSAL_ARCHITECTURES:
            arch_dir = ctx.chromium_src / f"out/Default_{arch}"
            if arch_dir.exists():
                log_info(f"  Removing {arch_dir}")
                safe_rmtree(arch_dir)

        # Clean universal directory
        universal_dir = ctx.chromium_src / "out/Default_universal"
        if universal_dir.exists():
            log_info(f"  Removing {universal_dir}")
            safe_rmtree(universal_dir)

        log_success("✅ Build directories cleaned")

    def _create_arch_context(self, base_ctx: Context, arch: str) -> Context:
        """Create a new context for a specific architecture

        Args:
            base_ctx: Base context with common settings
            arch: Architecture to build (arm64 or x64)

        Returns:
            New Context object with architecture set and fixed app path
            to prevent universal auto-detection
        """
        ctx = Context(
            root_dir=base_ctx.root_dir,
            chromium_src=base_ctx.chromium_src,
            architecture=arch,
            build_type=base_ctx.build_type,
        )
        # Pin the app path for this arch's context
        ctx._fixed_app_path = (
            ctx.chromium_src / f"out/Default_{arch}" / ctx.BROWSEROS_APP_NAME
        )
        return ctx

    def _create_universal_context(self, base_ctx: Context) -> Context:
        """Create a new context for the universal binary

        Args:
            base_ctx: Base context with common settings

        Returns:
            New Context object configured for universal binary
        """
        ctx = Context(
            root_dir=base_ctx.root_dir,
            chromium_src=base_ctx.chromium_src,
            architecture="universal",
            build_type=base_ctx.build_type,
        )
        # Set fixed app path to the universal binary
        ctx._fixed_app_path = (
            ctx.chromium_src / "out/Default_universal" / ctx.BROWSEROS_APP_NAME
        )
        # Override out_dir for universal
        ctx.out_dir = "out/Default_universal"
        return ctx

    def _merge_universal(
        self,
        ctx: Context,
        arm64_app: Path,
        x64_app: Path,
    ) -> None:
        """Merge arm64 + x64 into universal binary

        Args:
            ctx: Base context
            arm64_app: Path to arm64 .app bundle
            x64_app: Path to x64 .app bundle

        Raises:
            RuntimeError: If merge fails
        """
        # Use existing merge helper
        from ..package.merge import merge_architectures

        # Prepare output path
        universal_dir = ctx.chromium_src / "out/Default_universal"

        # Create universal directory (already cleaned in _clean_build_directories)
        universal_dir.mkdir(parents=True, exist_ok=True)
        universal_app = universal_dir / "BrowserOS.app"

        # Find universalizer script
        universalizer_script = (
            ctx.root_dir / "build/modules/package/universalizer_patched.py"
        )

        log_info(f"📱 Input 1 (arm64): {arm64_app}")
        log_info(f"📱 Input 2 (x64): {x64_app}")
        log_info(f"🎯 Output (universal): {universal_app}")
        log_info(f"🔧 Universalizer: {universalizer_script}")

        # Merge the architectures
        success = merge_architectures(
            arch1_path=arm64_app,
            arch2_path=x64_app,
            output_path=universal_app,
            universalizer_script=universalizer_script,
        )

        if not success:
            raise RuntimeError("Failed to merge architectures into universal binary")
