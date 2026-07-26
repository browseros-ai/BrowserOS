#!/usr/bin/env python3
"""BrowserOS — the flagship product."""

from pathlib import Path

from ...core.products import (
    BROWSEROS_AGENT_EXTENSION_ID,
    BROWSEROS_BUG_REPORTER_EXTENSION_ID,
    ProductDescriptor,
)
from ..server_binaries import ServerBundle, SignSpec

BROWSEROS_PRODUCT = ProductDescriptor.define(
    id="browseros",
    display_name="BrowserOS",
    windows_installer_guid="{5d8d08af-2df9-4da2-86c1-eac353a0ca32}",
    summary="The open source agentic browser",
    description="BrowserOS is a privacy-focused web browser built on Chromium.",
    required_extensions=(
        (BROWSEROS_AGENT_EXTENSION_ID, "BrowserOS agent"),
        (BROWSEROS_BUG_REPORTER_EXTENSION_ID, "BrowserOS bug reporter"),
    ),
)

BROWSEROS_SERVER_BUNDLE = ServerBundle(
    id="browseros-server",
    name="BrowserOS Server",
    product_ids=("browseros",),
    chromium_output_root="BrowserOSServer",
    local_resources_root=Path("resources/binaries/browseros_server"),
    chromium_resources_root=Path("chrome/browser/browseros/server/resources"),
    macos_bundle_resources_root=Path(
        "Contents/Resources/BrowserOSServer/default/resources"
    ),
    windows_bundle_resources_root=Path("BrowserOSServer/default/resources"),
    macos_binaries={
        "browseros_server": SignSpec(
            "browseros_server", "runtime", "browseros-executable-entitlements.plist"
        ),
        "bun": SignSpec("bun", "runtime", "browseros-executable-entitlements.plist"),
        "rg": SignSpec("rg", "runtime"),
    },
    windows_binaries=("browseros_server.exe",),
)
