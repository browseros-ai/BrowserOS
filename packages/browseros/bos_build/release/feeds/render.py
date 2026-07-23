#!/usr/bin/env python3
"""Feed renderers: complete single-item update feeds from explicit inputs.

No filesystem or network I/O — rendering is deterministic given its inputs
(the server appcast stamps UTC now only when no same-version existing item
pins the pubDate). The server appcast types moved in from
release/ota/common.py; the dependency direction is strictly ota → feeds.
"""

import json
import re
import textwrap
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, List, Optional, Tuple

from ...core.products import get_product_descriptor
from ...lib.utils import log_error, log_info
from .spec import (
    CDN_BASE_URL,
    EXTENSIONS,
    FeedSpec,
    extension_by_name,
    update_manifest_feed,
)

SPARKLE_NS = "http://www.andymatuschak.org/xml-namespaces/sparkle"
ET.register_namespace("sparkle", SPARKLE_NS)

GUPDATE_NS = "http://www.google.com/update2/response"

BROWSER_APPCAST_TEMPLATE = """<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>{title}</title>
    <link>{link}</link>
    <description>Most recent changes with links to updates.</description>
    <language>en</language>

{item}

  </channel>
</rss>
"""

SERVER_APPCAST_TEMPLATE = """<?xml version="1.0" encoding="utf-8"?>
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle" version="2.0">
  <channel>
    <title>{title}</title>
    <link>{link}</link>
    <description>{description}</description>
    <language>en</language>

    <item>
      <sparkle:version>{version}</sparkle:version>
      <pubDate>{pub_date}</pubDate>

{enclosures}
    </item>

  </channel>
</rss>
"""

SERVER_ENCLOSURE_TEMPLATE = """      <!-- {comment} -->
      <enclosure
        url="{url}"
        sparkle:os="{os}"
        sparkle:arch="{arch}"
        sparkle:edSignature="{signature}"
        length="{length}"
        type="application/zip"/>"""


@dataclass
class SignedArtifact:
    """Represents a signed artifact with Sparkle signature"""

    platform: str
    zip_path: Path
    signature: str
    length: int
    os: str
    arch: str


@dataclass
class ExistingAppcast:
    """Parsed data from an existing appcast file"""

    version: str
    pub_date: str
    artifacts: Dict[str, SignedArtifact]


def _format_pub_date(build_date: str) -> str:
    try:
        dt = datetime.fromisoformat(build_date.replace("Z", "+00:00"))
        return dt.strftime("%a, %d %b %Y %H:%M:%S %z")
    except Exception:
        return build_date


def render_browser_appcast(
    spec: FeedSpec,
    artifact: Dict,
    version: str,
    sparkle_version: str,
    build_date: str,
) -> str:
    """Render a complete single-item Sparkle/WinSparkle browser appcast.

    sparkle_version comes from release.json (stamped at build time) and the
    enclosure signature from the artifact metadata — both are what shipped
    clients verify, so absence is a hard error, not a warning.
    """
    if not sparkle_version:
        raise ValueError(
            f"{spec.key}: release metadata is missing sparkle_version "
            "(release predates the update-feed epoch?)"
        )
    if not artifact.get("sparkle_signature"):
        raise ValueError(
            f"{spec.key}: artifact {artifact.get('filename', '?')} is missing "
            "sparkle_signature — re-sign and re-upload before publishing"
        )

    length = artifact.get("sparkle_length", artifact.get("size", 0))
    if not length:
        raise ValueError(
            f"{spec.key}: artifact {artifact.get('filename', '?')} has no "
            "sparkle_length/size — refusing to ship length=\"0\" to clients"
        )

    product = get_product_descriptor(spec.product)
    os_attr = '\n    sparkle:os="windows"' if spec.platform == "win" else ""
    footer = (
        ""
        if spec.platform == "win"
        else "\n  <sparkle:minimumSystemVersion>10.15</sparkle:minimumSystemVersion>"
    )
    enclosure = f"""<enclosure
    url="{artifact['url']}"{os_attr}
    sparkle:edSignature="{artifact['sparkle_signature']}"
    length="{length}"
    type="application/octet-stream" />"""

    item = f"""<item>
  <title>{product.display_name} - {version}</title>
  <description sparkle:format="plain-text">
  </description>
  <sparkle:version>{sparkle_version}</sparkle:version>
  <sparkle:shortVersionString>{version}</sparkle:shortVersionString>
  <pubDate>{_format_pub_date(build_date)}</pubDate>
  <link>{product.homepage_url}</link>
  {enclosure}{footer}
</item>"""

    return BROWSER_APPCAST_TEMPLATE.format(
        title=spec.title,
        link=spec.link,
        item=textwrap.indent(item, "    "),
    )


def render_server_appcast(
    spec: FeedSpec,
    version: str,
    artifacts: List[SignedArtifact],
    existing: Optional[ExistingAppcast] = None,
) -> str:
    """Render a server appcast, merging platforms when the version matches.

    Same version as ``existing``: platform enclosures merge (new wins) and
    the original pubDate is preserved. New version: fresh item, UTC now.
    Title/link/zip naming all derive from the spec so a prod spec can never
    render alpha metadata.
    """
    if existing is not None and existing.version == version:
        pub_date = existing.pub_date
        merged = dict(existing.artifacts)
        for artifact in artifacts:
            merged[artifact.platform] = artifact
        final_artifacts = list(merged.values())
        log_info(
            f"Merging with existing appcast (kept {len(existing.artifacts)} "
            f"existing, added/updated {len(artifacts)} platforms)"
        )
    else:
        pub_date = datetime.now(timezone.utc).strftime("%a, %d %b %Y %H:%M:%S +0000")
        final_artifacts = list(artifacts)
        if existing is not None:
            log_info(
                f"Version changed ({existing.version} -> {version}), "
                "replacing appcast"
            )

    final_artifacts = sorted(final_artifacts, key=lambda a: a.platform)
    zip_prefix = spec.bundle_id.replace("-", "_")

    enclosures = []
    for artifact in final_artifacts:
        comment = f"{artifact.os.capitalize()} {artifact.arch}"
        if artifact.os == "macos":
            comment = f"macOS {artifact.arch}"

        zip_filename = f"{zip_prefix}_{version}_{artifact.platform}.zip"
        enclosures.append(
            SERVER_ENCLOSURE_TEMPLATE.format(
                comment=comment,
                url=f"{CDN_BASE_URL}/server/{zip_filename}",
                os=artifact.os,
                arch=artifact.arch,
                signature=artifact.signature,
                length=artifact.length,
            )
        )

    return SERVER_APPCAST_TEMPLATE.format(
        title=spec.title,
        link=spec.link,
        description=f"{spec.title} binary updates",
        version=version,
        pub_date=pub_date,
        enclosures="\n\n".join(enclosures),
    )


def parse_server_appcast_content(content: str) -> Optional[ExistingAppcast]:
    """Parse a server appcast document (single-item) from a string."""
    try:
        root = ET.fromstring(content)
    except ET.ParseError as e:
        log_error(f"Malformed appcast XML: {e}")
        return None
    return _parse_appcast_root(root)


def parse_existing_appcast(appcast_path: Path) -> Optional[ExistingAppcast]:
    """Parse an existing server appcast file, or None when absent/invalid."""
    if not appcast_path.exists():
        return None
    try:
        return parse_server_appcast_content(appcast_path.read_text())
    except Exception as e:
        log_error(f"Failed to parse existing appcast: {e}")
        return None


def _parse_appcast_root(root: ET.Element) -> Optional[ExistingAppcast]:
    channel = root.find("channel")
    if channel is None:
        return None

    item = channel.find("item")
    if item is None:
        return None

    version_elem = item.find(f"{{{SPARKLE_NS}}}version")
    if version_elem is None or version_elem.text is None:
        return None
    version = version_elem.text

    pub_date_elem = item.find("pubDate")
    pub_date = (
        pub_date_elem.text
        if pub_date_elem is not None and pub_date_elem.text
        else ""
    )

    artifacts: Dict[str, SignedArtifact] = {}
    for enclosure in item.findall("enclosure"):
        url = enclosure.get("url", "")
        os_type = enclosure.get(f"{{{SPARKLE_NS}}}os", "")
        arch = enclosure.get(f"{{{SPARKLE_NS}}}arch", "")
        signature = enclosure.get(f"{{{SPARKLE_NS}}}edSignature", "")
        length_str = enclosure.get("length", "0")

        if not all([url, os_type, arch, signature]):
            continue

        # Hand-edited live feeds are hostile input: a garbage length must
        # drop the enclosure, not crash the parse.
        try:
            length = int(length_str)
        except ValueError:
            continue

        filename = url.split("/")[-1]
        # e.g. browseros_server_0.0.37_darwin_arm64.zip -> darwin_arm64
        platform_match = _PLATFORM_RE.search(filename)
        if not platform_match:
            continue

        platform = platform_match.group(1)
        artifacts[platform] = SignedArtifact(
            platform=platform,
            zip_path=Path(filename),
            signature=signature,
            length=length,
            os=os_type,
            arch=arch,
        )

    return ExistingAppcast(version=version, pub_date=pub_date, artifacts=artifacts)


_PLATFORM_RE = re.compile(r"_([a-z]+_[a-z0-9]+)\.zip$")


def render_update_manifest(versions: Dict[str, str]) -> str:
    """Render a gupdate (Omaha) update manifest for name→version entries."""
    entries = [
        (extension_by_name(name), version) for name, version in versions.items()
    ]
    root = ET.Element("gupdate", {"xmlns": GUPDATE_NS, "protocol": "2.0"})
    for ext, version in sorted(entries, key=lambda e: e[0].extension_id):
        app = ET.SubElement(root, "app", {"appid": ext.extension_id})
        ET.SubElement(
            app,
            "updatecheck",
            {"codebase": ext.crx_url(version), "version": version},
        )
    ET.indent(root, space="  ")
    return ET.tostring(root, encoding="UTF-8", xml_declaration=True).decode() + "\n"


def render_extensions_json(channel: str) -> str:
    """Render the worker /extensions config: id → channel update-manifest URL."""
    url = update_manifest_feed(channel).url
    ids = sorted(ext.extension_id for ext in EXTENSIONS if ext.in_update_feed)
    data = {"extensions": {ext_id: {"external_update_url": url} for ext_id in ids}}
    return json.dumps(data, indent=2, sort_keys=True) + "\n"


def parse_dotted_version(version: str) -> Tuple[int, ...]:
    """Dotted version → int tuple; non-numeric parts count as 0.

    Trailing zeros are stripped so "0.0.118" == "0.0.118.0" — otherwise the
    downgrade guard would flag the short spelling of an equal version.
    """
    parts = []
    for part in version.strip().split("."):
        try:
            parts.append(int(part))
        except ValueError:
            parts.append(0)
    while parts and parts[-1] == 0:
        parts.pop()
    return tuple(parts)


def extract_appcast_version(content: str) -> Optional[str]:
    """Highest sparkle:version across an appcast's items, or None.

    Our feeds are single-item, but a hand-made multi-item live feed must
    still guard against the newest version it serves, not the first listed.
    """
    try:
        root = ET.fromstring(content)
    except ET.ParseError:
        return None
    versions = [
        elem.text
        for elem in root.findall(f"channel/item/{{{SPARKLE_NS}}}version")
        if elem.text
    ]
    if not versions:
        return None
    return max(versions, key=parse_dotted_version)


def extract_appcast_item_count(content: str) -> Optional[int]:
    """Return the channel item count, or None for malformed XML."""
    try:
        root = ET.fromstring(content)
    except ET.ParseError:
        return None
    return len(root.findall("channel/item"))


def _gupdate_apps(root: ET.Element) -> List[ET.Element]:
    apps = root.findall(f".//{{{GUPDATE_NS}}}app")
    return apps if apps else root.findall(".//app")


def extract_manifest_versions(content: str) -> Dict[str, str]:
    """gupdate manifest → {extension_id: version}; empty on parse failure."""
    try:
        root = ET.fromstring(content)
    except ET.ParseError:
        return {}

    versions: Dict[str, str] = {}
    for app in _gupdate_apps(root):
        app_id = app.get("appid")
        updatecheck = app.find(f"{{{GUPDATE_NS}}}updatecheck")
        if updatecheck is None:
            updatecheck = app.find("updatecheck")
        if not app_id or updatecheck is None:
            continue
        version = updatecheck.get("version")
        if version:
            versions[app_id] = version
    return versions


def extract_enclosure_urls(content: str) -> List[str]:
    """Every downloadable URL a feed points clients at (enclosures + crx)."""
    try:
        root = ET.fromstring(content)
    except ET.ParseError:
        return []

    urls = [
        enclosure.get("url", "")
        for enclosure in root.findall(".//enclosure")
    ]
    for app in _gupdate_apps(root):
        updatecheck = app.find(f"{{{GUPDATE_NS}}}updatecheck")
        if updatecheck is None:
            updatecheck = app.find("updatecheck")
        if updatecheck is not None:
            urls.append(updatecheck.get("codebase", ""))
    return [url for url in urls if url]


def extract_channel_metadata(content: str) -> Tuple[Optional[str], Optional[str]]:
    """(channel title, channel link) from an RSS appcast, or (None, None)."""
    try:
        root = ET.fromstring(content)
    except ET.ParseError:
        return (None, None)
    title = root.find("channel/title")
    link = root.find("channel/link")
    return (
        title.text if title is not None else None,
        link.text if link is not None else None,
    )
