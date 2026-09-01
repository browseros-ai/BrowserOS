#!/usr/bin/env python3
"""Runner adapter for the deep Linux packaging module.

The build runner owns validation and step registration. Payload selection,
format construction, verification, cleanup, and artifact correlation belong to
`linux_packaging`, whose successful result is always an AppImage/DEB pair.
"""

from pathlib import Path

from ...core.context import Context
from ...core.step import Step, ValidationError, step
from ...lib.utils import IS_LINUX, log_info, log_success
from .linux_packaging import build_linux_artifacts
from .linux_packaging.architecture import target_architecture


def get_linux_architecture_config(architecture: str) -> dict[str, str]:
    """Compatibility view of target-format names owned by the deep module."""
    target = target_architecture(architecture)
    return {"appimage_arch": target.appimage, "deb_arch": target.debian}


@step("package_linux", phase="package", platforms=("linux",))
class LinuxPackageModule(Step):
    """Adapt the zero-argument runner step to all-or-nothing Linux packaging."""

    produces = ["appimage", "deb"]
    requires = []
    description = "Create verified AppImage and .deb packages for Linux"

    def validate(self, ctx: Context) -> None:
        if not IS_LINUX():
            raise ValidationError("Linux packaging requires Linux")
        try:
            get_linux_architecture_config(ctx.architecture)
        except ValueError as exc:
            raise ValidationError(str(exc)) from exc

        browser = Path(ctx.chromium_src) / ctx.out_dir / ctx.BROWSEROS_APP_NAME
        if not browser.is_file():
            raise ValidationError(f"Browser binary not found: {browser}")

    def execute(self, ctx: Context) -> None:
        log_info(
            "\n📦 Packaging "
            f"{ctx.BROWSEROS_APP_BASE_NAME} "
            f"{ctx.get_browseros_chromium_version()} for Linux "
            f"({ctx.architecture})"
        )
        pair = build_linux_artifacts(ctx)
        log_success("Linux packaging complete")
        log_info(f"   AppImage: {pair.appimage}")
        log_info(f"   Debian:   {pair.deb}")
