#!/usr/bin/env python3
"""DMG creation and packaging module for BrowserOS"""

import shutil
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Optional, List, Dict
from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.notarization import notarytool_wait_args
from ...lib.utils import (
    run_command,
    log_info,
    log_error,
    log_success,
    log_warning,
    IS_MACOS,
)


@dataclass(frozen=True)
class DmgAppearance:
    """Resolved art and layout for a laid-out install DMG.

    Icon positions are the centers Finder shows them at, in the window's
    640x420 point space, and must line up with the arrow drawn into the
    background art (see resources/<product>/dmg-assets/README.md).
    """

    background: Path  # 1x PNG; dmgbuild picks up a sibling <name>@2x.png for Retina
    volume_icon: Optional[Path] = None  # .icns for the mounted volume, or None
    app_icon_pos: tuple = (165, 190)
    apps_icon_pos: tuple = (475, 190)
    window_rect: tuple = ((200, 120), (640, 420))
    icon_size: int = 128
    text_size: int = 13


def resolve_dmg_appearance(ctx: Context) -> Optional[DmgAppearance]:
    """Return the DMG appearance for this product, or None to use a plain
    image. A product opts in by shipping a dmg-assets/background.png."""
    background = ctx.get_dmg_assets_dir() / "background.png"
    if not background.exists():
        return None

    volume_icon = (
        ctx.root_dir / "resources" / ctx.product.id / "icons" / "mac" / "app.icns"
    )
    return DmgAppearance(
        background=background,
        volume_icon=volume_icon if volume_icon.exists() else None,
    )


@step("package_macos", phase="package", platforms=("macos",))
class MacOSPackageModule(Step):
    produces = ["dmg"]
    requires = []
    description = "Create DMG package for macOS"

    def validate(self, ctx: Context) -> None:
        if not IS_MACOS():
            raise ValidationError("DMG creation requires macOS")

        app_path = ctx.get_app_path()
        if not app_path.exists():
            raise ValidationError(f"App not found: {app_path}")

    def execute(self, ctx: Context) -> None:
        log_info("\n📀 Creating DMG package...")

        app_path = ctx.get_app_path()
        dmg_dir = ctx.get_dist_dir()
        dmg_name = ctx.get_artifact_name("dmg")
        dmg_path = dmg_dir / dmg_name
        pkg_dmg_path = ctx.get_pkg_dmg_path()
        appearance = resolve_dmg_appearance(ctx)
        if appearance:
            log_info("  Using laid-out install window (drag to Applications)")
        else:
            log_info("  Using plain disk image (no DMG art for this product)")

        if ctx.artifact_registry.has("signed_app"):
            self._create_signed_notarized_dmg(
                app_path, dmg_path, pkg_dmg_path, ctx, appearance
            )
        else:
            self._create_dmg(app_path, dmg_path, pkg_dmg_path, ctx, appearance)

        ctx.artifact_registry.add("dmg", dmg_path)
        log_success(f"DMG created: {dmg_name}")

    def _create_dmg(
        self,
        app_path: Path,
        dmg_path: Path,
        pkg_dmg_path: Path,
        ctx: Context,
        appearance: Optional[DmgAppearance] = None,
    ) -> None:
        if not create_dmg(
            app_path,
            dmg_path,
            ctx.product.mac.dmg_volume_name,
            pkg_dmg_path,
            appearance,
        ):
            raise RuntimeError("Failed to create DMG")

    def _create_signed_notarized_dmg(
        self,
        app_path: Path,
        dmg_path: Path,
        pkg_dmg_path: Path,
        ctx: Context,
        appearance: Optional[DmgAppearance] = None,
    ) -> None:
        from ..sign.macos import check_environment

        env_ok, env_vars = check_environment(ctx.env)
        if not env_ok:
            raise ValidationError("Signing environment not configured")

        certificate_name = env_vars["certificate_name"]
        keychain_profile = env_vars.get("keychain_profile", "notarytool-profile")
        keychain_path = (
            Path(env_vars["keychain_path"]) if env_vars.get("keychain_path") else None
        )

        if not create_signed_notarized_dmg(
            app_path,
            dmg_path,
            certificate_name,
            ctx.product.mac.dmg_volume_name,
            pkg_dmg_path,
            keychain_profile,
            keychain_path,
            env_vars,
            appearance,
        ):
            raise RuntimeError("Failed to create signed and notarized DMG")


def create_dmg(
    app_path: Path,
    dmg_path: Path,
    volume_name: str = "BrowserOS",
    pkg_dmg_path: Optional[Path] = None,
    appearance: Optional[DmgAppearance] = None,
) -> bool:
    """Create a DMG package from an app bundle.

    With an appearance, lay out a branded install window via dmgbuild; a
    failure there is fatal rather than a silent drop to an unstyled image.
    Without one, fall back to Chromium's pkg-dmg (products with no DMG art).
    """
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

    if appearance is not None:
        if create_dmg_with_layout(app_path, dmg_path, volume_name, appearance):
            return True
        log_error("DMG layout failed; refusing to fall back to an unstyled image")
        return False

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


def create_dmg_with_layout(
    app_path: Path,
    dmg_path: Path,
    volume_name: str,
    appearance: DmgAppearance,
) -> bool:
    """Lay out a branded install DMG (background + icon positions) via dmgbuild.

    The app icon and the Applications alias are placed over the background art,
    the window size and icon view options are baked into the image's .DS_Store,
    and the format stays UDBZ to match the plain path. No Finder is involved, so
    this runs headless in CI.
    """
    log_info("  Laying out install window with dmgbuild")

    try:
        import dmgbuild  # type: ignore  # macOS-only build dependency, no stubs
    except ImportError:
        log_error("dmgbuild is not installed; cannot lay out the DMG")
        return False

    icon_name = app_path.name
    lines = [
        f"app_path = {str(app_path)!r}",
        f"icon_name = {icon_name!r}",
        f"background = {str(appearance.background)!r}",
    ]
    if appearance.volume_icon is not None:
        lines.append(f"icon = {str(appearance.volume_icon)!r}")
    lines += [
        'format = "UDBZ"',
        "files = [app_path]",
        'symlinks = {"Applications": "/Applications"}',
        "icon_locations = {",
        f"    icon_name: {tuple(appearance.app_icon_pos)!r},",
        f'    "Applications": {tuple(appearance.apps_icon_pos)!r},',
        "}",
        f"window_rect = {appearance.window_rect!r}",
        'default_view = "icon-view"',
        "show_status_bar = False",
        "show_tab_view = False",
        "show_toolbar = False",
        "show_pathbar = False",
        "show_sidebar = False",
        f"icon_size = {appearance.icon_size}",
        f"text_size = {appearance.text_size}",
    ]
    settings_src = "\n".join(lines) + "\n"

    settings_path: Optional[Path] = None
    try:
        with tempfile.NamedTemporaryFile("w", suffix=".py", delete=False) as handle:
            handle.write(settings_src)
            settings_path = Path(handle.name)

        dmgbuild.build_dmg(
            filename=str(dmg_path),
            volume_name=volume_name,
            settings_file=str(settings_path),
            lookForHiDPI=True,
        )
        log_success(f"DMG laid out: {dmg_path}")
        return True
    except Exception as e:
        log_error(f"Failed to lay out DMG with dmgbuild: {e}")
        return False
    finally:
        if settings_path is not None:
            settings_path.unlink(missing_ok=True)


def sign_dmg(
    dmg_path: Path,
    certificate_name: str,
    keychain_path: Optional[Path] = None,
) -> bool:
    """Sign a DMG file"""
    log_info(f"\n🔏 Signing DMG: {dmg_path.name}")

    if not dmg_path.exists():
        log_error(f"DMG not found at: {dmg_path}")
        return False

    try:
        cmd = [
            "codesign",
            "--sign",
            certificate_name,
            "--force",
            "--timestamp",
        ]
        if keychain_path:
            cmd.extend(["--keychain", str(keychain_path)])
        cmd.append(str(dmg_path))
        run_command(cmd)

        # Verify signature
        log_info("🔍 Verifying DMG signature...")
        run_command(["codesign", "-vvv", str(dmg_path)])

        log_success("DMG signed successfully")
        return True
    except Exception as e:
        log_error(f"Failed to sign DMG: {e}")
        return False


def notarize_dmg(
    dmg_path: Path,
    keychain_profile: str = "notarytool-profile",
    keychain_path: Optional[Path] = None,
    notarization_env: Optional[Dict[str, str]] = None,
) -> bool:
    """Notarize a DMG file"""
    log_info(f"\n📤 Notarizing DMG: {dmg_path.name}")

    if not dmg_path.exists():
        log_error(f"DMG not found at: {dmg_path}")
        return False

    try:
        # Submit for notarization
        log_info("📤 Submitting DMG for notarization (this may take a while)...")
        submit_cmd = [
            "xcrun",
            "notarytool",
            "submit",
            str(dmg_path),
            "--keychain-profile",
            keychain_profile,
            *notarytool_wait_args(),
        ]
        if keychain_path:
            submit_cmd.extend(["--keychain", str(keychain_path)])
        result = run_command(submit_cmd, check=False)

        if (
            result.returncode != 0
            and not keychain_path
            and notarization_env is not None
        ):
            log_warning("Keychain profile unavailable — passing credentials directly")
            result = run_command(
                [
                    "xcrun",
                    "notarytool",
                    "submit",
                    str(dmg_path),
                    "--apple-id",
                    notarization_env["apple_id"],
                    "--team-id",
                    notarization_env["team_id"],
                    "--password",
                    notarization_env["notarization_pwd"],
                    *notarytool_wait_args(),
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
    volume_name: str = "BrowserOS",
    pkg_dmg_path: Optional[Path] = None,
    keychain_profile: str = "notarytool-profile",
    keychain_path: Optional[Path] = None,
    notarization_env: Optional[Dict[str, str]] = None,
    appearance: Optional[DmgAppearance] = None,
) -> bool:
    """Create, sign, and notarize a DMG in one go"""
    log_info("=" * 70)
    log_info("📦 Creating signed and notarized DMG package")
    log_info("=" * 70)

    # Create DMG
    if not create_dmg(app_path, dmg_path, volume_name, pkg_dmg_path, appearance):
        return False

    # Sign DMG
    if not sign_dmg(dmg_path, certificate_name, keychain_path):
        return False

    # Notarize DMG
    if not notarize_dmg(dmg_path, keychain_profile, keychain_path, notarization_env):
        return False

    log_info("=" * 70)
    log_success(f"DMG package ready: {dmg_path}")
    log_info("=" * 70)
    return True


def package_universal(contexts: List[Context]) -> bool:
    """Create DMG package for universal binary"""
    log_info("=" * 70)
    log_info("📦 Creating universal DMG package...")
    log_info("=" * 70)

    if len(contexts) < 2:
        log_error("Universal packaging requires at least 2 architectures")
        return False

    # Use the universal app path
    product = contexts[0].product
    universal_ctx = Context(
        root_dir=contexts[0].root_dir,
        chromium_src=contexts[0].chromium_src,
        architecture="universal",
        build_type=contexts[0].build_type,
        product=product,
    )
    universal_dir = contexts[0].chromium_src / universal_ctx.out_dir
    universal_app_path = universal_dir / universal_ctx.BROWSEROS_APP_NAME

    if not universal_app_path.exists():
        log_error(f"Universal app not found: {universal_app_path}")
        return False

    # Create DMG in dist/<version> directory
    dmg_dir = universal_ctx.get_dist_dir()
    dmg_dir.mkdir(parents=True, exist_ok=True)

    # Use context's DMG naming
    dmg_name = universal_ctx.get_artifact_name("dmg")
    dmg_path = dmg_dir / dmg_name

    # Get pkg-dmg tool
    pkg_dmg_path = contexts[0].get_pkg_dmg_path()
    appearance = resolve_dmg_appearance(universal_ctx)

    # Create the universal DMG
    if create_dmg(
        universal_app_path,
        dmg_path,
        product.mac.dmg_volume_name,
        pkg_dmg_path,
        appearance,
    ):
        log_success(f"Universal DMG created: {dmg_name}")
        return True
    else:
        log_error("Failed to create universal DMG")
        return False
