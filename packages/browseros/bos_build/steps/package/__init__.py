"""Packaging steps (Windows, macOS, Linux).

Creates installer packages and distributables (DMG, exe, AppImage, deb, etc.)
from compiled browser outputs.
"""

from .linux import LinuxPackageModule
from .macos import MacOSPackageModule
from .windows import MiniInstallerModule, WindowsPackageModule

__all__ = [
    "LinuxPackageModule",
    "MacOSPackageModule",
    "MiniInstallerModule",
    "WindowsPackageModule",
]
