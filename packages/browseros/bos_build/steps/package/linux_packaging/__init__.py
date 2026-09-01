"""Deep interface for producing and recovering Linux release packages.

Callers see one required AppImage/DEB pair. Payload assembly, format policy,
external tools, verification, rollback, and sliced-run recovery stay hidden.
"""

from .pipeline import (
    LinuxArtifactPair,
    LinuxPackagingError,
    build_linux_artifacts,
    require_linux_artifacts,
)

__all__ = [
    "LinuxArtifactPair",
    "LinuxPackagingError",
    "build_linux_artifacts",
    "require_linux_artifacts",
]
