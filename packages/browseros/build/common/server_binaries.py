#!/usr/bin/env python3
"""Shared sign metadata for bundled BrowserOS server binaries."""

from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Tuple


@dataclass(frozen=True)
class SignSpec:
    """Per-binary codesign metadata."""

    identifier_suffix: str
    options: str
    entitlements: Optional[str] = None


@dataclass(frozen=True)
class ServerBundle:
    """Resource roots and signing metadata for one bundled server."""

    id: str
    name: str
    product_ids: Tuple[str, ...]
    chromium_output_root: str
    local_resources_root: Path
    chromium_resources_root: Path
    macos_bundle_resources_root: Path
    windows_bundle_resources_root: Path
    macos_binaries: Dict[str, SignSpec]
    windows_binaries: Tuple[str, ...]
    required_in_chromium_output: bool = True


BROWSEROS_SERVER_MACOS_BINARIES: Dict[str, SignSpec] = {
    "browseros_server": SignSpec(
        "browseros_server", "runtime", "browseros-executable-entitlements.plist"
    ),
    "bun": SignSpec("bun", "runtime", "browseros-executable-entitlements.plist"),
    "rg": SignSpec("rg", "runtime"),
}

BROWSEROS_CLAW_SERVER_MACOS_BINARIES: Dict[str, SignSpec] = {
    "browseros-claw-server": SignSpec(
        "browseros_claw_server",
        "runtime",
        "browseros-executable-entitlements.plist",
    ),
}


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
    macos_binaries=BROWSEROS_SERVER_MACOS_BINARIES,
    windows_binaries=("browseros_server.exe",),
)

BROWSEROS_CLAW_SERVER_BUNDLE = ServerBundle(
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
    macos_binaries=BROWSEROS_CLAW_SERVER_MACOS_BINARIES,
    windows_binaries=("browseros-claw-server.exe",),
    required_in_chromium_output=False,
)

SERVER_BUNDLES: Tuple[ServerBundle, ...] = (
    BROWSEROS_SERVER_BUNDLE,
    BROWSEROS_CLAW_SERVER_BUNDLE,
)

MACOS_SERVER_BINARIES: Dict[str, SignSpec] = {
    stem: spec
    for bundle in SERVER_BUNDLES
    for stem, spec in bundle.macos_binaries.items()
}

WINDOWS_SERVER_BINARIES: List[str] = list(BROWSEROS_SERVER_BUNDLE.windows_binaries)


def server_bundles_for_product(product_id: str) -> Tuple[ServerBundle, ...]:
    """Return server bundles owned by one build product."""
    return tuple(
        bundle for bundle in SERVER_BUNDLES if product_id in bundle.product_ids
    )


def macos_sign_spec_for(binary_path: Path) -> Optional[SignSpec]:
    """Look up sign metadata by file stem."""
    return MACOS_SERVER_BINARIES.get(binary_path.stem)


def expected_windows_binary_paths(server_bin_dir: Path) -> List[Path]:
    """Resolve the Windows relative-path list against a ``resources/bin`` dir."""
    return [server_bin_dir / rel for rel in WINDOWS_SERVER_BINARIES]


def expected_windows_bundle_binary_paths(
    build_output_dir: Path, product_id: Optional[str] = None
) -> List[Path]:
    """Resolve all bundled server binaries under a Chromium build output dir."""
    paths: List[Path] = []
    bundles = server_bundles_for_product(product_id) if product_id else SERVER_BUNDLES
    for bundle in bundles:
        bin_dir = build_output_dir / bundle.windows_bundle_resources_root / "bin"
        paths.extend(bin_dir / rel for rel in bundle.windows_binaries)
    return paths
