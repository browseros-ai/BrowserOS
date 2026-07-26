#!/usr/bin/env python3
"""List module - Display release artifacts from R2"""

from typing import List, Optional, Tuple

from ..core.context import Context
from ..core.products import ProductDescriptor
from ..core.step import Step, ValidationError
from ..lib.env import EnvConfig
from ..lib.utils import log_info
from ..lib.r2 import BOTO3_AVAILABLE
from ..products import DEFAULT_PRODUCT_ID, PRODUCTS
from .common import (
    PLATFORMS,
    PLATFORM_DISPLAY_NAMES,
    fetch_all_release_metadata,
    format_size,
    list_all_versions,
    list_legacy_versions,
    version_sort_key,
)

DEFAULT_LIST_LIMIT = 5


def merge_product_versions(
    productized: List[str], legacy: List[str]
) -> List[Tuple[str, bool]]:
    """Merge productized and legacy versions into (version, is_legacy) rows.

    Newest first; a version present in both layouts counts as productized.
    """
    productized_set = set(productized)
    rows = [(version, False) for version in productized]
    rows += [(version, True) for version in legacy if version not in productized_set]
    rows.sort(key=lambda row: version_sort_key(row[0]), reverse=True)
    return rows


def collect_product_rows(
    product: ProductDescriptor, env: EnvConfig
) -> List[Tuple[str, bool]]:
    """Gather a product's version rows from R2.

    Bare pre-product releases predate the registry, so they surface under
    the default product only.
    """
    # Listing keys off release_prefix (R2 layout) while release.json fetches
    # key off product.id (get_release_json's contract); the two are equal
    # for every registered product today.
    productized = list_all_versions(product.release_prefix, env)
    legacy = list_legacy_versions(env) if product.id == DEFAULT_PRODUCT_ID else []
    return merge_product_versions(productized, legacy)


def apply_list_limit(
    rows: List[Tuple[str, bool]], limit: Optional[int]
) -> Tuple[List[Tuple[str, bool]], int]:
    """Slice rows to the display limit; return (visible, hidden count)."""
    if limit is None or limit >= len(rows):
        return rows, 0
    return rows[:limit], len(rows) - limit


class ListModule(Step):
    """List release versions per product, or artifacts for one version"""

    produces = []
    requires = []
    description = "List release artifacts from R2"

    def __init__(
        self,
        products: Optional[List[ProductDescriptor]] = None,
        limit: Optional[int] = DEFAULT_LIST_LIMIT,
    ):
        self.products = products
        self.limit = limit

    def validate(self, ctx: Context) -> None:
        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )

        if not ctx.env.has_r2_config():
            raise ValidationError("R2 configuration not set")

    def execute(self, ctx: Context) -> None:
        if not ctx.release_version:
            self._list_all_versions(ctx)
            return

        self._list_version_details(ctx)

    def _list_all_versions(self, ctx: Context) -> None:
        """Print a bounded newest-first section per product"""
        products = self.products or list(PRODUCTS.values())

        log_info("\nAvailable releases (newest first):")
        for product in products:
            rows = collect_product_rows(product, ctx.env)
            self._print_product_section(product, rows)

        log_info("\nUse `release list <version>` for artifact details")

    def _print_product_section(
        self, product: ProductDescriptor, rows: List[Tuple[str, bool]]
    ) -> None:
        visible, hidden = apply_list_limit(rows, self.limit)

        log_info(f"\n{product.display_name}:")
        if not visible:
            log_info("  (no releases found)")
            return

        for version, is_legacy in visible:
            suffix = " (legacy)" if is_legacy else ""
            log_info(f"  {version}{suffix}")

        if hidden:
            log_info(f"  … {hidden} more (use --all)")

    def _list_version_details(self, ctx: Context) -> None:
        """List detailed artifacts for a specific version"""
        version = ctx.release_version
        metadata = fetch_all_release_metadata(version, ctx.env, ctx.product.id)

        if not metadata:
            log_info(
                f"No release metadata found for {ctx.product.id} version {version}"
            )
            return

        log_info(f"\n{'='*60}")
        log_info(f"Release: {ctx.product.display_name} v{version}")
        log_info(f"{'='*60}")

        download_urls: dict[str, list[str]] = {}

        for platform in PLATFORMS:
            if platform not in metadata:
                continue

            release = metadata[platform]
            log_info(f"\n{PLATFORM_DISPLAY_NAMES[platform]}:")
            log_info(f"  Build Date: {release.get('build_date', 'N/A')}")
            log_info(f"  Chromium: {release.get('chromium_version', 'N/A')}")

            if platform == "macos" and "sparkle_version" in release:
                log_info(f"  Sparkle Version: {release['sparkle_version']}")

            platform_urls = []
            for key, artifact in release.get("artifacts", {}).items():
                size = format_size(artifact.get("size", 0))
                sig_indicator = " [signed]" if "sparkle_signature" in artifact else ""
                log_info(f"  - {key}: {artifact['filename']} ({size}){sig_indicator}")
                if "url" in artifact:
                    platform_urls.append(artifact["url"])

            if platform_urls:
                download_urls[platform] = platform_urls

        log_info(f"\n{'='*60}")
        log_info("Downloads:")
        log_info(f"{'='*60}")

        for platform in PLATFORMS:
            if platform not in download_urls:
                continue
            log_info(f"\n{PLATFORM_DISPLAY_NAMES[platform]}:")
            for url in download_urls[platform]:
                log_info(f"  {url}")

        log_info(f"\n{'='*60}")
