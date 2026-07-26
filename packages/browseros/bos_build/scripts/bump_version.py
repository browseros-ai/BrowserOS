#!/usr/bin/env python3
"""Bump BrowserOS version files for CI and print the resulting version."""

import argparse
import re
import sys
from pathlib import Path
from typing import Sequence


BUMP_MODES = ("none", "offset-only", "offset+build", "offset+patch")


def _read_int_setting(text: str, key: str) -> int:
    match = re.search(rf"^{re.escape(key)}=(\d+)$", text, re.MULTILINE)
    if match is None:
        raise ValueError(f"Missing {key}")
    return int(match.group(1))


def _replace_int_setting(text: str, key: str, value: int) -> str:
    pattern = re.compile(rf"^({re.escape(key)}=)(\d+)$", re.MULTILINE)
    updated, count = pattern.subn(lambda match: f"{match.group(1)}{value}", text)
    if count != 1:
        raise ValueError(f"Expected exactly one {key} entry")
    return updated


def _semantic_version(version_text: str) -> str:
    major = _read_int_setting(version_text, "BROWSEROS_MAJOR")
    minor = _read_int_setting(version_text, "BROWSEROS_MINOR")
    build = _read_int_setting(version_text, "BROWSEROS_BUILD")
    patch = _read_int_setting(version_text, "BROWSEROS_PATCH")
    if patch != 0:
        return f"{major}.{minor}.{build}.{patch}"
    if build != 0:
        return f"{major}.{minor}.{build}"
    return f"{major}.{minor}.0"


def _bump_version_key(version_text: str, key: str) -> str:
    current = _read_int_setting(version_text, key)
    return _replace_int_setting(version_text, key, current + 1)


def bump_version(package_root: Path, mode: str) -> str:
    """Apply the requested CI version bump and return BrowserOS semantic version."""
    if mode not in BUMP_MODES:
        raise ValueError(f"Unsupported bump mode: {mode}")

    offset_file = package_root / "bos_build" / "config" / "BROWSEROS_BUILD_OFFSET"
    version_file = package_root / "resources" / "BROWSEROS_VERSION"
    offset = int(offset_file.read_text().strip()) if mode != "none" else None
    version_text = version_file.read_text()
    next_version_text = version_text
    if mode == "offset+build":
        next_version_text = _bump_version_key(version_text, "BROWSEROS_BUILD")
        next_version_text = _replace_int_setting(
            next_version_text,
            "BROWSEROS_PATCH",
            0,
        )
    elif mode == "offset+patch":
        next_version_text = _bump_version_key(version_text, "BROWSEROS_PATCH")

    version = _semantic_version(next_version_text)
    if next_version_text != version_text:
        version_file.write_text(next_version_text)
    if offset is not None:
        offset_file.write_text(f"{offset + 1}\n")
    return version


def main(argv: Sequence[str] | None = None) -> int:
    """Parse CLI arguments for GitHub Actions and print the bumped version."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", default="offset+build", choices=BUMP_MODES)
    args = parser.parse_args(argv)

    package_root = Path(__file__).resolve().parents[2]
    print(bump_version(package_root, args.mode))
    return 0


if __name__ == "__main__":
    sys.exit(main())
