#!/usr/bin/env python3
"""
Build context dataclass to hold all build state

REFACTOR NOTE: This module is being refactored to use sub-components (PathConfig,
BuildConfig, ArtifactRegistry, EnvConfig) to avoid god object anti-pattern.
The old interface is maintained for backward compatibility during the migration.
"""

import time
from pathlib import Path
from dataclasses import dataclass, field
from typing import Dict, List, Optional
from .utils import (
    get_platform,
    get_platform_arch,
    get_executable_extension,
    join_paths,
    IS_WINDOWS,
    IS_MACOS,
)
from .env import EnvConfig
from .paths import get_package_root


# =============================================================================
# Sub-Components - New modular structure
# =============================================================================


class ArtifactRegistry:
    """
    Simple artifact tracking registry

    Tracks artifacts produced during the build process. Each artifact has a unique
    name (string) and a path (Path object). If you need to track multiple paths
    for the same logical artifact, use different names (e.g., "signed_app_arm64",
    "signed_app_x64").

    Example:
        artifacts = ArtifactRegistry()
        artifacts.add("built_app", Path("/path/to/BrowserOS.app"))
        app_path = artifacts.get("built_app")
        if artifacts.has("signed_app"):
            ...
    """

    def __init__(self):
        self._artifacts: Dict[str, Path] = {}

    def add(self, name: str, path: Path) -> None:
        """
        Register an artifact

        Args:
            name: Unique artifact name (e.g., "built_app", "signed_dmg")
            path: Path to the artifact

        Note:
            If an artifact with the same name already exists, it will be overwritten.
        """
        self._artifacts[name] = path

    def get(self, name: str) -> Path:
        """
        Get artifact path by name

        Args:
            name: Artifact name

        Returns:
            Path to the artifact

        Raises:
            KeyError: If artifact not found
        """
        return self._artifacts[name]

    def has(self, name: str) -> bool:
        """
        Check if artifact exists

        Args:
            name: Artifact name

        Returns:
            True if artifact exists, False otherwise
        """
        return name in self._artifacts

    def all(self) -> Dict[str, Path]:
        """Get all artifacts as a dictionary"""
        return self._artifacts.copy()


class PathConfig:
    """
    Path-related configuration

    Centralizes all path construction and validation logic. This prevents the
    BuildContext from becoming a god object with dozens of path-related methods.
    """

    def __init__(
        self,
        root_dir: Path,
        chromium_src: Optional[Path] = None,
        gn_flags_file: Optional[Path] = None,
    ):
        self.root_dir = root_dir
        self._chromium_src = chromium_src or Path()
        self._out_dir = "out/Default"
        self.gn_flags_file = gn_flags_file

    @property
    def chromium_src(self) -> Path:
        """Chromium source directory"""
        return self._chromium_src

    @chromium_src.setter
    def chromium_src(self, value: Path):
        """Set chromium source directory"""
        self._chromium_src = value

    @property
    def out_dir(self) -> str:
        """Output directory (relative to chromium_src)"""
        return self._out_dir

    @out_dir.setter
    def out_dir(self, value: str):
        """Set output directory"""
        self._out_dir = value


class BuildConfig:
    """
    Build-specific configuration

    Contains all build-related settings like architecture, build type, versions, etc.
    """

    def __init__(
        self,
        architecture: Optional[str] = None,
        build_type: str = "debug",
    ):
        self.architecture = architecture or get_platform_arch()
        self.build_type = build_type
        self.chromium_version = ""
        self.browseros_version = ""
        self.browseros_chromium_version = ""

        # App names - will be set based on platform
        self.CHROMIUM_APP_NAME = ""
        self.BROWSEROS_APP_NAME = ""
        self.BROWSEROS_APP_BASE_NAME = "BrowserOS"

        # Third party versions
        self.SPARKLE_VERSION = "2.7.0"

        # Set platform-specific app names
        self._set_app_names()

    def _set_app_names(self):
        """Set platform-specific application names"""
        if IS_WINDOWS():
            self.CHROMIUM_APP_NAME = f"chrome{get_executable_extension()}"
            self.BROWSEROS_APP_NAME = (
                f"{self.BROWSEROS_APP_BASE_NAME}{get_executable_extension()}"
            )
        elif IS_MACOS():
            self.CHROMIUM_APP_NAME = "Chromium.app"
            self.BROWSEROS_APP_NAME = f"{self.BROWSEROS_APP_BASE_NAME}.app"
        else:
            self.CHROMIUM_APP_NAME = "chrome"
            self.BROWSEROS_APP_NAME = self.BROWSEROS_APP_BASE_NAME.lower()


@dataclass
class Context:
    """
    Context Object pattern - ONE place for all build state
    """

    root_dir: Path = field(default_factory=get_package_root)
    chromium_src: Path = Path()
    out_dir: str = "out/Default"
    architecture: str = ""  # Will be set in __post_init__
    build_type: str = "debug"
    chromium_version: str = ""
    browseros_build_offset: str = ""
    browseros_chromium_version: str = ""
    semantic_version: str = ""  # e.g., "0.31.0" from resources/BROWSEROS_VERSION
    release_version: str = ""  # Explicit version for release operations (overrides semantic_version)
    github_repo: str = ""  # GitHub repo for release operations (owner/repo)
    start_time: float = 0.0

    # App names - will be set based on platform
    CHROMIUM_APP_NAME: str = ""
    BROWSEROS_APP_NAME: str = ""
    BROWSEROS_APP_BASE_NAME: str = "BrowserOS"  # Base name without extension

    # Third party
    SPARKLE_VERSION: str = "2.7.0"

    # Legacy artifacts dict - kept for backward compatibility
    # New code should use ctx.artifacts (ArtifactRegistry) instead
    artifacts: Dict[str, List[Path]] = field(default_factory=dict)

    # Fixed app path - used by UniversalBuildModule to prevent auto-detection
    # When set, get_app_path() returns this directly instead of auto-detecting
    _fixed_app_path: Optional[Path] = None

    # New sub-components (initialized in __post_init__)
    paths: PathConfig = field(init=False)
    build: BuildConfig = field(init=False)
    artifact_registry: ArtifactRegistry = field(init=False)  # New artifact system
    env: EnvConfig = field(init=False)

    def __post_init__(self):
        """Load version files and set platform/architecture-specific configurations"""
        # Initialize new sub-components
        self.paths = PathConfig(self.root_dir, self.chromium_src)
        self.build = BuildConfig(self.architecture, self.build_type)
        self.artifact_registry = ArtifactRegistry()  # New artifact system
        self.env = EnvConfig()

        # Set default gn_flags_file if not provided
        if not self.paths.gn_flags_file:
            self.paths.gn_flags_file = self.get_gn_flags_file()

        # Set platform-specific defaults
        if not self.architecture:
            self.architecture = get_platform_arch()
            self.build.architecture = self.architecture

        # Set platform-specific app names
        if IS_WINDOWS():
            self.CHROMIUM_APP_NAME = f"chrome{get_executable_extension()}"
            self.BROWSEROS_APP_NAME = (
                f"{self.BROWSEROS_APP_BASE_NAME}{get_executable_extension()}"
            )
        elif IS_MACOS():
            self.CHROMIUM_APP_NAME = "Chromium.app"
            self.BROWSEROS_APP_NAME = f"{self.BROWSEROS_APP_BASE_NAME}.app"
        else:
            self.CHROMIUM_APP_NAME = "chrome"
            self.BROWSEROS_APP_NAME = self.BROWSEROS_APP_BASE_NAME.lower()

        # Sync with BuildConfig
        self.build.CHROMIUM_APP_NAME = self.CHROMIUM_APP_NAME
        self.build.BROWSEROS_APP_NAME = self.BROWSEROS_APP_NAME

        # Set architecture-specific output directory with platform separator
        if IS_WINDOWS():
            self.out_dir = f"out\\Default_{self.architecture}"
        else:
            self.out_dir = f"out/Default_{self.architecture}"

        # Sync with PathConfig
        self.paths.out_dir = self.out_dir

        # Load version information using static methods
        if not self.chromium_version:
            self.chromium_version, version_dict = self._load_chromium_version(
                self.root_dir
            )
        else:
            # If chromium_version was provided, we still need to parse it for version_dict
            version_dict = {}

        if not self.browseros_build_offset:
            self.browseros_build_offset = self._load_browseros_build_offset(
                self.root_dir
            )

        # Load semantic version from resources/BROWSEROS_VERSION
        if not self.semantic_version:
            self.semantic_version = self._load_semantic_version(self.root_dir)

        # Set nxtscape_chromium_version as chromium version with BUILD + nxtscape_version
        if self.chromium_version and self.browseros_build_offset and version_dict:
            # Calculate new BUILD number by adding nxtscape_version to original BUILD
            new_build = int(version_dict["BUILD"]) + int(self.browseros_build_offset)
            self.browseros_chromium_version = f"{version_dict['MAJOR']}.{version_dict['MINOR']}.{new_build}.{version_dict['PATCH']}"

        # Sync versions with BuildConfig
        self.build.chromium_version = self.chromium_version
        self.build.browseros_version = self.browseros_build_offset
        self.build.browseros_chromium_version = self.browseros_chromium_version

        # Sync chromium_src with PathConfig (validation done by resolver)
        self.paths.chromium_src = self.chromium_src

        self.start_time = time.time()

    # === Initialization ===

    @classmethod
    def init_context(cls, config: Dict) -> "Context":
        """
        Initialize context from config
        Replaces __post_init__ logic for better testability

        Note: root_dir is always computed from package location, never from config.
        """
        chromium_src = (
            Path(config.get("chromium_src", ""))
            if config.get("chromium_src")
            else Path()
        )

        # Get architecture or use platform default
        arch = config.get("architecture") or get_platform_arch()

        # Create instance - root_dir uses default_factory (get_package_root)
        ctx = cls(
            chromium_src=chromium_src,
            architecture=arch,
            build_type=config.get("build_type", "debug"),
        )

        return ctx

    @staticmethod
    def _load_chromium_version(root_dir: Path):
        """
        Load chromium version from CHROMIUM_VERSION file
        Returns: (version_string, version_dict)
        """
        version_dict = {}
        version_file = join_paths(root_dir, "CHROMIUM_VERSION")

        if version_file.exists():
            # Parse VERSION file format: MAJOR=137\nMINOR=0\nBUILD=7151\nPATCH=69
            for line in version_file.read_text().strip().split("\n"):
                key, value = line.split("=")
                version_dict[key] = value

            # Construct chromium_version as MAJOR.MINOR.BUILD.PATCH
            chromium_version = f"{version_dict['MAJOR']}.{version_dict['MINOR']}.{version_dict['BUILD']}.{version_dict['PATCH']}"
            return chromium_version, version_dict

        return "", version_dict

    @staticmethod
    def _load_browseros_build_offset(root_dir: Path) -> str:
        """Load browseros build offset from config/BROWSEROS_BUILD_OFFSET"""
        version_file = join_paths(root_dir, "build", "config", "BROWSEROS_BUILD_OFFSET")
        if version_file.exists():
            return version_file.read_text().strip()
        return ""

    @staticmethod
    def _load_semantic_version(root_dir: Path) -> str:
        """Load semantic version from resources/BROWSEROS_VERSION

        File format:
            BROWSEROS_MAJOR=0
            BROWSEROS_MINOR=31
            BROWSEROS_BUILD=0
            BROWSEROS_PATCH=0

        Returns: "0.31.0" (PATCH only included if non-zero)
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

        # Include patch only if non-zero
        if patch != "0":
            return f"{major}.{minor}.{build}.{patch}"
        elif build != "0":
            return f"{major}.{minor}.{build}"
        else:
            return f"{major}.{minor}.0"

    # Path getter methods
    def get_config_dir(self) -> Path:
        """Get build config directory"""
        return join_paths(self.root_dir, "build", "config")

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

    def get_sparkle_dir(self) -> Path:
        """Get Sparkle directory"""
        return join_paths(self.chromium_src, "third_party", "sparkle")

    def get_sparkle_url(self) -> str:
        """Get Sparkle download URL"""
        return f"https://github.com/sparkle-project/Sparkle/releases/download/{self.SPARKLE_VERSION}/Sparkle-{self.SPARKLE_VERSION}.tar.xz"

    def get_extensions_manifest_url(self) -> str:
        """Get CDN URL for bundled extensions update manifest"""
        return "https://cdn.browseros.com/extensions/update-manifest.xml"

    def get_entitlements_dir(self) -> Path:
        """Get entitlements directory"""
        return join_paths(self.root_dir, "resources", "entitlements")

    def get_pkg_dmg_path(self) -> Path:
        """Get pkg-dmg tool path (macOS only)"""
        return join_paths(self.chromium_src, "chrome", "installer", "mac", "pkg-dmg")

    def get_app_path(self) -> Path:
        """Get built app path

        For universal builds, checks if out/Default_universal/BrowserOS.app exists
        and returns that instead of the architecture-specific path.

        This allows downstream modules (sign, package) to work on the universal
        binary after UniversalBuildModule has run.

        Note: If _fixed_app_path is set, returns that directly (used by
        UniversalBuildModule to prevent auto-detection during arch-specific ops).
        """
        # If fixed path is set (for arch-specific operations), use it directly
        if self._fixed_app_path:
            return self._fixed_app_path

        # Check for universal binary first (macOS only)
        if IS_MACOS():
            universal_app = join_paths(
                self.chromium_src, "out/Default_universal", self.BROWSEROS_APP_NAME
            )
            if universal_app.exists():
                return universal_app

        # For debug builds, check if the app has a different name
        if self.build_type == "debug" and IS_MACOS():
            # Check for debug-branded app name
            debug_app_name = f"{self.BROWSEROS_APP_BASE_NAME} Dev.app"
            debug_app_path = join_paths(self.chromium_src, self.out_dir, debug_app_name)
            if debug_app_path.exists():
                return debug_app_path

        # Return architecture-specific path
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
        """Get standardized artifact filename

        Args:
            artifact_type: One of "dmg", "appimage", "deb", "installer", "installer_zip"

        Returns:
            Standardized filename, e.g., "BrowserOS_v0.31.0_arm64.dmg"
        """
        if not self.semantic_version:
            raise ValueError("semantic_version is not set to generate artifact name")

        version = self.semantic_version
        base = self.BROWSEROS_APP_BASE_NAME
        arch = self.architecture

        match artifact_type:
            case "dmg":
                return f"{base}_v{version}_{arch}.dmg"
            case "appimage":
                return f"{base}_v{version}_{arch}.AppImage"
            case "deb":
                deb_arch = {"x64": "amd64", "arm64": "arm64"}.get(arch, arch)
                return f"{base}_v{version}_{deb_arch}.deb"
            case "installer":
                return f"{base}_v{version}_{arch}_installer.exe"
            case "installer_zip":
                return f"{base}_v{version}_{arch}_installer.zip"
            case _:
                raise ValueError(f"Unknown artifact type: {artifact_type}")

    def get_browseros_chromium_version(self) -> str:
        """Get browseros chromium version string"""
        return self.browseros_chromium_version

    def get_browseros_version(self) -> str:
        """Get browseros version string (build offset)"""
        return self.browseros_build_offset

    def get_semantic_version(self) -> str:
        """Get semantic version from resources/BROWSEROS_VERSION

        Returns: e.g., "0.31.0"
        """
        return self.semantic_version

    def get_sparkle_version(self) -> str:
        """Get Sparkle-compatible version from browseros_chromium_version

        Sparkle uses BUILD.PATCH format for version comparison.
        Returns: e.g., "7231.69"
        """
        if not self.browseros_chromium_version:
            raise ValueError("browseros_chromium_version is not set")

        parts = self.browseros_chromium_version.split(".")
        if len(parts) < 4:
            raise ValueError(
                f"Invalid browseros_chromium_version format: {self.browseros_chromium_version}"
            )
        return f"{parts[2]}.{parts[3]}"

    def get_release_path(self, platform: str) -> str:
        """Get R2 path for release artifacts

        Args:
            platform: "macos", "win", or "linux"

        Returns: e.g., "releases/0.31.0/macos/"
        """
        return f"releases/{self.semantic_version}/{platform}/"

    def get_app_base_name(self) -> str:
        """Get app base name without extension"""
        return self.BROWSEROS_APP_BASE_NAME

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

    def get_features_yaml_path(self) -> Path:
        """Get features.yaml file path"""
        return join_paths(self.root_dir, "build", "features.yaml")

    def get_patch_path_for_file(self, file_path: str) -> Path:
        """Convert a chromium file path to patch file path"""
        return join_paths(self.get_patches_dir(), file_path)

    def get_series_patches_dir(self) -> Path:
        """Get series patches directory (GNU Quilt format)"""
        return join_paths(self.root_dir, "series_patches")
