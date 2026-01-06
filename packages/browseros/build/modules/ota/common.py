#!/usr/bin/env python3
"""Common utilities for OTA update modules"""

import base64
import os
import re
import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import List, Optional, Tuple
from dataclasses import dataclass

from ...common.env import EnvConfig
from ...common.utils import log_error, log_success

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


def get_sign_update_path(chromium_src: Optional[Path] = None, env: Optional[EnvConfig] = None) -> Path:
    """Get path to Sparkle sign_update tool

    Checks in order:
    1. SPARKLE_SIGN_UPDATE_PATH env var
    2. Chromium third_party if chromium_src provided
    """
    if env is None:
        env = EnvConfig()

    # Check env var first
    if env.sparkle_sign_update_path:
        env_path = Path(env.sparkle_sign_update_path)
        if env_path.exists():
            return env_path

    # Check chromium third_party
    if chromium_src:
        sparkle_in_chromium = chromium_src / "third_party" / "sparkle" / "bin" / "sign_update"
        if sparkle_in_chromium.exists():
            return sparkle_in_chromium

    raise FileNotFoundError(
        "sign_update tool not found. Set SPARKLE_SIGN_UPDATE_PATH or ensure "
        "chromium_src/third_party/sparkle/bin/sign_update exists"
    )


def sparkle_sign_file(
    file_path: Path,
    env: Optional[EnvConfig] = None,
    chromium_src: Optional[Path] = None,
) -> Tuple[Optional[str], int]:
    """Sign a file with Sparkle Ed25519 key

    Args:
        file_path: Path to file to sign (typically a zip)
        env: Environment config with Sparkle key
        chromium_src: Optional chromium source path for sign_update tool

    Returns:
        (signature, length) tuple, or (None, 0) on failure
    """
    if env is None:
        env = EnvConfig()

    sign_update = get_sign_update_path(chromium_src, env)

    if not env.has_sparkle_key():
        log_error("SPARKLE_PRIVATE_KEY not set")
        return None, 0

    key_file = None
    try:
        key_data = env.sparkle_private_key
        if not key_data:
            log_error("SPARKLE_PRIVATE_KEY is empty")
            return None, 0

        try:
            decoded = base64.b64decode(key_data)
            key_data = decoded.decode("utf-8")
        except Exception:
            pass

        key_file = tempfile.NamedTemporaryFile(mode="w", suffix=".key", delete=False)
        key_file.write(key_data)
        key_file.close()

        result = subprocess.run(
            [str(sign_update), "--ed-key-file", key_file.name, str(file_path)],
            capture_output=True,
            text=True,
            check=False,
        )

        if result.returncode != 0:
            log_error(f"sign_update failed: {result.stderr}")
            return None, 0

        return parse_sparkle_output(result.stdout.strip())

    except Exception as e:
        log_error(f"Error signing {file_path.name}: {e}")
        return None, 0
    finally:
        if key_file and os.path.exists(key_file.name):
            os.unlink(key_file.name)


def parse_sparkle_output(output: str) -> Tuple[Optional[str], int]:
    """Parse sign_update output to extract signature and length

    Example output:
        sparkle:edSignature="abc123..." length="126911210"
    """
    sig_match = re.search(r'sparkle:edSignature="([^"]+)"', output)
    len_match = re.search(r'length="(\d+)"', output)

    if sig_match and len_match:
        return sig_match.group(1), int(len_match.group(1))

    return None, 0


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
    import zipfile
    import shutil

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


def upload_to_r2(
    file_path: Path,
    r2_path: str,
    env: Optional[EnvConfig] = None,
) -> bool:
    """Upload file to R2 using wrangler

    Args:
        file_path: Local file path
        r2_path: R2 destination path (e.g., "browseros/server/file.zip")
        env: Environment config (unused, wrangler uses its own auth)

    Returns:
        True on success, False on failure
    """
    try:
        result = subprocess.run(
            ["wrangler", "r2", "object", "put", r2_path, f"--file={file_path}", "--remote"],
            capture_output=True,
            text=True,
            check=False,
        )

        if result.returncode != 0:
            log_error(f"wrangler upload failed: {result.stderr}")
            return False

        log_success(f"Uploaded {file_path.name} to {r2_path}")
        return True

    except FileNotFoundError:
        log_error("wrangler not installed. Install with: npm install -g wrangler")
        return False
    except Exception as e:
        log_error(f"Upload failed: {e}")
        return False


def get_appcast_path(channel: str = "alpha") -> Path:
    """Get path to appcast file in config/appcast directory"""
    appcast_dir = Path(__file__).parent.parent.parent / "config" / "appcast"
    if channel == "alpha":
        return appcast_dir / "appcast-server.alpha.xml"
    return appcast_dir / "appcast-server.xml"
