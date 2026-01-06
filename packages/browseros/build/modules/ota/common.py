#!/usr/bin/env python3
"""Common utilities for OTA update modules"""

import os
import shutil
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from typing import List
from dataclasses import dataclass

from ...common.utils import log_error, log_success

# Re-export sparkle_sign_file from common module
from ...common.sparkle import sparkle_sign_file

SERVER_PLATFORMS = [
    {"name": "darwin_arm64", "binary": "browseros-server-darwin-arm64", "os": "macos", "arch": "arm64"},
    {"name": "darwin_x64", "binary": "browseros-server-darwin-x64", "os": "macos", "arch": "x86_64"},
    {"name": "linux_arm64", "binary": "browseros-server-linux-arm64", "os": "linux", "arch": "arm64"},
    {"name": "linux_x64", "binary": "browseros-server-linux-x64", "os": "linux", "arch": "x86_64"},
    {"name": "windows_x64", "binary": "browseros-server-windows-x64.exe", "os": "windows", "arch": "x86_64"},
]

APPCAST_TEMPLATE = """<?xml version="1.0" encoding="utf-8"?>
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle" version="2.0">
  <channel>
    <title>{title}</title>
    <link>{appcast_url}</link>
    <description>BrowserOS Server binary updates</description>
    <language>en</language>

    <item>
      <sparkle:version>{version}</sparkle:version>
      <pubDate>{pub_date}</pubDate>

{enclosures}
    </item>

  </channel>
</rss>
"""

ENCLOSURE_TEMPLATE = """      <!-- {comment} -->
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


def generate_server_appcast(
    version: str,
    artifacts: List[SignedArtifact],
    channel: str = "alpha",
) -> str:
    """Generate appcast XML for server OTA

    Args:
        version: Version string (e.g., "0.0.36")
        artifacts: List of SignedArtifact with signature info
        channel: "alpha" or "prod"

    Returns:
        Complete appcast XML string
    """
    if channel == "alpha":
        title = "BrowserOS Server (Alpha)"
        appcast_url = "https://cdn.browseros.com/appcast-server.alpha.xml"
    else:
        title = "BrowserOS Server"
        appcast_url = "https://cdn.browseros.com/appcast-server.xml"

    pub_date = datetime.now(timezone.utc).strftime("%a, %d %b %Y %H:%M:%S +0000")

    enclosures = []
    for artifact in artifacts:
        comment = f"{artifact.os.capitalize()} {artifact.arch}"
        if artifact.os == "macos":
            comment = f"macOS {artifact.arch}"

        zip_filename = f"browseros_server_{version}_{artifact.platform}.zip"
        url = f"https://cdn.browseros.com/server/{zip_filename}"

        enclosure = ENCLOSURE_TEMPLATE.format(
            comment=comment,
            url=url,
            os=artifact.os,
            arch=artifact.arch,
            signature=artifact.signature,
            length=artifact.length,
        )
        enclosures.append(enclosure)

    return APPCAST_TEMPLATE.format(
        title=title,
        appcast_url=appcast_url,
        version=version,
        pub_date=pub_date,
        enclosures="\n\n".join(enclosures),
    )


def create_server_zip(
    binary_path: Path,
    output_zip: Path,
    is_windows: bool = False,
) -> bool:
    """Create zip with proper structure: resources/bin/browseros_server

    Args:
        binary_path: Path to the binary to package
        output_zip: Path for output zip file
        is_windows: Whether this is Windows binary (affects target name)

    Returns:
        True on success, False on failure
    """
    staging_dir = output_zip.parent / f"staging_{output_zip.stem}"
    try:
        staging_dir.mkdir(parents=True, exist_ok=True)
        bin_dir = staging_dir / "resources" / "bin"
        bin_dir.mkdir(parents=True, exist_ok=True)

        target_name = "browseros_server.exe" if is_windows else "browseros_server"
        shutil.copy2(binary_path, bin_dir / target_name)

        with zipfile.ZipFile(output_zip, 'w', zipfile.ZIP_DEFLATED) as zf:
            for root, _, files in os.walk(staging_dir):
                for file in files:
                    file_path = Path(root) / file
                    arcname = file_path.relative_to(staging_dir)
                    zf.write(file_path, arcname)

        log_success(f"Created {output_zip.name}")
        return True

    except Exception as e:
        log_error(f"Failed to create zip: {e}")
        return False
    finally:
        if staging_dir.exists():
            shutil.rmtree(staging_dir)


def get_appcast_path(channel: str = "alpha") -> Path:
    """Get path to appcast file in config/appcast directory"""
    appcast_dir = Path(__file__).parent.parent.parent / "config" / "appcast"
    if channel == "alpha":
        return appcast_dir / "appcast-server.alpha.xml"
    return appcast_dir / "appcast-server.xml"
