#!/usr/bin/env python3
"""Common utilities for release modules"""

import re
import subprocess
from datetime import datetime
from typing import Dict, List, Optional, Tuple

from ..lib.env import EnvConfig
from ..lib.utils import log_warning
from ..core.products import ProductDescriptor, default_product_descriptor
from ..lib.r2 import get_release_json, get_r2_client, BOTO3_AVAILABLE

PLATFORMS = ["macos", "win", "linux"]
PLATFORM_DISPLAY_NAMES = {"macos": "macOS", "win": "Windows", "linux": "Linux"}


def get_download_path_mapping(
    product: ProductDescriptor | None = None,
) -> Dict[str, Dict[str, str]]:
    """Return product-specific latest-download aliases."""
    product = product or default_product_descriptor()
    prefix = product.artifact_prefix
    return {
        "macos": {
            "arm64": f"download/{prefix}-arm64.dmg",
            "x64": f"download/{prefix}-x86_64.dmg",
            "universal": f"download/{prefix}.dmg",
        },
        "win": {
            "x64_installer": f"download/{prefix}_installer.exe",
        },
        "linux": {
            "x64_appimage": f"download/{prefix}.AppImage",
            "x64_deb": f"download/{prefix}.deb",
            "arm64_appimage": f"download/{prefix}-arm64.AppImage",
            "arm64_deb": f"download/{prefix}-arm64.deb",
        },
    }


def fetch_all_release_metadata(
    version: str, env: Optional[EnvConfig] = None, product_id: str = "browseros"
) -> Dict[str, Dict]:
    """Fetch release.json from all platforms for a version"""
    if env is None:
        env = EnvConfig()

    metadata = {}
    for platform in PLATFORMS:
        release_data = get_release_json(version, platform, env, product_id)
        if release_data:
            metadata[platform] = release_data

    return metadata


# Pre-product releases live at bare releases/<version>/; productized ones
# at releases/<release_prefix>/<version>/. A version is digits-and-dots,
# a product prefix is a name — that shape difference drives legacy detection.
_VERSION_NAME_RE = re.compile(r"\d+(\.\d+)*")


def version_sort_key(version: str) -> tuple:
    """Numeric tuple key for sorting versions; non-numeric parts count as 0."""
    parts = []
    for part in version.split("."):
        try:
            parts.append(int(part))
        except ValueError:
            parts.append(0)
    return tuple(parts)


def _r2_listing_client(env: Optional[EnvConfig]) -> Optional[Tuple[object, EnvConfig]]:
    if not BOTO3_AVAILABLE:
        return None

    if env is None:
        env = EnvConfig()

    if not env.has_r2_config():
        return None

    client = get_r2_client(env)
    if not client:
        return None

    return client, env


def _list_common_prefixes(client, bucket: str, prefix: str) -> List[str]:
    """Return child names under an R2 prefix via paginated delimiter listing."""
    names = []
    continuation_token = None

    while True:
        kwargs = {"Bucket": bucket, "Prefix": prefix, "Delimiter": "/"}
        if continuation_token:
            kwargs["ContinuationToken"] = continuation_token

        try:
            response = client.list_objects_v2(**kwargs)
        except Exception as e:
            # A partial listing renders as "(no releases found)" — flag it.
            log_warning(f"R2 listing failed for {prefix}: {e}")
            break

        for entry in response.get("CommonPrefixes", []):
            name = entry["Prefix"].removeprefix(prefix).rstrip("/")
            if name:
                names.append(name)

        if not response.get("IsTruncated"):
            break
        continuation_token = response.get("NextContinuationToken")

    return names


def list_all_versions(
    release_prefix: str, env: Optional[EnvConfig] = None
) -> List[str]:
    """List a product's release versions from R2, newest first."""
    resolved = _r2_listing_client(env)
    if not resolved:
        return []
    client, env = resolved

    versions = _list_common_prefixes(
        client, env.r2_bucket, f"releases/{release_prefix}/"
    )
    versions.sort(key=version_sort_key, reverse=True)
    return versions


def list_legacy_versions(env: Optional[EnvConfig] = None) -> List[str]:
    """List pre-product bare releases/<version>/ entries, newest first."""
    resolved = _r2_listing_client(env)
    if not resolved:
        return []
    client, env = resolved

    names = _list_common_prefixes(client, env.r2_bucket, "releases/")
    versions = [name for name in names if _VERSION_NAME_RE.fullmatch(name)]
    versions.sort(key=version_sort_key, reverse=True)
    return versions


def format_size(size_bytes: int) -> str:
    """Format bytes as human-readable size"""
    if size_bytes >= 1024 * 1024 * 1024:
        return f"{size_bytes / (1024 * 1024 * 1024):.1f} GB"
    elif size_bytes >= 1024 * 1024:
        return f"{size_bytes / (1024 * 1024):.0f} MB"
    elif size_bytes >= 1024:
        return f"{size_bytes / 1024:.0f} KB"
    return f"{size_bytes} B"


def generate_appcast_item(
    artifact: Dict,
    version: str,
    sparkle_version: str,
    build_date: str,
    platform: str = "macos",
) -> str:
    """Generate a Sparkle/WinSparkle <item> XML for an artifact

    macOS items carry a minimumSystemVersion; Windows items instead tag the
    enclosure with sparkle:os="windows" so WinSparkle accepts it.
    """
    try:
        dt = datetime.fromisoformat(build_date.replace("Z", "+00:00"))
        pub_date = dt.strftime("%a, %d %b %Y %H:%M:%S %z")
    except Exception:
        pub_date = build_date

    signature = artifact.get("sparkle_signature", "")
    length = artifact.get("sparkle_length", artifact.get("size", 0))

    os_attr = '\n    sparkle:os="windows"' if platform == "win" else ""
    footer = (
        ""
        if platform == "win"
        else "\n  <sparkle:minimumSystemVersion>10.15</sparkle:minimumSystemVersion>"
    )
    enclosure = f"""<enclosure
    url="{artifact['url']}"{os_attr}
    sparkle:edSignature="{signature}"
    length="{length}"
    type="application/octet-stream" />"""

    return f"""<item>
  <title>BrowserOS - {version}</title>
  <description sparkle:format="plain-text">
  </description>
  <sparkle:version>{sparkle_version}</sparkle:version>
  <sparkle:shortVersionString>{version}</sparkle:shortVersionString>
  <pubDate>{pub_date}</pubDate>
  <link>https://browseros.com</link>
  {enclosure}{footer}
</item>"""


def generate_release_notes(version: str, metadata: Dict[str, Dict]) -> str:
    """Generate markdown release notes from metadata"""
    chromium_version = "unknown"
    for platform in PLATFORMS:
        if platform in metadata:
            chromium_version = metadata[platform].get("chromium_version", "unknown")
            break

    notes = f"""## BrowserOS v{version}

Chromium version: {chromium_version}

### Downloads

"""
    for platform in PLATFORMS:
        if platform not in metadata:
            continue

        platform_name = PLATFORM_DISPLAY_NAMES[platform]
        notes += f"**{platform_name}:**\n"

        for key, artifact in metadata[platform].get("artifacts", {}).items():
            notes += f"- [{artifact['filename']}]({artifact['url']})\n"
        notes += "\n"

    return notes


def get_repo_from_git() -> Optional[str]:
    """Get GitHub repo (owner/name) from git remote"""
    try:
        result = subprocess.run(
            ["git", "remote", "get-url", "origin"],
            capture_output=True,
            text=True,
            check=True,
        )
        remote_url = result.stdout.strip()

        if "github.com" not in remote_url:
            return None

        if remote_url.startswith("git@"):
            return remote_url.split(":")[-1].replace(".git", "")
        else:
            return "/".join(remote_url.split("/")[-2:]).replace(".git", "")
    except Exception:
        return None


def check_gh_cli() -> bool:
    """Check if gh CLI is available"""
    try:
        subprocess.run(["gh", "--version"], capture_output=True, check=True)
        return True
    except Exception:
        return False
