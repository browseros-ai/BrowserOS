#!/usr/bin/env python3
"""Update-feed ownership: spec table, renderers, and the rails publisher."""

from .spec import (
    CDN_BASE_URL,
    EXTENSIONS,
    ExtensionSpec,
    FeedSpec,
    all_feeds,
    browser_feeds_for_product,
    bundled_manifest_feed,
    extension_by_name,
    extensions_json_feed,
    feed_by_key,
    server_feed,
    update_manifest_feed,
)

__all__ = [
    "CDN_BASE_URL",
    "EXTENSIONS",
    "ExtensionSpec",
    "FeedSpec",
    "all_feeds",
    "browser_feeds_for_product",
    "bundled_manifest_feed",
    "extension_by_name",
    "extensions_json_feed",
    "feed_by_key",
    "server_feed",
    "update_manifest_feed",
]
