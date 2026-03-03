#!/usr/bin/env python3
"""Linux packaging module for BrowserOS (AppImage and .deb)"""

import os
import shutil
import subprocess
from pathlib import Path
from typing import List, Optional

from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import (
    log_info,
    log_error,
    log_warning,
    log_success,
    run_command,
    safe_rmtree,
    join_paths,
    IS_LINUX,
)
from ...common.notify import get_notifier, COLOR_GREEN


class LinuxPackageModule(CommandModule):
    produces = ["appimage", "deb"]
    requires = []
    description = "Create AppImage and .deb packages for Linux"

    def validate(self, ctx: Context) -> None:
        if not IS_LINUX():
            raise ValidationError("Linux packaging requires Linux")

        out_dir = join_paths(ctx.chromium_src, ctx.out_dir)
        chrome_binary = join_paths(out_dir, ctx.BROWSEROS_APP_NAME)

        if not chrome_binary.exists():
            raise ValidationError(f"Chrome binary not found: {chrome_binary}")

    def execute(self, ctx: Context) -> None:
        log_info(
            f"\n📦 Packaging {ctx.BROWSEROS_APP_BASE_NAME} {ctx.get_browseros_chromium_version()} for Linux ({ctx.architecture})"
        )

        package_dir = ctx.get_dist_dir()
        package_dir.mkdir(parents=True, exist_ok=True)

        appimage_path = self._package_appimage(ctx, package_dir)
        deb_path = self._package_deb(ctx, package_dir)

        if appimage_path:
            ctx.artifact_registry.add("appimage", appimage_path)
        if deb_path:
            ctx.artifact_registry.add("deb", deb_path)

        if not (appimage_path or deb_path):
            raise RuntimeError("Both AppImage and .deb packaging failed")

        log_success("✅ Linux packaging complete!")
        if appimage_path and deb_path:
            log_info("   Both AppImage and .deb created successfully")
        elif appimage_path:
            log_warning("   Only AppImage created (.deb failed)")
        elif deb_path:
            log_warning("   Only .deb created (AppImage failed)")

        # Send Slack notification
        notifier = get_notifier()
        artifacts = []
        if appimage_path:
            artifacts.append(appimage_path.name)
        if deb_path:
            artifacts.append(deb_path.name)
        notifier.notify(
            "📦 Package Created",
            f"Linux packages created successfully",
            {
                "Artifacts": ", ".join(artifacts),
                "Version": ctx.semantic_version,
            },
            color=COLOR_GREEN,
        )

    def _package_appimage(self, ctx: Context, package_dir: Path) -> Optional[Path]:
        return package_appimage(ctx, package_dir)

    def _package_deb(self, ctx: Context, package_dir: Path) -> Optional[Path]:
        return package_deb(ctx, package_dir)


# =============================================================================
# Shared Helper Functions (used by both AppImage and .deb)
# =============================================================================


def copy_browser_files(
    ctx: Context, target_dir: Path, set_sandbox_suid: bool = True
) -> bool:
    """Copy browser binaries, libraries, and resources to target directory.

    Args:
        ctx: Build context
        target_dir: Destination directory for browser files
        set_sandbox_suid: If True, set SUID bit on chrome_sandbox (AppImage only)

    Returns:
        True if successful, False otherwise
    """
    target_dir.mkdir(parents=True, exist_ok=True)
    out_dir = join_paths(ctx.chromium_src, ctx.out_dir)

    files_to_copy = [
        ctx.BROWSEROS_APP_NAME,
        "chrome_crashpad_handler",
        "chrome_sandbox",
        "chromedriver",
        "libEGL.so",
        "libGLESv2.so",
        "libvk_swiftshader.so",
        "libvulkan.so.1",
        "libqt5_shim.so",
        "libqt6_shim.so",
        "vk_swiftshader_icd.json",
        "icudtl.dat",
        "snapshot_blob.bin",
        "v8_context_snapshot.bin",
        "chrome_100_percent.pak",
        "chrome_200_percent.pak",
        "resources.pak",
    ]

    for file in files_to_copy:
        src = join_paths(out_dir, file)
        if Path(src).exists():
            shutil.copy2(src, join_paths(target_dir, file))
            log_info(f"  ✓ Copied {file}")
        else:
            log_warning(f"  ⚠ File not found: {file}")

    dirs_to_copy = ["locales", "MEIPreload", "BrowserOSServer"]
    for dir_name in dirs_to_copy:
        src = join_paths(out_dir, dir_name)
        if Path(src).exists():
            shutil.copytree(src, join_paths(target_dir, dir_name), dirs_exist_ok=True)
            log_info(f"  ✓ Copied {dir_name}/")

    browseros_path = Path(join_paths(target_dir, ctx.BROWSEROS_APP_NAME))
    if browseros_path.exists():
        browseros_path.chmod(0o755)

    sandbox_path = Path(join_paths(target_dir, "chrome_sandbox"))
    if sandbox_path.exists():
        if set_sandbox_suid:
            sandbox_path.chmod(0o4755)
        else:
            sandbox_path.chmod(0o755)

    crashpad_path = Path(join_paths(target_dir, "chrome_crashpad_handler"))
    if crashpad_path.exists():
        crashpad_path.chmod(0o755)

    return True


def create_desktop_file(apps_dir: Path, exec_path: str) -> Path:
    """Create .desktop file with specified Exec path.

    Args:
        apps_dir: Directory where .desktop file should be created
        exec_path: Full path for Exec= line in desktop file

    Returns:
        Path to created .desktop file
    """
    apps_dir.mkdir(parents=True, exist_ok=True)

    desktop_content = f"""[Desktop Entry]
Version=1.0
Name=BrowserOS
GenericName=Web Browser
Comment=Browse the World Wide Web
Exec={exec_path} %U
Terminal=false
Type=Application
Categories=Network;WebBrowser;
MimeType=text/html;text/xml;application/xhtml+xml;application/xml;application/vnd.mozilla.xul+xml;application/rss+xml;application/rdf+xml;image/gif;image/jpeg;image/png;x-scheme-handler/http;x-scheme-handler/https;x-scheme-handler/ftp;x-scheme-handler/chrome;video/webm;application/x-xpinstall;
Icon=browseros
StartupWMClass=chromium-browser
"""

    desktop_file = Path(join_paths(apps_dir, "browseros.desktop"))
    desktop_file.write_text(desktop_content)
    log_info("  ✓ Created desktop file")
    return desktop_file


def copy_icon(ctx: Context, icons_dir: Path) -> bool:
    """Copy product icons at multiple sizes to hicolor icon directory.

    Args:
        ctx: Build context
        icons_dir: Base icons directory (usr/share/icons/hicolor)

    Returns:
        True if at least one icon was copied, False if none found
    """
    icons_base = Path(join_paths(ctx.root_dir, "resources", "icons"))
    copied = False

    for size in [16, 22, 24, 32, 48, 64, 128, 256]:
        icon_src = Path(join_paths(icons_base, f"product_logo_{size}.png"))
        if icon_src.exists():
            icon_dest = Path(join_paths(icons_dir, f"{size}x{size}", "apps", "browseros.png"))
            icon_dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(icon_src, icon_dest)
            copied = True

    if copied:
        log_info("  ✓ Copied icons (multiple sizes)")
    else:
        log_warning("  ⚠ No icon files found in resources/icons/")

    return copied


# =============================================================================
# AppImage Packaging Functions
# =============================================================================


def prepare_appdir(ctx: Context, appdir: Path) -> bool:
    """Prepare the AppDir structure for AppImage"""
    log_info("📁 Preparing AppDir structure...")

    app_root = join_paths(appdir, "opt", "browseros")
    usr_share = join_paths(appdir, "usr", "share")
    icons_dir = join_paths(usr_share, "icons", "hicolor")
    apps_dir = join_paths(usr_share, "applications")

    # Copy browser files (with SUID on chrome_sandbox for AppImage)
    if not copy_browser_files(ctx, app_root, set_sandbox_suid=True):
        return False

    # Create desktop file
    desktop_file = create_desktop_file(
        apps_dir, f"/opt/browseros/{ctx.BROWSEROS_APP_NAME}"
    )

    # Copy icons (multiple sizes)
    copy_icon(ctx, icons_dir)

    # AppImage-specific: Copy desktop file to root and update Exec line
    appdir_desktop = Path(join_paths(appdir, "browseros.desktop"))
    shutil.copy2(desktop_file, appdir_desktop)
    desktop_content = appdir_desktop.read_text()
    desktop_content = desktop_content.replace(
        f"Exec=/opt/browseros/{ctx.BROWSEROS_APP_NAME} %U", "Exec=AppRun %U"
    )
    appdir_desktop.write_text(desktop_content)

    # AppImage-specific: Copy icon to root (256px for best quality)
    icon_src = Path(join_paths(ctx.root_dir, "resources", "icons", "product_logo_256.png"))
    if not icon_src.exists():
        icon_src = Path(join_paths(ctx.root_dir, "resources", "icons", "product_logo.png"))
    if icon_src.exists():
        appdir_icon = Path(join_paths(appdir, "browseros.png"))
        shutil.copy2(icon_src, appdir_icon)

    # AppImage-specific: Create AppRun script
    apprun_content = f"""#!/bin/sh
THIS="$(readlink -f "${{0}}")"
HERE="$(dirname "${{THIS}}")"
export LD_LIBRARY_PATH="${{HERE}}"/opt/browseros:$LD_LIBRARY_PATH
export CHROME_WRAPPER="${{THIS}}"
"${{HERE}}"/opt/browseros/{ctx.BROWSEROS_APP_NAME} "$@"
"""

    apprun_file = Path(join_paths(appdir, "AppRun"))
    apprun_file.write_text(apprun_content)
    apprun_file.chmod(0o755)
    log_info("  ✓ Created AppRun script")

    return True


def download_appimagetool(ctx: Context) -> Optional[Path]:
    """Download appimagetool if not available"""
    tool_dir = Path(join_paths(ctx.root_dir, "build", "tools"))
    tool_dir.mkdir(exist_ok=True)

    tool_path = Path(join_paths(tool_dir, "appimagetool-x86_64.AppImage"))

    if tool_path.exists():
        log_info("✓ appimagetool already available")
        return tool_path

    log_info("📥 Downloading appimagetool...")
    url = "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage"

    cmd = ["wget", "-O", str(tool_path), url]
    result = run_command(cmd, check=False)

    if result.returncode == 0:
        tool_path.chmod(0o755)
        log_success("✓ Downloaded appimagetool")
        return tool_path
    else:
        log_error("Failed to download appimagetool")
        return None


def create_appimage(ctx: Context, appdir: Path, output_path: Path) -> bool:
    """Create AppImage from AppDir"""
    log_info("📦 Creating AppImage...")

    # Download appimagetool if needed
    appimagetool = download_appimagetool(ctx)
    if not appimagetool:
        return False

    # Set architecture environment variable (required by appimagetool)
    arch = "x86_64" if ctx.architecture == "x64" else "aarch64"

    # Create AppImage with ARCH env var set for this command only
    cmd = [
        str(appimagetool),
        "--comp",
        "gzip",  # Use gzip compression
        str(appdir),
        str(output_path),
    ]

    # Pass ARCH as environment variable to the subprocess
    env = os.environ.copy()
    env["ARCH"] = arch

    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        env=env,
        check=False
    )

    if result.returncode == 0:
        log_success(f"✓ Created AppImage: {output_path}")
        # Make executable
        output_path.chmod(0o755)
        return True
    else:
        log_error("Failed to create AppImage")
        if result.stderr:
            log_error(result.stderr)
        return False


# =============================================================================
# Debian Package (.deb) Functions
# =============================================================================


def create_launcher_script(ctx: Context, bin_dir: Path) -> None:
    """Create launcher script in /usr/bin/browseros."""
    bin_dir.mkdir(parents=True, exist_ok=True)

    launcher_content = f"""#!/bin/sh
# BrowserOS launcher script
export LD_LIBRARY_PATH=/usr/lib/browseros:$LD_LIBRARY_PATH
exec /usr/lib/browseros/{ctx.BROWSEROS_APP_NAME} "$@"
"""

    launcher_path = Path(join_paths(bin_dir, "browseros"))
    launcher_path.write_text(launcher_content)
    launcher_path.chmod(0o755)
    log_info("  ✓ Created launcher script")


def create_control_file(ctx: Context, debian_dir: Path) -> None:
    """Create DEBIAN/control file with package metadata."""
    debian_dir.mkdir(parents=True, exist_ok=True)

    # Version formatting: strip 'v' prefix and spaces, ensure numeric
    version = ctx.get_browseros_chromium_version()
    version = version.lstrip("v").replace(" ", "").replace("_", ".")

    # Architecture mapping
    deb_arch = "amd64" if ctx.architecture == "x64" else "arm64"

    control_content = f"""Package: browseros
Version: {version}
Section: web
Priority: optional
Architecture: {deb_arch}
Depends: libc6 (>= 2.31), libglib2.0-0, libnss3, libnspr4, libx11-6, libatk1.0-0, libatk-bridge2.0-0, libcups2, libasound2, libdrm2, libgbm1, libpango-1.0-0, libcairo2, libudev1, libxcomposite1, libxdamage1, libxrandr2, libxkbcommon0, libgtk-3-0
Provides: www-browser, gnome-www-browser
Recommends: apparmor
Maintainer: BrowserOS Team <support@browseros.com>
Homepage: https://www.browseros.com/
Description: BrowserOS - The open source agentic browser
 BrowserOS is a privacy-focused web browser built on Chromium,
 designed for modern web browsing with AI capabilities.
"""

    control_path = Path(join_paths(debian_dir, "control"))
    control_path.write_text(control_content)
    log_info("  ✓ Created DEBIAN/control")


def create_postinst_script(debian_dir: Path) -> None:
    """Create DEBIAN/postinst script for sandbox, AppArmor, and alternatives."""
    postinst_content = """#!/bin/sh
# Post-installation script for BrowserOS
set -e

# Set SUID bit on chrome_sandbox for sandboxing support
if [ -f /usr/lib/browseros/chrome_sandbox ]; then
    chmod 4755 /usr/lib/browseros/chrome_sandbox
fi

# Load AppArmor profile (required for Ubuntu 23.10+ user namespace restrictions)
if [ -d /etc/apparmor.d ] && command -v apparmor_parser >/dev/null 2>&1; then
    apparmor_parser -r -T -W /etc/apparmor.d/browseros 2>/dev/null || true
fi

# Register as a selectable default browser
if [ "$1" = "configure" ]; then
    update-alternatives --install /usr/bin/x-www-browser x-www-browser /usr/bin/browseros 40
    update-alternatives --install /usr/bin/gnome-www-browser gnome-www-browser /usr/bin/browseros 40
fi

exit 0
"""

    postinst_path = Path(join_paths(debian_dir, "postinst"))
    postinst_path.write_text(postinst_content)
    postinst_path.chmod(0o755)
    log_info("  ✓ Created DEBIAN/postinst")


def create_prerm_script(debian_dir: Path) -> None:
    """Create DEBIAN/prerm script to clean up on removal."""
    prerm_content = """#!/bin/sh
# Pre-removal script for BrowserOS
set -e

# Unregister as default browser
if [ "$1" = "remove" ] || [ "$1" = "deconfigure" ]; then
    update-alternatives --remove x-www-browser /usr/bin/browseros 2>/dev/null || true
    update-alternatives --remove gnome-www-browser /usr/bin/browseros 2>/dev/null || true
fi

# Unload AppArmor profile before files are removed
if command -v apparmor_parser >/dev/null 2>&1 && [ -f /etc/apparmor.d/browseros ]; then
    apparmor_parser -R /etc/apparmor.d/browseros 2>/dev/null || true
fi

exit 0
"""

    prerm_path = Path(join_paths(debian_dir, "prerm"))
    prerm_path.write_text(prerm_content)
    prerm_path.chmod(0o755)
    log_info("  ✓ Created DEBIAN/prerm")


def create_apparmor_profile(ctx: Context, apparmor_dir: Path) -> None:
    """Create AppArmor profile that permits unprivileged user namespaces.

    Ubuntu 23.10+ restricts unprivileged user namespaces via AppArmor.
    Without this profile, the Chromium sandbox cannot create namespaces
    and the browser fatally crashes on launch (see GitHub issue #165).
    """
    apparmor_dir.mkdir(parents=True, exist_ok=True)

    profile_content = f"""# AppArmor profile for BrowserOS
# This profile allows everything and only exists to give the application
# a name instead of having the label "unconfined", and to grant permission
# to create unprivileged user namespaces (required for Chromium sandbox on
# Ubuntu 23.10+ and other distros that restrict userns via AppArmor).

abi <abi/4.0>,
include <tunables/global>

profile browseros /usr/lib/browseros/{ctx.BROWSEROS_APP_NAME} flags=(unconfined) {{
  userns,

  include if exists <local/browseros>
}}
"""

    profile_path = Path(join_paths(apparmor_dir, "browseros"))
    profile_path.write_text(profile_content)
    log_info("  ✓ Created AppArmor profile")


def create_metainfo_file(ctx: Context, metainfo_dir: Path) -> None:
    """Create AppStream metainfo file for software center discoverability.

    Installs to /usr/share/metainfo/ so GNOME Software, KDE Discover,
    and other AppStream-aware tools can display BrowserOS in their catalogs.
    """
    metainfo_dir.mkdir(parents=True, exist_ok=True)

    version = ctx.get_browseros_chromium_version()
    version = version.lstrip("v").replace(" ", "").replace("_", ".")

    metainfo_content = f"""<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>browseros.desktop</id>
  <launchable type="desktop-id">browseros.desktop</launchable>
  <name>BrowserOS</name>
  <developer id="com.browseros">
    <name>BrowserOS Team</name>
  </developer>
  <summary>The open source agentic browser</summary>
  <metadata_license>CC0-1.0</metadata_license>
  <project_license>BSD-3-Clause and LGPL-2.1+ and Apache-2.0 and IJG and MIT and GPL-2.0+ and ISC and OpenSSL and (MPL-1.1 or GPL-2.0 or LGPL-2.0)</project_license>
  <url type="homepage">https://www.browseros.com/</url>
  <url type="bugtracker">https://github.com/browseros-ai/BrowserOS/issues</url>
  <url type="help">https://docs.browseros.com/</url>
  <description>
    <p>
      BrowserOS is a privacy-focused web browser built on Chromium,
      designed for modern web browsing with AI capabilities.
    </p>
    <p>
      Browse the web with built-in agentic AI features that help you
      automate tasks and interact with web pages intelligently.
    </p>
  </description>
  <categories>
    <category>Network</category>
    <category>WebBrowser</category>
  </categories>
  <keywords>
    <keyword>web browser</keyword>
    <keyword>chromium</keyword>
    <keyword>ai</keyword>
    <keyword>agentic</keyword>
    <keyword>privacy</keyword>
  </keywords>
  <releases>
    <release version="{version}" />
  </releases>
  <content_rating type="oars-1.1" />
</component>
"""

    metainfo_path = Path(join_paths(metainfo_dir, "browseros.metainfo.xml"))
    metainfo_path.write_text(metainfo_content)
    log_info("  ✓ Created AppStream metainfo")


def prepare_debdir(ctx: Context, debdir: Path) -> bool:
    """Prepare directory structure for .deb package.

    Structure:
    debdir/
    ├── DEBIAN/
    │   ├── control
    │   ├── postinst
    │   └── prerm
    ├── etc/
    │   └── apparmor.d/
    │       └── browseros
    ├── usr/
    │   ├── bin/
    │   │   └── browseros (launcher script)
    │   ├── lib/browseros/
    │   │   └── [all browser files]
    │   └── share/
    │       ├── applications/browseros.desktop
    │       ├── icons/hicolor/{16..256}x{16..256}/apps/browseros.png
    │       └── metainfo/browseros.metainfo.xml
    """
    log_info("📁 Preparing .deb directory structure...")

    lib_dir = join_paths(debdir, "usr", "lib", "browseros")
    bin_dir = join_paths(debdir, "usr", "bin")
    share_dir = join_paths(debdir, "usr", "share")
    apps_dir = join_paths(share_dir, "applications")
    icons_dir = join_paths(share_dir, "icons", "hicolor")
    metainfo_dir = join_paths(share_dir, "metainfo")
    debian_dir = join_paths(debdir, "DEBIAN")
    apparmor_dir = join_paths(debdir, "etc", "apparmor.d")

    # Copy browser files (without SUID, will be set in postinst)
    if not copy_browser_files(ctx, lib_dir, set_sandbox_suid=False):
        return False

    # Create launcher script in /usr/bin/
    create_launcher_script(ctx, bin_dir)

    # Create desktop file
    create_desktop_file(apps_dir, "/usr/bin/browseros")

    # Copy icons (multiple sizes for hicolor theme)
    copy_icon(ctx, icons_dir)

    # Create AppStream metainfo for software center discoverability
    create_metainfo_file(ctx, metainfo_dir)

    # Install AppArmor profile (fixes crash on Ubuntu 23.10+)
    create_apparmor_profile(ctx, apparmor_dir)

    # Create DEBIAN metadata files
    create_control_file(ctx, debian_dir)
    create_postinst_script(debian_dir)
    create_prerm_script(debian_dir)

    log_success("✓ .deb directory prepared")
    return True


def create_deb(ctx: Context, debdir: Path, output_path: Path) -> bool:
    """Build .deb package using dpkg-deb."""
    log_info("📦 Creating .deb package...")

    # Verify dpkg-deb is available
    if not shutil.which("dpkg-deb"):
        log_error("dpkg-deb not found. Install with: sudo apt install dpkg")
        return False

    cmd = [
        "dpkg-deb",
        "--build",
        "--root-owner-group",  # Ensure files owned by root:root
        str(debdir),
        str(output_path),
    ]

    result = run_command(cmd, check=False)

    if result.returncode == 0:
        log_success(f"✓ Created .deb package: {output_path}")
        output_path.chmod(0o644)  # Standard package permissions
        return True
    else:
        log_error("Failed to create .deb package")
        return False


# =============================================================================
# Main Packaging Entry Points
# =============================================================================


def package_appimage(ctx: Context, package_dir: Path) -> Optional[Path]:
    """Create AppImage package.

    Returns:
        Path to created AppImage, or None if failed
    """
    log_info("🖼️  Building AppImage...")

    appdir = Path(join_paths(package_dir, f"{ctx.BROWSEROS_APP_BASE_NAME}.AppDir"))
    if appdir.exists():
        safe_rmtree(appdir)

    if not prepare_appdir(ctx, appdir):
        safe_rmtree(appdir)
        return None

    filename = ctx.get_artifact_name("appimage")
    output_path = Path(join_paths(package_dir, filename))

    success = create_appimage(ctx, appdir, output_path)
    safe_rmtree(appdir)

    if success:
        log_success(f"✅ AppImage created: {output_path.name}")
        log_info(f"   Size: {output_path.stat().st_size / 1024 / 1024:.1f} MB")
        return output_path

    return None


def package_deb(ctx: Context, package_dir: Path) -> Optional[Path]:
    """Create .deb package.

    Returns:
        Path to created .deb, or None if failed
    """
    log_info("📦 Building .deb package...")

    debdir = Path(join_paths(package_dir, f"{ctx.BROWSEROS_APP_BASE_NAME}_deb"))
    if debdir.exists():
        safe_rmtree(debdir)

    if not prepare_debdir(ctx, debdir):
        safe_rmtree(debdir)
        return None

    filename = ctx.get_artifact_name("deb")
    output_path = Path(join_paths(package_dir, filename))

    success = create_deb(ctx, debdir, output_path)
    safe_rmtree(debdir)

    if success:
        log_success(f"✅ .deb package created: {output_path.name}")
        log_info(f"   Size: {output_path.stat().st_size / 1024 / 1024:.1f} MB")
        return output_path

    return None
def package_universal(contexts: List[Context]) -> bool:
    """Linux doesn't support universal binaries"""
    log_warning("Universal binaries are not supported on Linux")
    return False


# Sign functions moved to sign/linux.py
# - sign_binaries()
# These are now in modules/sign/linux.py
