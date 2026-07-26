#!/usr/bin/env python3
"""Product doctor: verify a product's identity data and on-disk assets.

Turns "productization" into a checked contract — adding product #3 is
writing its define() call and assets until this is green. Checks:
cross-product uniqueness of every identity that must not collide,
identifier formats, and the chromium overlay branding assets each
product must ship.
"""

import re
from pathlib import Path
from typing import Dict, List, Optional

from ..core.products import ProductDescriptor
from .server_binaries import server_bundles_for_product

# Every product's chromium overlay must carry its branding files.
REQUIRED_OVERLAY_FILES = (
    "chrome/app/theme/chromium/BRANDING.debug",
    "chrome/app/theme/chromium/BRANDING.release",
    "chrome/updater/branding.gni",
    "chrome/enterprise_companion/branding.gni",
)

# Chrome extension ids are 32 chars of a-p (base16 mapped onto a-p).
_EXTENSION_ID_RE = re.compile(r"^[a-p]{32}$")
_GUID_RE = re.compile(
    r"^\{[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}"
    r"-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\}$"
)

# Identity fields that must be unique across all registered products.
_UNIQUE_FIELDS = {
    "id": lambda p: p.id,
    "release_prefix": lambda p: p.release_prefix,
    "mac bundle_id": lambda p: p.mac.bundle_id,
    "mac dev_bundle_id": lambda p: p.mac.dev_bundle_id,
    "windows installer_app_id": lambda p: p.windows.installer_app_id,
    "windows app_user_model_id": lambda p: p.windows.app_user_model_id,
    "linux package_name": lambda p: p.linux.package_name,
}


def check_product(product: ProductDescriptor, root_dir: Path) -> List[str]:
    """Per-product findings (empty list = healthy)."""
    findings: List[str] = []

    overlay = root_dir / "chromium_files" / "products" / product.id
    if not overlay.is_dir():
        findings.append(f"{product.id}: chromium overlay dir missing: {overlay}")
    else:
        for rel in REQUIRED_OVERLAY_FILES:
            if not (overlay / rel).is_file():
                findings.append(
                    f"{product.id}: overlay branding file missing: "
                    f"chromium_files/products/{product.id}/{rel}"
                )

    for ext_id, name in product.required_extension_ids:
        if not _EXTENSION_ID_RE.match(ext_id):
            findings.append(
                f"{product.id}: malformed extension id for '{name}': {ext_id}"
            )

    if not _GUID_RE.match(product.windows.installer_app_id):
        findings.append(
            f"{product.id}: malformed windows installer GUID: "
            f"{product.windows.installer_app_id}"
        )

    if not server_bundles_for_product(product.id):
        findings.append(f"{product.id}: no server bundle registered")

    return findings


def check_uniqueness(products: List[ProductDescriptor]) -> List[str]:
    """Cross-product identity collisions."""
    findings: List[str] = []
    for label, getter in _UNIQUE_FIELDS.items():
        seen: Dict[str, str] = {}
        for product in products:
            value = getter(product)
            if value in seen:
                findings.append(
                    f"duplicate {label} '{value}' ({seen[value]} vs {product.id})"
                )
            else:
                seen[value] = product.id
    return findings


def diagnose(
    root_dir: Path, product_id: Optional[str] = None
) -> List[str]:
    """Run all checks; returns findings (empty = healthy)."""
    from . import PRODUCTS

    products = (
        [PRODUCTS[product_id]] if product_id else list(PRODUCTS.values())
    )
    findings = check_uniqueness(list(PRODUCTS.values()))
    for product in products:
        findings.extend(check_product(product, root_dir))
    return findings
