#!/usr/bin/env python3
"""
Build context dataclass to hold all build state
"""

import time
from pathlib import Path
from dataclasses import dataclass
from utils import (
    log_error,
    log_warning,
    get_platform,
    get_platform_arch,
    get_executable_extension,
    join_paths,
    IS_WINDOWS,
    IS_MACOS,
)


@dataclass
class BuildContext:
    """Simple dataclass to hold all build state"""

    root_dir: Path
    chromium_src: Path = Path()
    out_dir: str = "out/Default"
    architecture: str = ""  # Will be set in __post_init__
    build_type: str = "debug"
    apply_patches: bool = False
    sign_package: bool = False
    package: bool = False
    build: bool = False
    chromium_version: str = ""
    blockbrowser_version: str = ""
    blockbrowser_chromium_version: str = ""
    start_time: float = 0.0

    # App names - will be set based on platform
    CHROMIUM_APP_NAME: str = ""
    NXTSCAPE_APP_NAME: str = ""
    NXTSCAPE_APP_BASE_NAME: str = "BlockBrowser"  # Base name without extension

    # Third party
    SPARKLE_VERSION: str = "2.7.0"

    def __post_init__(self):
        """Load version files and set platform/architecture-specific configurations"""
        # Set platform-specific defaults
        if not self.architecture:
            self.architecture = get_platform_arch()

        # Set platform-specific app names
        if IS_WINDOWS:
            self.CHROMIUM_APP_NAME = f"chrome{get_executable_extension()}"
            self.NXTSCAPE_APP_NAME = (
                f"{self.NXTSCAPE_APP_BASE_NAME}{get_executable_extension()}"
            )
        elif IS_MACOS:
            self.CHROMIUM_APP_NAME = "Chromium.app"
            self.NXTSCAPE_APP_NAME = f"{self.NXTSCAPE_APP_BASE_NAME}.app"
        else:
            self.CHROMIUM_APP_NAME = "chrome"
            self.NXTSCAPE_APP_NAME = self.NXTSCAPE_APP_BASE_NAME.lower()

        # Set architecture-specific output directory with platform separator
        if IS_WINDOWS:
            self.out_dir = f"out\\Default_{self.architecture}"
        else:
            self.out_dir = f"out/Default_{self.architecture}"

        version_dict = {}

        if not self.chromium_version:
            # Read from VERSION file
            version_file = join_paths(self.root_dir, "CHROMIUM_VERSION")
            if version_file.exists():
                # Parse VERSION file format: MAJOR=137\nMINOR=0\nBUILD=7151\nPATCH=69
                for line in version_file.read_text().strip().split("\n"):
                    key, value = line.split("=")
                    version_dict[key] = value

                # Construct chromium_version as MAJOR.MINOR.BUILD.PATCH
                self.chromium_version = f"{version_dict['MAJOR']}.{version_dict['MINOR']}.{version_dict['BUILD']}.{version_dict['PATCH']}"

        if not self.blockbrowser_version:
            # Read from NXTSCAPE_VERSION file
            version_file = join_paths(
                self.root_dir, "build", "config", "NXTSCAPE_VERSION"
            )
            if version_file.exists():
                self.blockbrowser_version = version_file.read_text().strip()

        # Set blockbrowser_chromium_version as chromium version with BUILD + blockbrowser_version
        if self.chromium_version and self.blockbrowser_version and version_dict:
            # Calculate new BUILD number by adding blockbrowser_version to original BUILD
            new_build = int(version_dict["BUILD"]) + int(self.blockbrowser_version)
            self.blockbrowser_chromium_version = f"{version_dict['MAJOR']}.{version_dict['MINOR']}.{new_build}.{version_dict['PATCH']}"

        # Determine chromium source directory
        if self.chromium_src and self.chromium_src.exists():
            log_warning(f"📁 Using provided Chromium source: {self.chromium_src}")
        else:
            log_warning(f"⚠️  Provided path does not exist: {self.chromium_src}")
            self.chromium_src = join_paths(self.root_dir, "chromium_src")
            if not self.chromium_src.exists():
                log_error(
                    f"⚠️  Default Chromium source path does not exist: {self.chromium_src}"
                )
                raise FileNotFoundError(
                    f"Chromium source path does not exist: {self.chromium_src}"
                )

        self.start_time = time.time()

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

    def get_patches_dir(self) -> Path:
        """Get patches directory"""
        return join_paths(self.root_dir, "patches")

    def get_blockbrowser_patches_dir(self) -> Path:
        """Get BlockBrowser specific patches directory"""
        return join_paths(self.get_patches_dir(), "browseros")

    def get_sparkle_dir(self) -> Path:
        """Get Sparkle directory"""
        return join_paths(self.chromium_src, "third_party", "sparkle")

    def get_sparkle_url(self) -> str:
        """Get Sparkle download URL"""
        return f"https://github.com/sparkle-project/Sparkle/releases/download/{self.SPARKLE_VERSION}/Sparkle-{self.SPARKLE_VERSION}.tar.xz"

    def get_resources_dir(self) -> Path:
        """Get resources directory"""
        return join_paths(self.root_dir, "resources")

    def get_resources_files_dir(self) -> Path:
        """Get resources files directory"""
        return join_paths(self.get_resources_dir(), "files")

    def get_resources_gen_dir(self) -> Path:
        """Get generated resources directory"""
        return join_paths(self.get_resources_dir(), "gen")

    def get_chrome_resources_dir(self) -> Path:
        """Get Chrome browser resources directory"""
        return join_paths(self.chromium_src, "chrome", "browser", "resources")

    def get_chrome_theme_dir(self) -> Path:
        """Get Chrome theme directory"""
        return join_paths(self.chromium_src, "chrome", "app", "theme", "chromium")

    def get_chrome_app_dir(self) -> Path:
        """Get Chrome app directory"""
        return join_paths(self.chromium_src, "chrome", "app")

    def get_entitlements_dir(self) -> Path:
        """Get entitlements directory"""
        return join_paths(self.root_dir, "resources", "entitlements")

    def get_dmg_dir(self) -> Path:
        """Get DMG output directory (macOS only)"""
        return join_paths(self.chromium_src, self.out_dir, "dmg")

    def get_pkg_dmg_path(self) -> Path:
        """Get pkg-dmg tool path (macOS only)"""
        return join_paths(self.chromium_src, "chrome", "installer", "mac", "pkg-dmg")

    def get_app_path(self) -> Path:
        """Get built app path"""
        # For debug builds, check if the app has a different name
        if self.build_type == "debug" and IS_MACOS:
            # Check for debug-branded app name
            debug_app_name = f"{self.NXTSCAPE_APP_BASE_NAME} Dev.app"
            debug_app_path = join_paths(self.chromium_src, self.out_dir, debug_app_name)
            if debug_app_path.exists():
                return debug_app_path
        return join_paths(self.chromium_src, self.out_dir, self.NXTSCAPE_APP_NAME)

    def get_chromium_app_path(self) -> Path:
        """Get original Chromium app path"""
        return join_paths(self.chromium_src, self.out_dir, self.CHROMIUM_APP_NAME)

    def get_gn_args_file(self) -> Path:
        """Get GN args file path"""
        return join_paths(self.chromium_src, self.out_dir, "args.gn")

    def get_notarization_zip(self) -> Path:
        """Get notarization zip path (macOS only)"""
        return join_paths(self.chromium_src, self.out_dir, "notarize.zip")

    def get_dmg_name(self, signed=False) -> str:
        """Get DMG filename with architecture suffix"""
        if self.architecture == "universal":
            if signed:
                return f"{self.NXTSCAPE_APP_BASE_NAME}_{self.blockbrowser_chromium_version}_universal_signed.dmg"
            return f"{self.NXTSCAPE_APP_BASE_NAME}_{self.blockbrowser_chromium_version}_universal.dmg"
        else:
            if signed:
                return f"{self.NXTSCAPE_APP_BASE_NAME}_{self.blockbrowser_chromium_version}_{self.architecture}_signed.dmg"
            return f"{self.NXTSCAPE_APP_BASE_NAME}_{self.blockbrowser_chromium_version}_{self.architecture}.dmg"

    def get_blockbrowser_chromium_version(self) -> str:
        """Get BlockBrowser version string"""
        return self.blockbrowser_chromium_version

    def get_blockbrowser_version(self) -> str:
        """Get BlockBrowser version string"""
        return self.blockbrowser_version

    def get_app_base_name(self) -> str:
        """Get app base name without extension"""
        return self.NXTSCAPE_APP_BASE_NAME

    def get_dist_dir(self) -> Path:
        """Get distribution output directory with version"""
        return join_paths(self.root_dir, "dist", self.blockbrowser_version)

    # Dev CLI specific methods
    def get_dev_patches_dir(self) -> Path:
        """Get individual patches directory"""
        return join_paths(self.root_dir, "chromium_patches")

    def get_chromium_replace_files_dir(self) -> Path:
        """Get chromium files replacement directory"""
        return join_paths(self.root_dir, "chromium_files")

    def get_features_yaml_path(self) -> Path:
        """Get features.yaml file path"""
        return join_paths(self.root_dir, "features.yaml")

    def get_patch_path_for_file(self, file_path: str) -> Path:
        """Convert a chromium file path to patch file path"""
        return join_paths(self.get_dev_patches_dir(), file_path)
