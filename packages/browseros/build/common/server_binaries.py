#!/usr/bin/env python3
"""Shared sign metadata for BrowserOS Server binaries."""

from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional


@dataclass(frozen=True)
class SignSpec:
    """Per-binary codesign metadata."""

    identifier_suffix: str
    options: str
    entitlements: Optional[str] = None


MACOS_SERVER_BINARIES: Dict[str, SignSpec] = {
    "browseros_server": SignSpec(
        "browseros_server", "runtime", "browseros-executable-entitlements.plist"
    ),
    "bun": SignSpec("bun", "runtime", "browseros-executable-entitlements.plist"),
    "codex": SignSpec("codex", "runtime"),
    "rg": SignSpec("rg", "runtime"),
}


WINDOWS_SERVER_BINARIES: List[str] = [
    "browseros_server.exe",
    "third_party/codex.exe",
]


def macos_sign_spec_for(binary_path: Path) -> Optional[SignSpec]:
    """Look up sign metadata by file stem."""
    return MACOS_SERVER_BINARIES.get(binary_path.stem)


def expected_windows_binary_paths(server_bin_dir: Path) -> List[Path]:
    """Resolve the Windows relative-path list against a ``resources/bin`` dir."""
    return [server_bin_dir / rel for rel in WINDOWS_SERVER_BINARIES]
