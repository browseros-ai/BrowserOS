#!/usr/bin/env python3
"""Update-feed spec table — one owner for every feed key clients poll.

Same descriptor pattern as products: a frozen dataclass plus a module-level
table built from small derivation helpers. Publishing anywhere else than
through these specs is the bug class this module exists to kill (hand-edited
XML in the api-worker repo, alpha files byte-copied to prod keys).
"""

from dataclasses import dataclass
from typing import Tuple

from ...core.products import (
    BROWSEROS_AGENT_EXTENSION_ID,
    BROWSEROS_BUG_REPORTER_EXTENSION_ID,
    BROWSERCLAW_EXTENSION_ID,
    get_product_descriptor,
)
from ...products import SERVER_BUNDLES
from ...products.server_binaries import ServerBundle

CDN_BASE_URL = "https://cdn.browseros.com"


@dataclass(frozen=True)
class ExtensionSpec:
    """One CDN-distributed extension; name is the crx filename prefix."""

    name: str
    extension_id: str
    in_update_feed: bool  # listed in update-manifest + extensions.json

    def crx_key(self, version: str) -> str:
        return f"extensions/{self.name}-{version}.crx"

    def crx_url(self, version: str) -> str:
        return f"{CDN_BASE_URL}/{self.crx_key(version)}"


EXTENSIONS: Tuple[ExtensionSpec, ...] = (
    ExtensionSpec("agent", BROWSEROS_AGENT_EXTENSION_ID, True),
    ExtensionSpec("bugreporter", BROWSEROS_BUG_REPORTER_EXTENSION_ID, True),
    ExtensionSpec("browserclaw", BROWSERCLAW_EXTENSION_ID, True),
)


def extension_by_name(name: str) -> ExtensionSpec:
    for ext in EXTENSIONS:
        if ext.name == name:
            return ext
    valid = ", ".join(sorted(ext.name for ext in EXTENSIONS))
    raise ValueError(f"Unknown extension '{name}'. Valid: {valid}")


@dataclass(frozen=True)
class FeedSpec:
    """One publishable feed object: a flat R2 key plus render metadata."""

    key: str
    kind: str  # "browser" | "server" | "extensions"
    product: str
    channel: str  # "prod" | "alpha" | "" (channel-independent)
    title: str = ""  # channel <title> ("" for extensions kind)
    link: str = ""  # channel <link> ("" for extensions kind)
    platform: str = ""  # browser feeds: "macos" | "win"
    artifact_keys: Tuple[str, ...] = ()  # browser feeds: release.json priority
    bundle_id: str = ""  # server feeds: owning ServerBundle id
    publishable: bool = True

    @property
    def url(self) -> str:
        return f"{CDN_BASE_URL}/{self.key}"


# Browser feed key infix per product ("" keeps today's browseros keys).
_BROWSER_FEED_SLUGS = {"browseros": "", "browserclaw": "claw"}

# Products whose shipping updater selects this feed by browseros::GetProduct()
# (sparkle_glue.mm / winsparkle_glue.cc). A product listed in
# _BROWSER_FEED_SLUGS but missing here would ship an updater still pointing at
# the browseros feed, so its feeds stay unpublishable until that client lands.
_BROWSER_FEED_CLIENTS = frozenset({"browseros", "browserclaw"})

# Server feed key slug per bundle (appcast-<slug>[.alpha].xml).
_SERVER_FEED_SLUGS = {
    "browseros-server": "server",
    "browserclaw-server": "claw-server",
}


def _browser_feeds(product_id: str) -> Tuple[FeedSpec, ...]:
    """Derive the four Sparkle/WinSparkle browser feeds for one product.

    A product's feeds are publishable once its updater selects them by
    browseros::GetProduct() — see _BROWSER_FEED_CLIENTS.
    """
    slug = _BROWSER_FEED_SLUGS.get(product_id)
    if slug is None:
        raise ValueError(
            f"Product '{product_id}' has no browser feed key slug — add it "
            "to _BROWSER_FEED_SLUGS in release/feeds/spec.py"
        )
    infix = f"-{slug}" if slug else ""
    display = get_product_descriptor(product_id).display_name
    publishable = product_id in _BROWSER_FEED_CLIENTS

    def feed(suffix: str, platform: str, artifact_keys: Tuple[str, ...],
             title: str) -> FeedSpec:
        key = f"appcast{infix}{suffix}.xml"
        return FeedSpec(
            key=key,
            kind="browser",
            product=product_id,
            channel="prod",
            title=title,
            link=f"{CDN_BASE_URL}/{key}",
            platform=platform,
            artifact_keys=artifact_keys,
            publishable=publishable,
        )

    return (
        # A universal dmg serves both mac arches, so it wins when present.
        feed("", "macos", ("universal", "arm64"), display),
        feed("-x86_64", "macos", ("x64", "universal"), display),
        feed("-win", "win", ("x64_installer",), f"{display} Windows Updates"),
        feed("-win-arm64", "win", ("arm64_installer",),
             f"{display} Windows Updates"),
    )


def _server_feeds(bundle: ServerBundle) -> Tuple[FeedSpec, ...]:
    slug = _SERVER_FEED_SLUGS.get(bundle.id)
    if slug is None:
        raise ValueError(
            f"Server bundle '{bundle.id}' has no feed key slug — add it to "
            "_SERVER_FEED_SLUGS in release/feeds/spec.py"
        )
    product_id = bundle.product_ids[0]

    def feed(channel: str, suffix: str, title: str) -> FeedSpec:
        key = f"appcast-{slug}{suffix}.xml"
        return FeedSpec(
            key=key,
            kind="server",
            product=product_id,
            channel=channel,
            title=title,
            link=f"{CDN_BASE_URL}/{key}",
            bundle_id=bundle.id,
        )

    return (
        feed("prod", "", bundle.name),
        feed("alpha", ".alpha", f"{bundle.name} (Alpha)"),
    )


def _extension_feeds() -> Tuple[FeedSpec, ...]:
    feeds = []
    for channel, suffix in (("prod", ""), ("alpha", ".alpha")):
        feeds.append(
            FeedSpec(
                key=f"extensions/update-manifest{suffix}.xml",
                kind="extensions",
                product="browseros",
                channel=channel,
            )
        )
        feeds.append(
            FeedSpec(
                key=f"extensions/extensions{suffix}.json",
                kind="extensions",
                product="browseros",
                channel=channel,
            )
        )
    # Consumed at build time by the bundled_extensions step, not by clients.
    feeds.append(
        FeedSpec(
            key="extensions/bundled-manifest.xml",
            kind="extensions",
            product="browseros",
            channel="",
        )
    )
    return tuple(feeds)


def _build_feeds() -> Tuple[FeedSpec, ...]:
    feeds = (
        *_browser_feeds("browseros"),
        *_browser_feeds("browserclaw"),
        *(feed for bundle in SERVER_BUNDLES for feed in _server_feeds(bundle)),
        *_extension_feeds(),
    )
    keys = [feed.key for feed in feeds]
    duplicates = sorted({key for key in keys if keys.count(key) > 1})
    if duplicates:
        raise ValueError(f"Duplicate feed keys: {', '.join(duplicates)}")
    return feeds


FEEDS: Tuple[FeedSpec, ...] = _build_feeds()


def all_feeds() -> Tuple[FeedSpec, ...]:
    return FEEDS


def feed_by_key(key: str) -> FeedSpec:
    for feed in FEEDS:
        if feed.key == key:
            return feed
    raise ValueError(f"Unknown feed key: {key}")


def browser_feeds_for_product(product_id: str) -> Tuple[FeedSpec, ...]:
    return tuple(
        feed
        for feed in FEEDS
        if feed.kind == "browser" and feed.product == product_id
    )


def server_feed(bundle_id: str, channel: str) -> FeedSpec:
    for feed in FEEDS:
        if feed.kind == "server" and feed.bundle_id == bundle_id \
                and feed.channel == channel:
            return feed
    raise ValueError(f"No server feed for bundle '{bundle_id}' channel '{channel}'")


def update_manifest_feed(channel: str) -> FeedSpec:
    suffix = ".alpha" if channel == "alpha" else ""
    return feed_by_key(f"extensions/update-manifest{suffix}.xml")


def extensions_json_feed(channel: str) -> FeedSpec:
    suffix = ".alpha" if channel == "alpha" else ""
    return feed_by_key(f"extensions/extensions{suffix}.json")


def bundled_manifest_feed() -> FeedSpec:
    return feed_by_key("extensions/bundled-manifest.xml")
