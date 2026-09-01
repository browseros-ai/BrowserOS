"""AppImage adapter over a verified, format-neutral Linux runtime."""

from __future__ import annotations

import hashlib
import os
import shutil
import tempfile
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Protocol

from ....lib.utils import get_platform_arch
from .architecture import target_architecture
from .policy import (
    absolute_linux_path,
    copy_hicolor_icons,
    linux_identifier,
    product_icon_source,
    single_line,
    write_asset,
)
from .runtime import (
    RuntimePayloadError,
    VerifiedRuntime,
    verify_runtime_layout,
)
from .system import LinuxToolchain, ToolExecutionError

if TYPE_CHECKING:
    from ....core.context import Context


class AppImagePackagingError(RuntimeError):
    """AppImage policy, acquisition, construction, or inspection failed."""


@dataclass(frozen=True)
class ToolPin:
    """Versioned executable identity trusted by the packaging pipeline."""

    version: str
    filename: str
    url: str
    sha256: str


class Downloader(Protocol):
    """True external seam for fetching a pinned executable."""

    def download(self, url: str, destination: Path) -> None: ...


class HttpsDownloader:
    """Stream one HTTPS response into a caller-owned temporary file."""

    def download(self, url: str, destination: Path) -> None:
        if not url.startswith("https://"):
            raise AppImagePackagingError(f"Refusing non-HTTPS tool URL: {url}")
        request = urllib.request.Request(
            url,
            headers={"User-Agent": "BrowserOS-Linux-Packager"},
        )
        with (
            urllib.request.urlopen(request, timeout=60) as response,
            destination.open("wb") as output,
        ):
            shutil.copyfileobj(response, output)


_APPIMAGETOOL_PINS = {
    "x64": ToolPin(
        version="1.9.1",
        filename="appimagetool-x86_64.AppImage",
        url=(
            "https://github.com/AppImage/appimagetool/releases/download/"
            "1.9.1/appimagetool-x86_64.AppImage"
        ),
        sha256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0",
    ),
    # This selects a tool that runs on an ARM Linux build host; it does not add
    # an ARM64 target artifact or release lane.
    "arm64": ToolPin(
        version="1.9.1",
        filename="appimagetool-aarch64.AppImage",
        url=(
            "https://github.com/AppImage/appimagetool/releases/download/"
            "1.9.1/appimagetool-aarch64.AppImage"
        ),
        sha256="f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158",
    ),
}
_DOWNLOADER: Downloader = HttpsDownloader()


def build_appimage(
    ctx: Context,
    runtime: VerifiedRuntime,
    workspace: Path,
    output: Path,
    toolchain: LinuxToolchain,
) -> Path:
    """Stage, construct, and inspect an AppImage candidate."""
    appdir = workspace / "AppDir"
    runtime_root = _inside(appdir, ctx.product.linux.appimage_dir)
    runtime.materialize(runtime_root, sandbox_mode=0o4755)
    _install_appimage_policy(ctx, appdir)
    _verify_appdir(ctx, appdir, runtime)

    try:
        target_arch = target_architecture(ctx.architecture).appimage
    except ValueError as exc:
        raise AppImagePackagingError(str(exc)) from exc
    tool = acquire_appimagetool(ctx)
    toolchain.build_appimage(tool, appdir, output, target_arch)
    if output.is_file():
        output.chmod(0o755)
    _verify_finished_appimage(ctx, output, workspace, toolchain, runtime)
    return output


def verify_appimage_package(
    ctx: Context,
    package: Path,
    workspace: Path,
    toolchain: LinuxToolchain,
) -> None:
    """Inspect an AppImage recovered without its build-time payload inventory."""
    _verify_finished_appimage(ctx, package, workspace, toolchain, None)


def acquire_appimagetool(ctx: Context) -> Path:
    """Return checksum-verified tool bytes from a versioned managed cache.

    Downloads land in a sibling temporary file and become cache-visible only
    after hashing. This prevents an interrupted request from poisoning later
    builds that would otherwise trust cache-file existence.
    """
    host_arch = get_platform_arch()
    pin = _APPIMAGETOOL_PINS.get(host_arch)
    if pin is None:
        supported = ", ".join(sorted(_APPIMAGETOOL_PINS))
        raise AppImagePackagingError(
            f"No appimagetool pin for host architecture {host_arch}; "
            f"supported: {supported}"
        )

    cache_dir = Path(ctx.root_dir) / "build" / "tools" / "appimagetool" / pin.version
    cache_dir.mkdir(parents=True, exist_ok=True)
    cached = cache_dir / pin.filename
    if cached.is_symlink():
        cached.unlink()
    if cached.is_file() and _sha256(cached) == pin.sha256:
        cached.chmod(0o755)
        return cached
    if cached.exists():
        if cached.is_dir():
            raise AppImagePackagingError(
                f"appimagetool cache path is a directory: {cached}"
            )
        cached.unlink()

    temporary: Path | None = None
    try:
        descriptor, temporary_name = tempfile.mkstemp(
            dir=cache_dir,
            prefix=f".{pin.filename}.",
            suffix=".part",
        )
        os.close(descriptor)
        temporary = Path(temporary_name)
        _DOWNLOADER.download(pin.url, temporary)
        actual = _sha256(temporary)
        if actual != pin.sha256:
            raise AppImagePackagingError(
                f"appimagetool checksum mismatch for {pin.filename}: "
                f"expected {pin.sha256}, got {actual}"
            )
        temporary.chmod(0o755)
        os.replace(temporary, cached)
        temporary = None
        return cached
    except AppImagePackagingError:
        raise
    except Exception as exc:
        raise AppImagePackagingError(
            f"Could not download pinned appimagetool {pin.version}: {exc}"
        ) from exc
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def _install_appimage_policy(ctx: Context, appdir: Path) -> None:
    app_dir = absolute_linux_path(
        ctx.product.linux.appimage_dir,
        "AppImage runtime path",
    )
    browser = linux_identifier(ctx.BROWSEROS_APP_NAME, "browser executable")
    display_name = single_line(ctx.product.display_name, "product display name")
    icon_name = linux_identifier(ctx.product.linux.icon_name, "icon name")
    desktop_id = linux_identifier(ctx.product.linux.desktop_id, "desktop id")

    applications = appdir / "usr/share/applications"
    write_asset(
        applications / desktop_id,
        "desktop",
        {
            "DISPLAY_NAME": display_name,
            "EXEC": single_line(f"{app_dir}/{browser} %U", "desktop Exec"),
            "ICON_NAME": icon_name,
        },
    )
    root_desktop = appdir / desktop_id
    write_asset(
        root_desktop,
        "desktop",
        {
            "DISPLAY_NAME": display_name,
            "EXEC": "AppRun %U",
            "ICON_NAME": icon_name,
        },
    )
    write_asset(
        appdir / "AppRun",
        "apprun",
        {"APP_DIR": app_dir, "BROWSER_NAME": browser},
    )
    _copy_icons(ctx, appdir)


def _copy_icons(ctx: Context, appdir: Path) -> None:
    icon_name = linux_identifier(ctx.product.linux.icon_name, "icon name")
    root_source = product_icon_source(ctx)
    shutil.copy2(root_source, appdir / f"{icon_name}.png")
    copy_hicolor_icons(ctx, appdir / "usr/share/icons/hicolor")


def _verify_finished_appimage(
    ctx: Context,
    package: Path,
    workspace: Path,
    toolchain: LinuxToolchain,
    runtime: VerifiedRuntime | None,
) -> None:
    if not package.is_file() or package.stat().st_size == 0:
        raise AppImagePackagingError(f"AppImage was not created or is empty: {package}")
    with package.open("rb") as source:
        if source.read(4) != b"\x7fELF":
            raise AppImagePackagingError(f"AppImage lacks an ELF header: {package}")
    mode = package.stat().st_mode & 0o777
    if mode != 0o755:
        raise AppImagePackagingError(
            f"AppImage mode is {mode:o}; expected executable mode 755: {package}"
        )

    if toolchain.can_execute_target(ctx.architecture):
        extracted = workspace / "appimage-extracted"
        toolchain.extract_appimage(package, extracted)
        _verify_appdir(ctx, extracted, runtime)
        runtime_root = _inside(extracted, ctx.product.linux.appimage_dir)
        try:
            unresolved = toolchain.unresolved_libraries(
                runtime_root / ctx.BROWSEROS_APP_NAME,
                runtime_root,
            )
        except ToolExecutionError as exc:
            raise AppImagePackagingError(
                f"Extracted AppImage runtime library check failed: {exc}"
            ) from exc
        if unresolved:
            raise AppImagePackagingError(
                "Extracted AppImage runtime has unresolved libraries:\n- "
                + "\n- ".join(unresolved)
            )


def _verify_appdir(
    ctx: Context,
    appdir: Path,
    runtime: VerifiedRuntime | None,
) -> None:
    runtime_root = _inside(appdir, ctx.product.linux.appimage_dir)
    try:
        verify_runtime_layout(ctx, runtime_root, sandbox_mode=0o4755)
        if runtime is not None:
            runtime.verify_materialized(runtime_root, sandbox_mode=0o4755)
    except RuntimePayloadError as exc:
        raise AppImagePackagingError(str(exc)) from exc

    required = (
        appdir / "AppRun",
        appdir / ctx.product.linux.desktop_id,
        appdir / f"{ctx.product.linux.icon_name}.png",
        appdir / "usr/share/applications" / ctx.product.linux.desktop_id,
    )
    missing = [str(path) for path in required if not path.is_file()]
    if missing:
        raise AppImagePackagingError(
            "AppImage layout is missing required file(s):\n- " + "\n- ".join(missing)
        )
    icon_root = appdir / "usr/share/icons/hicolor"
    if not any(icon_root.glob(f"*x*/apps/{ctx.product.linux.icon_name}.png")):
        raise AppImagePackagingError(
            f"AppImage layout has no hicolor product icon under {icon_root}"
        )
    if ((appdir / "AppRun").stat().st_mode & 0o777) != 0o755:
        raise AppImagePackagingError("AppRun must have mode 0755")
    root_desktop = (appdir / ctx.product.linux.desktop_id).read_text(encoding="utf-8")
    if "Exec=AppRun %U" not in root_desktop:
        raise AppImagePackagingError("AppImage root desktop entry must launch AppRun")


def _inside(root: Path, absolute_path: str) -> Path:
    validated = absolute_linux_path(absolute_path, "package path")
    return root.joinpath(*Path(validated.lstrip("/")).parts)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()
