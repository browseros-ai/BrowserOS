"""Debian adapter over a verified, format-neutral Linux runtime."""

from __future__ import annotations

import re
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import TYPE_CHECKING

from .architecture import target_architecture
from .policy import (
    absolute_linux_path,
    copy_hicolor_icons,
    linux_identifier,
    single_line,
    write_asset,
    xml_text,
)
from .runtime import RuntimePayloadError, VerifiedRuntime, verify_runtime_layout
from .system import LinuxToolchain, ToolExecutionError

if TYPE_CHECKING:
    from ....core.context import Context


class DebianPackagingError(RuntimeError):
    """Debian policy, construction, metadata, or extraction check failed."""


_DEBIAN_VERSION = re.compile(r"^[0-9A-Za-z.+:~\-]+$")


def build_debian_package(
    ctx: Context,
    runtime: VerifiedRuntime,
    workspace: Path,
    output: Path,
    toolchain: LinuxToolchain,
) -> Path:
    """Stage, construct, and inspect a Debian package candidate."""
    root = workspace / "deb-root"
    lib_root = _inside(root, ctx.product.linux.lib_dir)
    runtime.materialize(lib_root, sandbox_mode=0o755)
    _install_debian_policy(ctx, root)
    _verify_debian_tree(ctx, root, root / "DEBIAN", runtime, toolchain)

    toolchain.build_deb(root, output)
    if output.is_file():
        output.chmod(0o644)
    _verify_finished_deb(ctx, output, workspace, toolchain, runtime)
    return output


def verify_debian_package(
    ctx: Context,
    package: Path,
    workspace: Path,
    toolchain: LinuxToolchain,
) -> None:
    """Inspect a Debian archive recovered without a build-time inventory."""
    _verify_finished_deb(ctx, package, workspace, toolchain, None)


def normalized_debian_version(ctx: Context) -> str:
    """Return the existing Chromium-derived package version in Debian syntax."""
    version = single_line(
        ctx.get_browseros_chromium_version(),
        "Debian package version",
    ).lstrip("v")
    version = version.replace(" ", "").replace("_", ".")
    if not version or not _DEBIAN_VERSION.fullmatch(version):
        raise DebianPackagingError(f"Invalid Debian package version: {version!r}")
    return version


def _install_debian_policy(ctx: Context, root: Path) -> None:
    identity = ctx.product.linux
    package_name = linux_identifier(identity.package_name, "Debian package name")
    launcher = linux_identifier(identity.launcher_name, "launcher name")
    browser = linux_identifier(ctx.BROWSEROS_APP_NAME, "browser executable")
    lib_dir = absolute_linux_path(identity.lib_dir, "Debian runtime path")
    desktop_id = linux_identifier(identity.desktop_id, "desktop id")
    icon_name = linux_identifier(identity.icon_name, "icon name")
    apparmor = linux_identifier(identity.apparmor_profile_name, "AppArmor profile")
    version = normalized_debian_version(ctx)
    try:
        architecture = target_architecture(ctx.architecture).debian
    except ValueError as exc:
        raise DebianPackagingError(str(exc)) from exc

    write_asset(
        root / "usr/bin" / launcher,
        "launcher",
        {"LIB_DIR": lib_dir, "BROWSER_NAME": browser},
    )
    write_asset(
        root / "usr/share/applications" / desktop_id,
        "desktop",
        {
            "DISPLAY_NAME": single_line(ctx.product.display_name, "display name"),
            "EXEC": f"/usr/bin/{launcher} %U",
            "ICON_NAME": icon_name,
        },
    )
    copy_hicolor_icons(ctx, root / "usr/share/icons/hicolor")
    write_asset(
        root / "usr/share/metainfo" / f"{package_name}.metainfo.xml",
        "metainfo",
        {
            "METAINFO_ID": xml_text(identity.metainfo_id, "AppStream id"),
            "DESKTOP_ID": xml_text(identity.desktop_id, "desktop id"),
            "DISPLAY_NAME": xml_text(ctx.product.display_name, "display name"),
            "SUMMARY": xml_text(ctx.product.summary, "summary"),
            "HOMEPAGE_URL": xml_text(ctx.product.homepage_url, "homepage URL"),
            "BUGTRACKER_URL": xml_text(
                ctx.product.bugtracker_url,
                "bug tracker URL",
            ),
            "SUPPORT_URL": xml_text(ctx.product.support_url, "support URL"),
            "DESCRIPTION": xml_text(ctx.product.description, "description"),
            "VERSION": xml_text(version, "version"),
        },
    )
    write_asset(
        root / "etc/apparmor.d" / apparmor,
        "apparmor",
        {
            "DISPLAY_NAME": single_line(ctx.product.display_name, "display name"),
            "APPARMOR_PROFILE": apparmor,
            "LIB_DIR": lib_dir,
            "BROWSER_NAME": browser,
        },
    )

    control_root = root / "DEBIAN"
    write_asset(
        control_root / "control",
        "control",
        {
            "PACKAGE_NAME": package_name,
            "VERSION": version,
            "DEB_ARCH": architecture,
            "HOMEPAGE_URL": single_line(ctx.product.homepage_url, "homepage URL"),
            "DISPLAY_NAME": single_line(ctx.product.display_name, "display name"),
            "SUMMARY": single_line(ctx.product.summary, "summary"),
            "DESCRIPTION": single_line(ctx.product.description, "description"),
        },
    )
    write_asset(
        control_root / "postinst",
        "postinst",
        {
            "LIB_DIR": lib_dir,
            "APPARMOR_PROFILE": apparmor,
            "LAUNCHER_NAME": launcher,
        },
    )
    write_asset(
        control_root / "prerm",
        "prerm",
        {"APPARMOR_PROFILE": apparmor, "LAUNCHER_NAME": launcher},
    )


def _verify_finished_deb(
    ctx: Context,
    package: Path,
    workspace: Path,
    toolchain: LinuxToolchain,
    runtime: VerifiedRuntime | None,
) -> None:
    if not package.is_file() or package.stat().st_size == 0:
        raise DebianPackagingError(
            f"Debian package was not created or is empty: {package}"
        )
    with package.open("rb") as source:
        if source.read(8) != b"!<arch>\n":
            raise DebianPackagingError(
                f"Debian package has an invalid ar header: {package}"
            )
    mode = package.stat().st_mode & 0o777
    if mode != 0o644:
        raise DebianPackagingError(
            f"Debian package mode is {mode:o}; expected archive mode 644: {package}"
        )

    try:
        architecture = target_architecture(ctx.architecture).debian
    except ValueError as exc:
        raise DebianPackagingError(str(exc)) from exc
    expected_fields = {
        "Package": ctx.product.linux.package_name,
        "Version": normalized_debian_version(ctx),
        "Architecture": architecture,
    }
    actual_fields = dict(toolchain.deb_fields(package, tuple(expected_fields)))
    if actual_fields != expected_fields:
        raise DebianPackagingError(
            f"Debian metadata mismatch: expected {expected_fields}, got {actual_fields}"
        )

    data_root = workspace / "deb-extracted-data"
    control_root = workspace / "deb-extracted-control"
    toolchain.extract_deb_data(package, data_root)
    toolchain.extract_deb_control(package, control_root)
    _verify_debian_tree(ctx, data_root, control_root, runtime, toolchain)


def _verify_debian_tree(
    ctx: Context,
    data_root: Path,
    control_root: Path,
    runtime: VerifiedRuntime | None,
    toolchain: LinuxToolchain,
) -> None:
    runtime_root = _inside(data_root, ctx.product.linux.lib_dir)
    try:
        verify_runtime_layout(ctx, runtime_root, sandbox_mode=0o755)
        if runtime is not None:
            runtime.verify_materialized(runtime_root, sandbox_mode=0o755)
    except RuntimePayloadError as exc:
        raise DebianPackagingError(str(exc)) from exc

    identity = ctx.product.linux
    required = (
        data_root / "usr/bin" / identity.launcher_name,
        data_root / "usr/share/applications" / identity.desktop_id,
        data_root / "usr/share/metainfo" / f"{identity.package_name}.metainfo.xml",
        data_root / "etc/apparmor.d" / identity.apparmor_profile_name,
        control_root / "control",
        control_root / "postinst",
        control_root / "prerm",
    )
    missing = [str(path) for path in required if not path.is_file()]
    icon_root = data_root / "usr/share/icons/hicolor"
    if not any(icon_root.glob(f"*x*/apps/{identity.icon_name}.png")):
        missing.append(f"product icon under {icon_root}")
    if missing:
        raise DebianPackagingError(
            "Debian layout is missing required file(s):\n- " + "\n- ".join(missing)
        )

    for executable in (
        data_root / "usr/bin" / identity.launcher_name,
        control_root / "postinst",
        control_root / "prerm",
    ):
        if (executable.stat().st_mode & 0o777) != 0o755:
            raise DebianPackagingError(
                f"Debian executable must have mode 0755: {executable}"
            )

    postinst = (control_root / "postinst").read_text(encoding="utf-8")
    expected_promotion = f"chmod 4755 {identity.lib_dir}/chrome_sandbox"
    if expected_promotion not in postinst:
        raise DebianPackagingError(
            "Debian postinst no longer promotes chrome_sandbox to mode 4755"
        )

    metainfo = (
        data_root / "usr/share/metainfo" / f"{identity.package_name}.metainfo.xml"
    )
    try:
        ET.parse(metainfo)
    except ET.ParseError as exc:
        raise DebianPackagingError(f"Invalid AppStream XML: {exc}") from exc

    if toolchain.can_execute_target(ctx.architecture):
        try:
            unresolved = toolchain.unresolved_libraries(
                runtime_root / ctx.BROWSEROS_APP_NAME,
                runtime_root,
            )
        except ToolExecutionError as exc:
            raise DebianPackagingError(
                f"Extracted Debian runtime library check failed: {exc}"
            ) from exc
        if unresolved:
            raise DebianPackagingError(
                "Extracted Debian runtime has unresolved libraries:\n- "
                + "\n- ".join(unresolved)
            )


def _inside(root: Path, absolute_path: str) -> Path:
    validated = absolute_linux_path(absolute_path, "package path")
    return root.joinpath(*Path(validated.lstrip("/")).parts)
