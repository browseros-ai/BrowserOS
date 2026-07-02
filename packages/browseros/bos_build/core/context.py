#!/usr/bin/env python3
"""Build context: resolved state shared across pipeline steps.

Single-sourced by design — the earlier PathConfig/BuildConfig
sub-objects and SCREAMING name fields duplicated this state and had to
be manually synced; they are gone. App names derive from the product
descriptor on access, version data is parsed by lib.versions, and the
artifact registry is the only artifact channel.
"""

import time
from pathlib import Path
from dataclasses import dataclass, field
from typing import Any, Dict, Optional

from ..lib import versions as versions_mod
from ..lib.env import EnvConfig
from ..lib.paths import get_package_root
from .products import (
    ProductDescriptor,
    default_product_descriptor,
    get_product_descriptor,
)
from ..lib.utils import (
    get_platform,
    get_platform_arch,
    get_executable_extension,
    join_paths,
    IS_WINDOWS,
    IS_MACOS,
)


class ArtifactRegistry:
    """
    Artifact tracking registry — the only artifact channel between steps.

    Values are usually Paths, but steps may register richer objects
    (e.g. the OTA step stores its signed-artifact map).
    """

    def __init__(self):
        self._artifacts: Dict[str, Any] = {}

    def add(self, name: str, value: Any) -> None:
        """Register an artifact; an existing name is overwritten."""
        self._artifacts[name] = value

    def get(self, name: str, default: Any = None) -> Any:
        """Get artifact by name, or default when absent."""
        return self._artifacts.get(name, default)

    def has(self, name: str) -> bool:
        return name in self._artifacts

    def all(self) -> Dict[str, Any]:
        return self._artifacts.copy()


@dataclass
class Context:
    """Resolved build state shared across build steps."""

    root_dir: Path = field(default_factory=get_package_root)
    chromium_src: Path = Path()
    out_dir: str = "out/Default"
    architecture: str = ""  # Defaults to host arch in __post_init__
    build_type: str = "debug"
    chromium_version: str = ""
    browseros_build_offset: str = ""
    browseros_version_parts: tuple = ()  # (major, minor, build, patch) ints
    browseros_chromium_version: str = ""
    semantic_version: str = ""  # e.g. "0.31.0" from resources/BROWSEROS_VERSION
    release_version: str = (
        ""  # Explicit version for release operations (overrides semantic_version)
    )
    github_repo: str = ""  # GitHub repo for release operations (owner/repo)
    start_time: float = 0.0
    product: ProductDescriptor = field(default_factory=default_product_descriptor)
    gn_flags_file: Optional[Path] = None
    # Per-invocation --gn-arg overrides; configure appends them last in
    # args.gn (GN last-write-wins). Never persisted to profiles.
    extra_gn_args: tuple[str, ...] = ()

    # Third party pins
    SPARKLE_VERSION: str = "2.7.0"
    WINSPARKLE_VERSION: str = "0.9.3"

    artifact_registry: ArtifactRegistry = field(init=False)
    env: EnvConfig = field(init=False)

    def __post_init__(self):
        if not isinstance(self.product, ProductDescriptor):
            self.product = get_product_descriptor(self.product)

        self.artifact_registry = ArtifactRegistry()
        self.env = EnvConfig()

        if not self.architecture:
            self.architecture = get_platform_arch()

        if not self.gn_flags_file:
            self.gn_flags_file = self.get_gn_flags_file()

        # Architecture- and product-specific output directory
        sep = "\\" if IS_WINDOWS() else "/"
        self.out_dir = f"out{sep}Default_{self.product.id}_{self.architecture}"

        version_dict: Dict[str, str] = {}
        if not self.chromium_version:
            self.chromium_version, version_dict = versions_mod.load_chromium_version(
                self.root_dir
            )

        if not self.browseros_build_offset:
            self.browseros_build_offset = versions_mod.load_build_offset(self.root_dir)

        if not self.semantic_version:
            self.semantic_version = versions_mod.load_semantic_version(self.root_dir)

        if not self.browseros_version_parts:
            self.browseros_version_parts = versions_mod.load_browseros_version_parts(
                self.root_dir
            )

        if not self.browseros_chromium_version:
            self.browseros_chromium_version = (
                versions_mod.derive_browseros_chromium_version(
                    version_dict, self.browseros_build_offset
                )
            )

        self.start_time = time.time()

    # App names derive from the product descriptor per platform.
    @property
    def BROWSEROS_APP_BASE_NAME(self) -> str:
        return self.product.app_base_name

    @property
    def BROWSEROS_APP_NAME(self) -> str:
        if IS_WINDOWS():
            return f"{self.product.app_base_name}{get_executable_extension()}"
        if IS_MACOS():
            return f"{self.product.app_base_name}.app"
        return self.product.app_base_name.lower()

    @property
    def CHROMIUM_APP_NAME(self) -> str:
        if IS_WINDOWS():
            return f"chrome{get_executable_extension()}"
        if IS_MACOS():
            return "Chromium.app"
        return "chrome"

    # Path getter methods
    def get_config_dir(self) -> Path:
        """Get build config directory"""
        return join_paths(self.root_dir, "bos_build", "config")

    def get_gn_config_dir(self) -> Path:
        """Get GN config directory"""
        return join_paths(self.get_config_dir(), "gn")

    def get_gn_flags_file(self) -> Path:
        """Get GN flags file for current build type"""
        platform = get_platform()
        return join_paths(
            self.get_gn_config_dir(), f"flags.{platform}.{self.build_type}.gn"
        )

    def get_copy_resources_config(self) -> Path:
        """Get copy resources configuration file"""
        return join_paths(self.get_config_dir(), "copy_resources.yaml")

    def get_download_resources_config(self) -> Path:
        """Get download resources configuration file"""
        return join_paths(self.get_config_dir(), "download_resources.yaml")

    def get_sparkle_dir(self) -> Path:
        """Get Sparkle directory"""
        return join_paths(self.chromium_src, "third_party", "sparkle")

    def get_sparkle_url(self) -> str:
        """Get Sparkle download URL"""
        return f"https://github.com/sparkle-project/Sparkle/releases/download/{self.SPARKLE_VERSION}/Sparkle-{self.SPARKLE_VERSION}.tar.xz"

    def get_winsparkle_dir(self) -> Path:
        """Get WinSparkle directory"""
        return join_paths(self.chromium_src, "third_party", "winsparkle")

    def get_winsparkle_url(self) -> str:
        """Get WinSparkle download URL (note the v-prefixed release tag)"""
        return f"https://github.com/vslavik/winsparkle/releases/download/v{self.WINSPARKLE_VERSION}/WinSparkle-{self.WINSPARKLE_VERSION}.zip"

    def get_extensions_manifest_url(self) -> str:
        """Get CDN URL for bundled extensions manifest"""
        return "https://cdn.browseros.com/extensions/bundled-manifest.xml"

    def get_entitlements_dir(self) -> Path:
        """Get entitlements directory"""
        return join_paths(self.root_dir, "resources", "entitlements")

    def get_pkg_dmg_path(self) -> Path:
        """Get pkg-dmg tool path (macOS only)"""
        return join_paths(self.chromium_src, "chrome", "installer", "mac", "pkg-dmg")

    def get_app_path(self) -> Path:
        """Get built app path

        Resolves strictly from this context's own out_dir. Never probes
        other out dirs: a stale product universal app must not hijack
        arch-specific builds' sign/package stages. Universal flows resolve
        here too, since architecture="universal" derives the
        product-specific universal out_dir.
        """
        # Debug builds may carry the dev-branded app name
        if self.build_type == "debug" and IS_MACOS():
            debug_app_name = f"{self.product.app_base_name} Dev.app"
            debug_app_path = join_paths(self.chromium_src, self.out_dir, debug_app_name)
            if debug_app_path.exists():
                return debug_app_path

        return join_paths(self.chromium_src, self.out_dir, self.BROWSEROS_APP_NAME)

    def get_chromium_app_path(self) -> Path:
        """Get original Chromium app path"""
        return join_paths(self.chromium_src, self.out_dir, self.CHROMIUM_APP_NAME)

    def get_gn_args_file(self) -> Path:
        """Get GN args file path"""
        return join_paths(self.chromium_src, self.out_dir, "args.gn")

    def get_notarization_zip(self) -> Path:
        """Get notarization zip path (macOS only)"""
        return join_paths(self.chromium_src, self.out_dir, "notarize.zip")

    def get_artifact_name(self, artifact_type: str) -> str:
        """Get standardized artifact filename for this product/version/arch."""
        if not self.semantic_version:
            raise ValueError("semantic_version is not set to generate artifact name")
        return self.product.artifact_filename(
            artifact_type, self.semantic_version, self.architecture
        )

    def get_browseros_chromium_version(self) -> str:
        """Get browseros chromium version string"""
        return self.browseros_chromium_version

    def get_browseros_version(self) -> str:
        """Get browseros version string (build offset)"""
        return self.browseros_build_offset

    def get_semantic_version(self) -> str:
        """Get semantic version from resources/BROWSEROS_VERSION"""
        return self.semantic_version

    def get_sparkle_version(self) -> str:
        """Update feed version compared by Sparkle/WinSparkle.

        Epoch-prefixed BrowserOS version (see the derivation notes in
        lib/versions.py). Stamped into CFBundleVersion at signing,
        mirrored by chrome/browser/win/winsparkle_glue.cc, and carried in
        the appcast's sparkle:version — the three must stay in lockstep.
        Returns: e.g., "10000.0.47.0.2"
        """
        return versions_mod.update_feed_version(self.browseros_version_parts)

    def get_release_path(self, platform: str) -> str:
        """Get R2 path for release artifacts, e.g. "releases/browseros/0.31.0/macos/"."""
        return (
            f"releases/{self.product.release_prefix}/"
            f"{self.semantic_version}/{platform}/"
        )

    def get_app_base_name(self) -> str:
        """Get app base name without extension"""
        return self.product.app_base_name

    def get_dist_dir(self) -> Path:
        """Get distribution output directory with semantic version"""
        return join_paths(self.root_dir, "releases", self.semantic_version)

    # Dev CLI specific methods
    def get_patches_dir(self) -> Path:
        """Get individual patches directory"""
        return join_paths(self.root_dir, "chromium_patches")

    def get_chromium_replace_files_dir(self) -> Path:
        """Get chromium files replacement directory"""
        return join_paths(self.root_dir, "chromium_files")

    def get_chromium_replace_roots(self) -> list[Path]:
        """Return ordered Chromium overlay roots for the active product."""
        base = self.get_chromium_replace_files_dir()
        return [base / "common", base / "products" / self.product.id]

    def get_product_gn_args(self) -> list[str]:
        """Product GN args: release bakes identity; debug keeps the runtime
        product switch working (it needs both server resource sets)."""
        dev_build = "true" if self.build_type == "debug" else "false"
        return [
            f'browseros_product = "{self.product.gn_product}"',
            f"browseros_allow_runtime_product_override = {dev_build}",
            f"browseros_package_all_server_resources = {dev_build}",
        ]

    def get_features_yaml_path(self) -> Path:
        """Get features.yaml file path"""
        return join_paths(self.root_dir, "bos_build", "features.yaml")

    def get_patch_path_for_file(self, file_path: str) -> Path:
        """Convert a chromium file path to patch file path"""
        return join_paths(self.get_patches_dir(), file_path)

    def get_series_patches_dir(self) -> Path:
        """Get series patches directory (GNU Quilt format)"""
        return join_paths(self.root_dir, "series_patches")
