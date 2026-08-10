#!/usr/bin/env python3
"""Common utilities for OTA update modules."""

import zipfile
from pathlib import Path
from typing import List, Optional

from ...lib.sparkle import sparkle_sign_file as sparkle_sign_file
from ...lib.utils import log_error, log_success
from ..feeds.publisher import FeedPublisher
from ..feeds.render import (
    ExistingAppcast as ExistingAppcast,
    SignedArtifact as SignedArtifact,
    parse_existing_appcast as parse_existing_appcast,
    parse_server_appcast_content,
    render_server_appcast,
)
from ..feeds.spec import FeedSpec, server_feed

SERVER_PLATFORMS = [
    {"name": "darwin_arm64", "binary": "browseros-server-darwin-arm64", "target": "darwin-arm64", "os": "macos", "arch": "arm64"},
    {"name": "darwin_x64", "binary": "browseros-server-darwin-x64", "target": "darwin-x64", "os": "macos", "arch": "x86_64"},
    {"name": "linux_arm64", "binary": "browseros-server-linux-arm64", "target": "linux-arm64", "os": "linux", "arch": "arm64"},
    {"name": "linux_x64", "binary": "browseros-server-linux-x64", "target": "linux-x64", "os": "linux", "arch": "x86_64"},
    {"name": "windows_x64", "binary": "browseros-server-windows-x64.exe", "target": "windows-x64", "os": "windows", "arch": "x86_64"},
]


def find_server_resources_dir(binaries_dir: Path, platform: dict) -> Optional[Path]:
    """Return a platform's extracted resources directory when present."""
    target = platform.get("target", platform["name"].replace("_", "-"))
    resources = binaries_dir / target / "resources"
    return resources if resources.is_dir() else None


def generate_server_appcast(
    version: str,
    artifacts: List[SignedArtifact],
    channel: str = "alpha",
    existing: Optional[ExistingAppcast] = None,
    bundle_id: str = "browseros-server",
) -> str:
    """Render a server appcast for a bundle+channel via its FeedSpec."""
    spec = server_feed(bundle_id, channel)
    return render_server_appcast(spec, version, artifacts, existing)


def merge_base_appcast(publisher, spec: FeedSpec, staging_path: Path) -> Optional[ExistingAppcast]:
    """Choose the live appcast as the merge base before local staging."""
    live = publisher.fetch_live(spec.key)
    if live is not None:
        parsed = parse_server_appcast_content(live)
        if parsed is not None:
            return parsed
        log_error(f"Live {spec.key} is unparseable — falling back to {staging_path}")
    return parse_existing_appcast(staging_path)


def promote_appcast_content(source_content: str, target_spec: FeedSpec) -> str:
    """Re-render an appcast under another channel's feed specification."""
    existing = parse_server_appcast_content(source_content)
    if existing is None:
        raise ValueError(
            "source appcast is not a valid single-item server appcast"
        )
    if not existing.artifacts:
        raise ValueError("source appcast has no parseable enclosures")
    return render_server_appcast(
        target_spec, existing.version, [], existing=existing
    )


def create_server_bundle_zip(resources_dir: Path, output_zip: Path) -> bool:
    """Zip an extracted resources tree into a Sparkle payload."""
    if not resources_dir.is_dir():
        log_error(f"Resources dir not found: {resources_dir}")
        return False

    bundle_root = resources_dir.parent
    try:
        with zipfile.ZipFile(output_zip, "w", zipfile.ZIP_DEFLATED) as zf:
            for path in sorted(resources_dir.rglob("*")):
                if not path.is_file():
                    continue
                arcname = path.relative_to(bundle_root).as_posix()
                zf.write(path, arcname)
        log_success(f"Created {output_zip.name}")
        return True
    except Exception as e:
        log_error(f"Failed to create bundle zip: {e}")
        return False


def get_appcast_path(channel: str = "alpha", bundle_id: str = "browseros-server") -> Path:
    """Return the tracked staging path for a server appcast."""
    return FeedPublisher().staging_path(server_feed(bundle_id, channel))
