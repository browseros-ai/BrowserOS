#!/usr/bin/env python3
"""Test fixtures that fake the two directory trees the build system operates on.

MockChromium builds a minimal gclient checkout (a real one is multi-GB; this
keeps only the marker files build modules actually touch), and
MockBrowserOSRoot builds a minimal packages/browseros root. Tests compose
the pieces they need instead of depending on a real Chromium checkout.

Only for use from *_test.py files.
"""

import subprocess
from pathlib import Path
from typing import Dict, Optional

import yaml

from .context import Context
from .products import get_product_descriptor

DEFAULT_CHROMIUM_VERSION = "137.0.7151.69"
DEFAULT_BUILD_OFFSET = "80"

# Realistic-enough sample for branding string replacement tests: contains the
# brand terms string_replaces.py rewrites and the "Google Play" exception it
# must preserve.
_BRANDING_SAMPLE = """\
<?xml version="1.0" encoding="UTF-8"?>
<!-- Copyright 2024 The Chromium Authors. All rights reserved. -->
<grit>
  <message name="IDS_PRODUCT_NAME">Google Chrome</message>
  <message name="IDS_SHORT_NAME">Chromium</message>
  <message name="IDS_PLAY">Get it on Google Play</message>
</grit>
"""


def _write_version_file(path: Path, version: str, prefix: str = "") -> None:
    """Write a chromium-style KEY=VALUE version file from 'MAJOR.MINOR.BUILD.PATCH'."""
    major, minor, build, patch = version.split(".")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        f"{prefix}MAJOR={major}\n"
        f"{prefix}MINOR={minor}\n"
        f"{prefix}BUILD={build}\n"
        f"{prefix}PATCH={patch}\n"
    )


class MockChromium:
    """Minimal fake of a Chromium gclient checkout.

    The constructor writes only the cheap baseline markers (.gclient, the
    src/ dir with chrome/VERSION and BUILD.gn); everything else is opt-in so
    each test pays for exactly the tree it needs.
    """

    def __init__(self, root: Path, chromium_version: str = DEFAULT_CHROMIUM_VERSION):
        self.root = root
        self.chromium_version = chromium_version
        self.src = root / "src"

        self.src.mkdir(parents=True, exist_ok=True)
        (root / ".gclient").write_text(
            'solutions = [\n'
            '  {\n'
            '    "name": "src",\n'
            '    "url": "https://chromium.googlesource.com/chromium/src.git",\n'
            '    "managed": False,\n'
            '    "custom_deps": {},\n'
            '    "custom_vars": {},\n'
            '  },\n'
            ']\n'
        )
        _write_version_file(self.src / "chrome" / "VERSION", chromium_version)
        (self.src / "BUILD.gn").write_text("# mock chromium root build file\n")

    def add_file(self, relative_path: str, content: str = "") -> Path:
        """Create a file under src/ (parents included) and return its path."""
        path = self.src / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def with_out_dir(
        self, arch: str, args_gn: Optional[str] = None, product: str = "browseros"
    ) -> Path:
        """Create out/Default_{arch}/, optionally with an args.gn file."""
        out_dir = self.src / "out" / f"Default_{product}_{arch}"
        out_dir.mkdir(parents=True, exist_ok=True)
        if args_gn is not None:
            (out_dir / "args.gn").write_text(args_gn)
        return out_dir

    def with_branding_files(self) -> None:
        """Create the two grd/grdp files string_replaces.py rewrites."""
        self.add_file("chrome/app/chromium_strings.grd", _BRANDING_SAMPLE)
        self.add_file("chrome/app/settings_chromium_strings.grdp", _BRANDING_SAMPLE)

    def with_sparkle(self) -> Path:
        """Create the third_party/sparkle directory marker."""
        sparkle = self.src / "third_party" / "sparkle"
        sparkle.mkdir(parents=True, exist_ok=True)
        return sparkle

    def with_winsparkle(self) -> Path:
        """Create the third_party/winsparkle directory marker."""
        winsparkle = self.src / "third_party" / "winsparkle"
        winsparkle.mkdir(parents=True, exist_ok=True)
        return winsparkle

    def with_pkg_dmg(self) -> Path:
        """Create the chrome/installer/mac/pkg-dmg tool marker."""
        return self.add_file("chrome/installer/mac/pkg-dmg", "#!/bin/sh\n")

    def with_git(self) -> "MockChromium":
        """Turn src/ into a real git repo with an initial commit.

        Identity and signing are configured repo-locally so commits succeed
        regardless of the machine's global git config.
        """
        self._git("init", "-q", "-b", "main")
        self._git("config", "user.email", "tests@browseros.invalid")
        self._git("config", "user.name", "BrowserOS Build Tests")
        self._git("config", "commit.gpgsign", "false")
        self.commit_all("initial mock checkout")
        return self

    def commit_all(self, message: str, allow_empty: bool = False) -> str:
        """Stage everything, commit, and return the commit hash."""
        self._git("add", "-A")
        commit_cmd = ["commit", "-q", "-m", message]
        if allow_empty:
            commit_cmd.append("--allow-empty")
        self._git(*commit_cmd)
        return self._git("rev-parse", "HEAD").stdout.strip()

    def _git(self, *args: str) -> subprocess.CompletedProcess:
        result = subprocess.run(
            ["git", *args], cwd=self.src, capture_output=True, text=True
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"git {' '.join(args)} failed in mock checkout: {result.stderr}"
            )
        return result


class MockBrowserOSRoot:
    """Minimal fake of the packages/browseros root (ctx.root_dir).

    The constructor writes the version files Context.__post_init__ reads;
    patches, replacement files, and config files are opt-in.
    """

    def __init__(
        self,
        root: Path,
        chromium_version: str = DEFAULT_CHROMIUM_VERSION,
        build_offset: str = DEFAULT_BUILD_OFFSET,
        browseros_version: str = "0.31.0.0",
    ):
        self.root = root
        root.mkdir(parents=True, exist_ok=True)

        _write_version_file(root / "CHROMIUM_VERSION", chromium_version)

        offset_file = root / "build" / "config" / "BROWSEROS_BUILD_OFFSET"
        offset_file.parent.mkdir(parents=True, exist_ok=True)
        offset_file.write_text(f"{build_offset}\n")

        _write_version_file(
            root / "resources" / "BROWSEROS_VERSION",
            browseros_version,
            prefix="BROWSEROS_",
        )

    def add_patch(self, relative_path: str, content: str) -> Path:
        """Create a patch file under chromium_patches/ and return its path."""
        path = self.root / "chromium_patches" / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def add_replacement_file(
        self, relative_path: str, content: str, product: str = "browseros"
    ) -> Path:
        """Create a replacement file under chromium_files/ and return its path."""
        root = (
            self.root / "chromium_files" / "common"
            if product == "common"
            else self.root / "chromium_files" / "products" / product
        )
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def write_features_yaml(self, features: Dict) -> Path:
        """Write build/features.yaml wrapping the given features mapping."""
        path = self.root / "build" / "features.yaml"
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, "w") as f:
            yaml.safe_dump(
                {"version": "1.0", "features": features},
                f,
                sort_keys=False,
                default_flow_style=False,
            )
        return path

    def write_copy_config(self, config: Dict) -> Path:
        """Write build/config/copy_resources.yaml with the given mapping."""
        path = self.root / "build" / "config" / "copy_resources.yaml"
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, "w") as f:
            yaml.safe_dump(config, f, sort_keys=False, default_flow_style=False)
        return path

    def write_gn_flags(self, platform: str, build_type: str, content: str) -> Path:
        """Write build/config/gn/flags.{platform}.{build_type}.gn."""
        path = self.root / "build" / "config" / "gn" / f"flags.{platform}.{build_type}.gn"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path


def make_context(
    chromium: MockChromium,
    root: MockBrowserOSRoot,
    architecture: str = "x64",
    build_type: str = "release",
    product: str = "browseros",
) -> Context:
    """Build a real Context wired to the mock trees.

    Returns an actual Context (not a stub), so version loading and path
    derivation in __post_init__ run against the mock files.
    """
    return Context(
        root_dir=root.root,
        chromium_src=chromium.src,
        architecture=architecture,
        build_type=build_type,
        product=get_product_descriptor(product),
    )
