#!/usr/bin/env python3
"""OTA (Over-The-Air) update modules for BrowserOS Server and Browser"""

from .common import (
    sparkle_sign_file,
    generate_server_appcast,
    parse_existing_appcast,
    ExistingAppcast,
    SignedArtifact,
    SERVER_PLATFORMS,
    APPCAST_TEMPLATE,
)
from .sign_binary import (
    sign_macos_binary,
    notarize_macos_binary,
    sign_windows_binary,
)
from .server import ServerOTAModule

AVAILABLE_MODULES = {
    "server_ota": ServerOTAModule,
}

__all__ = [
    "AVAILABLE_MODULES",
    "ServerOTAModule",
    "sparkle_sign_file",
    "generate_server_appcast",
    "parse_existing_appcast",
    "ExistingAppcast",
    "SignedArtifact",
    "sign_macos_binary",
    "notarize_macos_binary",
    "sign_windows_binary",
    "SERVER_PLATFORMS",
    "APPCAST_TEMPLATE",
]
