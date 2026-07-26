"""Build pipeline steps.

Import order below is canonical pipeline order: the step registry
preserves registration (import) order within each phase, so reordering
these imports reorders the pipeline.
"""

from .compile.standard import CompileModule
from .compile.universal import MergeUniversalModule
from .extensions.bundled_extensions import BundledExtensionsModule
from .package.linux import LinuxPackageModule
from .package.macos import MacOSPackageModule
from .package.windows import MiniInstallerModule, WindowsPackageModule
from .patches.patches import PatchesModule
from .patches.series_patches import SeriesPatchesModule
from .resources.chromium_replace import ChromiumReplaceModule
from .resources.resources import ResourcesModule
from .resources.string_replaces import StringReplacesModule
from .setup.clean import CleanModule
from .setup.configure import ConfigureModule
from .setup.git import GitSetupModule, SparkleSetupModule, WinSparkleSetupModule
from .sign.linux import LinuxSignModule
from .sign.macos import MacOSSignModule
from .sign.sparkle import SparkleSignModule
from .sign.windows import WindowsSignModule
from .source.provision import SourceCheckoutModule, SourceSyncModule
from .storage.download import DownloadResourcesModule
from .storage.upload import UploadModule

__all__ = [
    "BundledExtensionsModule",
    "ChromiumReplaceModule",
    "CleanModule",
    "CompileModule",
    "ConfigureModule",
    "DownloadResourcesModule",
    "GitSetupModule",
    "LinuxPackageModule",
    "LinuxSignModule",
    "MacOSPackageModule",
    "MacOSSignModule",
    "MergeUniversalModule",
    "MiniInstallerModule",
    "PatchesModule",
    "ResourcesModule",
    "SeriesPatchesModule",
    "SourceCheckoutModule",
    "SourceSyncModule",
    "SparkleSetupModule",
    "SparkleSignModule",
    "StringReplacesModule",
    "UploadModule",
    "WinSparkleSetupModule",
    "WindowsPackageModule",
    "WindowsSignModule",
]
