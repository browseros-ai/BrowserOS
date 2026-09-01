"""Target-format architecture policy shared by both Linux adapters.

Target names belong here; executable tool selection remains host-owned in the
AppImage adapter. Mixing those two concepts silently breaks cross-builds.
"""

from dataclasses import dataclass


@dataclass(frozen=True)
class LinuxTargetArchitecture:
    """Architecture spellings required by AppImage and Debian metadata."""

    appimage: str
    debian: str


_TARGETS = {
    "x64": LinuxTargetArchitecture(appimage="x86_64", debian="amd64"),
    "arm64": LinuxTargetArchitecture(appimage="aarch64", debian="arm64"),
}


def target_architecture(architecture: str) -> LinuxTargetArchitecture:
    """Resolve one supported target without implying a publication lane."""
    target = _TARGETS.get(architecture)
    if target is None:
        supported = ", ".join(sorted(_TARGETS))
        raise ValueError(
            f"Unsupported Linux architecture: {architecture}. Supported: {supported}"
        )
    return target
