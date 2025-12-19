#!/usr/bin/env python3
"""Common utilities for release modules"""

import subprocess
from datetime import datetime
from typing import Dict, List, Optional

from ...common.env import EnvConfig
from ...common.utils import log_warning
from ..upload import get_release_json, get_r2_client, BOTO3_AVAILABLE

PLATFORMS = ["macos", "win", "linux"]
PLATFORM_DISPLAY_NAMES = {"macos": "macOS", "win": "Windows", "linux": "Linux"}

DOWNLOAD_PATH_MAPPING = {
    "macos": {
        "arm64": "download/BrowserOS-arm64.dmg",
        "x64": "download/BrowserOS-x86_64.dmg",
        "universal": "download/BrowserOS.dmg",
    },
    "win": {
        "x64_installer": "download/BrowserOS_installer.exe",
    },
    "linux": {
        "x64_appimage": "download/BrowserOS.AppImage",
        "x64_deb": "download/browseros.deb",
    },
}


def fetch_all_release_metadata(
    version: str, env: Optional[EnvConfig] = None
) -> Dict[str, Dict]:
    """Fetch release.json from all platforms for a version"""
    if env is None:
        env = EnvConfig()

    metadata = {}
    for platform in PLATFORMS:
        release_data = get_release_json(version, platform, env)
        if release_data:
            metadata[platform] = release_data

    return metadata


def list_all_versions(env: Optional[EnvConfig] = None) -> List[str]:
    """List all available release versions from R2.

    Returns versions sorted in descending order (newest first).
    """
    if not BOTO3_AVAILABLE:
        return []

    if env is None:
        env = EnvConfig()

    if not env.has_r2_config():
        return []

    client = get_r2_client(env)
    if not client:
        return []

    versions = []
    continuation_token = None

    while True:
        kwargs = {
            "Bucket": env.r2_bucket,
            "Prefix": "releases/",
            "Delimiter": "/",
        }
        if continuation_token:
            kwargs["ContinuationToken"] = continuation_token

        try:
            response = client.list_objects_v2(**kwargs)
        except Exception:
            break

        for prefix in response.get("CommonPrefixes", []):
            # prefix looks like "releases/0.31.0/"
            version = prefix["Prefix"].replace("releases/", "").rstrip("/")
            if version:
                versions.append(version)

        if not response.get("IsTruncated"):
            break
        continuation_token = response.get("NextContinuationToken")

    # Sort versions descending (newest first) using version tuple comparison
    def version_key(v: str) -> tuple:
        parts = []
        for part in v.split("."):
            try:
                parts.append(int(part))
            except ValueError:
                parts.append(0)
        return tuple(parts)

    versions.sort(key=version_key, reverse=True)
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
) -> str:
    """Generate Sparkle <item> XML for an artifact"""
    try:
        dt = datetime.fromisoformat(build_date.replace("Z", "+00:00"))
        pub_date = dt.strftime("%a, %d %b %Y %H:%M:%S %z")
    except Exception:
        pub_date = build_date

    signature = artifact.get("sparkle_signature", "")
    length = artifact.get("sparkle_length", artifact.get("size", 0))

    return f"""<item>
  <title>BrowserOS - {version}</title>
  <description sparkle:format="plain-text">
  </description>
  <sparkle:version>{sparkle_version}</sparkle:version>
  <sparkle:shortVersionString>{version}</sparkle:shortVersionString>
  <pubDate>{pub_date}</pubDate>
  <link>https://browseros.com</link>
  <enclosure
    url="{artifact['url']}"
    sparkle:edSignature="{signature}"
    length="{length}"
    type="application/octet-stream" />
  <sparkle:minimumSystemVersion>10.15</sparkle:minimumSystemVersion>
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
