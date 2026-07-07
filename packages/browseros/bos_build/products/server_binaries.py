#!/usr/bin/env python3
"""Shared sign metadata types and lookups for bundled server binaries.

The bundle definitions themselves live with their owning product in
bos_build/products/<id>/product.py; this module keeps the types and
product-keyed lookups. Registry access is lazy to avoid an import
cycle (product files import these types).
"""

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
    unsigned_artifact_prefix: str = "artifacts/server"
    unsigned_artifact_base_name: Optional[str] = None

    def unsigned_artifact_key(self, target: str) -> str:
        """R2 source key of the unsigned resource zip consumed by OTA."""
        base_name = self.unsigned_artifact_base_name or f"{self.id}-resources"
        return f"{self.unsigned_artifact_prefix}/latest/{base_name}-{target}.zip"


def all_server_bundles() -> Tuple[ServerBundle, ...]:
    """Every product's active browser-build server bundles."""
    return _browser_build_server_bundles()


def server_bundles_for_product(product_id: str) -> Tuple[ServerBundle, ...]:
    """Return active browser-build server bundles owned by one product."""
    return tuple(
        bundle
        for bundle in all_server_bundles()
        if product_id in bundle.product_ids
    )


def server_ota_bundles_for_product(product_id: str) -> Tuple[ServerBundle, ...]:
    """Return server OTA bundles; BrowserClaw OTA stays on TypeScript for now."""
    from . import SERVER_BUNDLES

    return tuple(
        bundle
        for bundle in SERVER_BUNDLES
        if product_id in bundle.product_ids
    )


def macos_sign_spec_for(binary_path: Path) -> Optional[SignSpec]:
    """Look up sign metadata by file stem across all bundles."""
    for bundle in all_server_bundles():
        spec = bundle.macos_binaries.get(binary_path.stem)
        if spec is not None:
            return spec
    return None


def expected_windows_binary_paths(server_bin_dir: Path) -> List[Path]:
    """Resolve the browseros server's Windows binaries under resources/bin."""
    bundles = server_bundles_for_product("browseros")
    return [
        server_bin_dir / rel for bundle in bundles for rel in bundle.windows_binaries
    ]


def expected_windows_bundle_binary_paths(
    build_output_dir: Path,
    product_id: Optional[str] = None,
) -> List[Path]:
    """Resolve all bundled server binaries under a Chromium build output dir."""
    paths: List[Path] = []
    bundles = (
        server_bundles_for_product(product_id)
        if product_id
        else all_server_bundles()
    )
    for bundle in bundles:
        bin_dir = build_output_dir / bundle.windows_bundle_resources_root / "bin"
        paths.extend(bin_dir / rel for rel in bundle.windows_binaries)
    return paths


def _browser_build_server_bundles() -> Tuple[ServerBundle, ...]:
    from .browseros.product import BROWSEROS_SERVER_BUNDLE
    from .browserclaw.product import BROWSERCLAW_RUST_SERVER_BUNDLE

    return (BROWSEROS_SERVER_BUNDLE, BROWSERCLAW_RUST_SERVER_BUNDLE)
