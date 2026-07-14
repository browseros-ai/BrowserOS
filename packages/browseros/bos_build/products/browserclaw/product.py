#!/usr/bin/env python3
"""BrowserClaw — the browser for web agents."""

from pathlib import Path

from ...core.products import (
    BROWSEROS_BUG_REPORTER_EXTENSION_ID,
    BROWSERCLAW_EXTENSION_ID,
    ProductDescriptor,
)
from ..server_binaries import ServerBundle, SignSpec

BROWSERCLAW_PRODUCT = ProductDescriptor.define(
    id="browserclaw",
    display_name="BrowserClaw",
    windows_installer_guid="{FA2AFFF8-647B-477C-A5D2-905BA8DB9B82}",
    summary="The open source browser for web agents",
    description="BrowserClaw is a Chromium-based browser for agent workflows.",
    required_extensions=(
        (BROWSERCLAW_EXTENSION_ID, "BrowserClaw app"),
        (BROWSEROS_BUG_REPORTER_EXTENSION_ID, "BrowserOS bug reporter"),
    ),
)

BROWSERCLAW_SERVER_BUNDLE = ServerBundle(
    id="browserclaw-server",
    name="BrowserOS Claw Server",
    product_ids=("browserclaw",),
    chromium_output_root="BrowserClawServer",
    local_resources_root=Path("resources/binaries/browseros_claw_server"),
    chromium_resources_root=Path("chrome/browser/browseros/claw_server/resources"),
    macos_bundle_resources_root=Path(
        "Contents/Resources/BrowserClawServer/default/resources"
    ),
    windows_bundle_resources_root=Path("BrowserClawServer/default/resources"),
    macos_binaries={
        "browseros-claw-server": SignSpec(
            "browseros_claw_server",
            "runtime",
            "browseros-executable-entitlements.plist",
        ),
    },
    windows_binaries=("browseros-claw-server.exe",),
    required_in_chromium_output=False,
    unsigned_artifact_prefix="claw-server/prod-resources",
    unsigned_artifact_base_name="browseros-claw-server-resources",
)

# Rust release/OTA metadata stays defined for release-claw-server-rust.yml.
# It is not in SERVER_BUNDLES and ships only when the commented YAML blocks are
# flipped by hand.
BROWSERCLAW_RUST_SERVER_BUNDLE = ServerBundle(
    id="browserclaw-server-rust",
    name="BrowserOS Claw Server (Rust)",
    product_ids=("browserclaw",),
    chromium_output_root="BrowserClawServer",
    local_resources_root=Path("resources/binaries/browseros_claw_server_rust"),
    # Chromium launches BrowserClaw through this existing resources root and
    # expects the staged binary to be named browseros-claw-server.
    chromium_resources_root=Path("chrome/browser/browseros/claw_server/resources"),
    macos_bundle_resources_root=Path(
        "Contents/Resources/BrowserClawServer/default/resources"
    ),
    windows_bundle_resources_root=Path("BrowserClawServer/default/resources"),
    macos_binaries={
        "browseros-claw-server": SignSpec(
            "browseros_claw_server",
            "runtime",
            "browseros-executable-entitlements.plist",
        ),
    },
    windows_binaries=("browseros-claw-server.exe",),
    required_in_chromium_output=False,
    unsigned_artifact_prefix="claw-server-rust/prod-resources",
    unsigned_artifact_base_name="browseros-claw-server-rust-resources",
)
