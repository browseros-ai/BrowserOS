#!/usr/bin/env python3
"""Version file parsing and derivation for the build context.

Three inputs live in the repo:
- CHROMIUM_VERSION (MAJOR=/MINOR=/BUILD=/PATCH=): the chromium pin
- bos_build/config/BROWSEROS_BUILD_OFFSET: added to chromium BUILD to
  produce the browseros chromium version (keeps our build numbers above
  upstream's for update ordering)
- resources/BROWSEROS_VERSION: the semantic product version
"""

from pathlib import Path
from typing import Dict, Tuple

from .utils import join_paths


def load_chromium_version(root_dir: Path) -> Tuple[str, Dict[str, str]]:
    """Parse CHROMIUM_VERSION into ("MAJOR.MINOR.BUILD.PATCH", parts)."""
    version_dict: Dict[str, str] = {}
    version_file = join_paths(root_dir, "CHROMIUM_VERSION")

    if version_file.exists():
        for line in version_file.read_text().strip().split("\n"):
            key, value = line.split("=")
            version_dict[key] = value

        chromium_version = (
            f"{version_dict['MAJOR']}.{version_dict['MINOR']}."
            f"{version_dict['BUILD']}.{version_dict['PATCH']}"
        )
        return chromium_version, version_dict

    return "", version_dict


def load_build_offset(root_dir: Path) -> str:
    """Read bos_build/config/BROWSEROS_BUILD_OFFSET."""
    version_file = join_paths(root_dir, "bos_build", "config", "BROWSEROS_BUILD_OFFSET")
    if version_file.exists():
        return version_file.read_text().strip()
    return ""


def load_semantic_version(root_dir: Path) -> str:
    """Read resources/BROWSEROS_VERSION into e.g. "0.31.0".

    PATCH is only included when non-zero; a zero BUILD renders as ".0".
    """
    version_file = join_paths(root_dir, "resources", "BROWSEROS_VERSION")
    if not version_file.exists():
        return ""

    version_dict = {}
    for line in version_file.read_text().strip().split("\n"):
        line = line.strip()
        if not line or "=" not in line:
            continue
        key, value = line.split("=", 1)
        version_dict[key.strip()] = value.strip()

    major = version_dict.get("BROWSEROS_MAJOR", "0")
    minor = version_dict.get("BROWSEROS_MINOR", "0")
    build = version_dict.get("BROWSEROS_BUILD", "0")
    patch = version_dict.get("BROWSEROS_PATCH", "0")

    if patch != "0":
        return f"{major}.{minor}.{build}.{patch}"
    elif build != "0":
        return f"{major}.{minor}.{build}"
    else:
        return f"{major}.{minor}.0"


def derive_browseros_chromium_version(
    version_dict: Dict[str, str], build_offset: str
) -> str:
    """chromium version with BUILD shifted by the browseros offset."""
    if not version_dict or not build_offset:
        return ""
    new_build = int(version_dict["BUILD"]) + int(build_offset)
    return (
        f"{version_dict['MAJOR']}.{version_dict['MINOR']}."
        f"{new_build}.{version_dict['PATCH']}"
    )


# resources/BROWSEROS_VERSION is the single source of update identity. The
# feed version is "10000.MAJOR.MINOR.BUILD.PATCH": carried in the appcast's
# sparkle:version, stamped into CFBundleVersion before signing (what Sparkle
# compares), and mirrored by chrome/browser/win/winsparkle_glue.cc for
# WinSparkle. The fixed 10000 epoch sorts above the retired feed scheme
# (chromium BUILD.PATCH inflated by BROWSEROS_BUILD_OFFSET, ~7950.97 at
# cutover) so already-shipped clients keep seeing new releases as upgrades.
#
# chrome/VERSION deliberately keeps the BUILD+offset scheme on every
# platform: the Windows installer needs a unique, monotonically increasing
# install version per release (versioned install dir + downgrade guard
# against registry versions already shipped in the offset space), and one
# uniform scheme everywhere beats a per-platform split. The updaters no
# longer read it — which also means a release that bumps only the offset is
# invisible to updaters; every release must bump the BrowserOS version.
UPDATE_FEED_EPOCH = 10000


def load_browseros_version_parts(root_dir: Path) -> tuple:
    """Load (major, minor, build, patch) ints from resources/BROWSEROS_VERSION."""
    version_file = join_paths(root_dir, "resources", "BROWSEROS_VERSION")
    if not version_file.exists():
        return ()

    version_dict = {}
    for line in version_file.read_text().strip().split("\n"):
        line = line.strip()
        if not line or "=" not in line:
            continue
        key, value = line.split("=", 1)
        version_dict[key.strip()] = value.strip()

    return tuple(
        int(version_dict.get(f"BROWSEROS_{key}", "0"))
        for key in ("MAJOR", "MINOR", "BUILD", "PATCH")
    )


def update_feed_version(browseros_version_parts: tuple) -> str:
    """Epoch-prefixed feed version compared by Sparkle/WinSparkle."""
    if not browseros_version_parts:
        raise ValueError("resources/BROWSEROS_VERSION was not loaded")

    major, minor, build, patch = browseros_version_parts
    return f"{UPDATE_FEED_EPOCH}.{major}.{minor}.{build}.{patch}"
