"""Build pipeline steps.

Import order below is canonical pipeline order: the step registry
preserves registration (import) order within each phase, so reordering
these imports reorders the pipeline.
"""

from .source.provision import SourceCheckoutModule, SourceSyncModule
from .setup.clean import CleanModule
from .setup.git import GitSetupModule, SparkleSetupModule, WinSparkleSetupModule
from .storage.download import DownloadResourcesModule
from .resources.resources import ResourcesModule
from .extensions.bundled_extensions import BundledExtensionsModule
from .resources.chromium_replace import ChromiumReplaceModule
from .resources.string_replaces import StringReplacesModule
from .patches.series_patches import SeriesPatchesModule
from .patches.patches import PatchesModule
from .setup.configure import ConfigureModule
from .compile.standard import CompileModule
from .compile.universal import MergeUniversalModule
from .sign.macos import MacOSSignModule
from .sign.windows import WindowsSignModule
from .sign.linux import LinuxSignModule
from .sign.sparkle import SparkleSignModule
from .package.windows import MiniInstallerModule, WindowsPackageModule
from .package.macos import MacOSPackageModule
from .package.linux import LinuxPackageModule
from .storage.upload import UploadModule

__all__ = [
    "SourceCheckoutModule",
    "SourceSyncModule",
    "CleanModule",
    "GitSetupModule",
    "SparkleSetupModule",
    "WinSparkleSetupModule",
    "DownloadResourcesModule",
    "ResourcesModule",
    "BundledExtensionsModule",
    "ChromiumReplaceModule",
    "StringReplacesModule",
    "SeriesPatchesModule",
    "PatchesModule",
    "ConfigureModule",
    "CompileModule",
    "MergeUniversalModule",
    "MacOSSignModule",
    "WindowsSignModule",
    "LinuxSignModule",
    "SparkleSignModule",
    "MiniInstallerModule",
    "WindowsPackageModule",
    "MacOSPackageModule",
    "LinuxPackageModule",
    "UploadModule",
]
