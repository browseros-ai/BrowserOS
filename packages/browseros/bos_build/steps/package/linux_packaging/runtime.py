"""Assemble the single verified runtime consumed by both Linux formats.

Required/optional Chromium outputs live here so AppImage and Debian cannot
silently drift. A content-and-mode inventory makes the verified payload an
immutable handoff even though its representation is a temporary directory.
"""

from __future__ import annotations

import hashlib
import json
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

from ....products.server_binaries import server_bundles_for_product
from .policy import PolicyAssetError, product_icon_source
from .system import LinuxToolchain, ToolExecutionError

if TYPE_CHECKING:
    from ....core.context import Context


class RuntimePayloadError(RuntimeError):
    """The Chromium output cannot become a trustworthy package payload."""


_REQUIRED_FILES = (
    "chrome_crashpad_handler",
    "chrome_sandbox",
    "icudtl.dat",
    "v8_context_snapshot.bin",
    "chrome_100_percent.pak",
    "chrome_200_percent.pak",
    "resources.pak",
    # Chromium's Linux bundle loads ANGLE and the native-dialog shims at
    # runtime rather than linking them into the browser executable, so `ldd`
    # alone cannot detect their absence.
    "libEGL.so",
    "libGLESv2.so",
    "libqt5_shim.so",
    "libqt6_shim.so",
)

# These files vary across Chromium configurations. Their optional status is
# explicit so adding a new required capability is a reviewable policy change.
_OPTIONAL_FILES = (
    "chromedriver",
    "libvk_swiftshader.so",
    "libvulkan.so.1",
    "vk_swiftshader_icd.json",
    "snapshot_blob.bin",
)
_REQUIRED_DIRECTORIES = ("browseros_extensions", "locales")
_OPTIONAL_DIRECTORIES = ("MEIPreload",)
_EXECUTABLE_FILES = ("chrome_crashpad_handler", "chrome_sandbox", "chromedriver")


@dataclass(frozen=True)
class PayloadFile:
    """One immutable file record in a verified runtime."""

    relative_path: str
    size: int
    sha256: str
    mode: int


@dataclass(frozen=True)
class VerifiedRuntime:
    """Canonical payload plus the inventory adapters must preserve."""

    root: Path
    browser_name: str
    files: tuple[PayloadFile, ...]
    digest: str

    def assert_unchanged(self) -> None:
        current_files, current_digest = _inventory(self.root)
        if current_files != self.files or current_digest != self.digest:
            raise RuntimePayloadError(
                "Verified Linux runtime changed after validation; "
                "refusing to package mutable input"
            )

    def materialize(self, destination: Path, *, sandbox_mode: int) -> None:
        """Copy the verified payload and apply the format-owned sandbox mode.

        Hard links are intentionally forbidden: AppImage needs setuid mode while
        the Debian archive ships 0755 and promotes it in postinst. A chmod through
        a shared inode would mutate the other adapter's supposedly stable input.
        """
        self.assert_unchanged()
        shutil.copytree(self.root, destination)
        (destination / "chrome_sandbox").chmod(sandbox_mode)
        _verify_materialized(self, destination, sandbox_mode=sandbox_mode)

    def verify_materialized(self, destination: Path, *, sandbox_mode: int) -> None:
        """Prove an adapter or extracted package preserved the verified payload."""
        _verify_materialized(self, destination, sandbox_mode=sandbox_mode)


def assemble_verified_runtime(
    ctx: Context,
    destination: Path,
    toolchain: LinuxToolchain,
) -> VerifiedRuntime:
    """Select, copy, mode, inventory, and library-check one Chromium runtime."""
    source = Path(ctx.chromium_src) / ctx.out_dir
    missing = _missing_inputs(ctx, source)
    if missing:
        raise RuntimePayloadError(
            "Linux runtime is missing required input(s):\n- " + "\n- ".join(missing)
        )

    destination.mkdir(parents=True, exist_ok=False)
    file_names = (ctx.BROWSEROS_APP_NAME, *_REQUIRED_FILES, *_OPTIONAL_FILES)
    for name in file_names:
        candidate = source / name
        if candidate.is_file():
            shutil.copy2(candidate, destination / name)

    directory_names = [*_REQUIRED_DIRECTORIES, *_OPTIONAL_DIRECTORIES]
    directory_names.extend(
        bundle.chromium_output_root
        for bundle in server_bundles_for_product(ctx.product.id)
        if (source / bundle.chromium_output_root).is_dir()
    )
    for name in directory_names:
        candidate = source / name
        if candidate.is_dir():
            shutil.copytree(candidate, destination / name)

    (destination / ctx.BROWSEROS_APP_NAME).chmod(0o755)
    for name in _EXECUTABLE_FILES:
        executable = destination / name
        if executable.is_file():
            executable.chmod(0o755)

    files, digest = _inventory(destination)
    runtime = VerifiedRuntime(
        root=destination,
        browser_name=ctx.BROWSEROS_APP_NAME,
        files=files,
        digest=digest,
    )

    if toolchain.can_execute_target(ctx.architecture):
        try:
            unresolved = toolchain.unresolved_libraries(
                destination / ctx.BROWSEROS_APP_NAME,
                destination,
            )
        except ToolExecutionError as exc:
            raise RuntimePayloadError(
                f"Linux runtime library check failed: {exc}"
            ) from exc
        if unresolved:
            raise RuntimePayloadError(
                "Linux runtime has unresolved shared libraries:\n- "
                + "\n- ".join(unresolved)
            )

    return runtime


def verify_runtime_layout(
    ctx: Context,
    root: Path,
    *,
    sandbox_mode: int,
) -> None:
    """Check the required runtime layout when no build-time inventory exists.

    This is used only for sliced recovery, where the original temporary payload
    is unavailable and the finished archive must establish its own contents.
    """
    _verify_packaged_tree_entries(root)
    missing: list[str] = []
    for name in (ctx.BROWSEROS_APP_NAME, *_REQUIRED_FILES):
        if not (root / name).is_file():
            missing.append(str(root / name))
    for name in _REQUIRED_DIRECTORIES:
        directory = root / name
        if not _directory_has_files(directory):
            missing.append(f"{directory}/")
    for bundle in server_bundles_for_product(ctx.product.id):
        directory = root / bundle.chromium_output_root
        if bundle.required_in_chromium_output and (not _directory_has_files(directory)):
            missing.append(f"{directory}/")
    if missing:
        raise RuntimePayloadError(
            "Packaged Linux runtime is missing required content:\n- "
            + "\n- ".join(missing)
        )

    expected_modes = {
        ctx.BROWSEROS_APP_NAME: 0o755,
        "chrome_crashpad_handler": 0o755,
        "chrome_sandbox": sandbox_mode,
    }
    for name, expected_mode in expected_modes.items():
        actual_mode = (root / name).stat().st_mode & 0o7777
        if actual_mode != expected_mode:
            raise RuntimePayloadError(
                f"Packaged runtime mode for {name} is {actual_mode:o}; "
                f"expected {expected_mode:o}"
            )


def _missing_inputs(ctx: Context, source: Path) -> list[str]:
    missing: list[str] = []
    required_files = (ctx.BROWSEROS_APP_NAME, *_REQUIRED_FILES)
    for name in required_files:
        if not (source / name).is_file():
            missing.append(str(source / name))

    for name in _REQUIRED_DIRECTORIES:
        directory = source / name
        if not _directory_has_files(directory):
            missing.append(f"{directory}/")

    for bundle in server_bundles_for_product(ctx.product.id):
        directory = source / bundle.chromium_output_root
        if bundle.required_in_chromium_output and (not _directory_has_files(directory)):
            missing.append(f"{directory}/")

    try:
        product_icon_source(ctx)
    except PolicyAssetError as exc:
        missing.append(str(exc))
    return missing


def _directory_has_files(directory: Path) -> bool:
    return directory.is_dir() and any(path.is_file() for path in directory.rglob("*"))


def _verify_packaged_tree_entries(root: Path) -> None:
    """Reject links and special files that could escape an installed payload.

    Chromium-output links are dereferenced while assembling the canonical
    runtime. A link that reappears after archive extraction therefore means the
    finished package no longer represents that self-contained runtime.
    """
    if root.is_symlink():
        raise RuntimePayloadError(
            f"Packaged Linux runtime contains unsupported symbolic link: {root}"
        )
    for path in root.rglob("*"):
        if path.is_symlink():
            raise RuntimePayloadError(
                f"Packaged Linux runtime contains unsupported symbolic link: {path}"
            )
        if not path.is_dir() and not path.is_file():
            raise RuntimePayloadError(
                f"Packaged Linux runtime contains unsupported filesystem entry: {path}"
            )


def _inventory(root: Path) -> tuple[tuple[PayloadFile, ...], str]:
    files: list[PayloadFile] = []
    for path in sorted(root.rglob("*")):
        # Source copying dereferences valid links. Rejecting any link that
        # survives into the canonical payload prevents a package from referring
        # to a build-host path that will not exist on an installed machine.
        if path.is_symlink():
            raise RuntimePayloadError(
                f"Linux runtime contains unsupported symbolic link: {path}"
            )
        if path.is_dir():
            continue
        if not path.is_file():
            raise RuntimePayloadError(
                f"Linux runtime contains unsupported filesystem entry: {path}"
            )
        files.append(
            PayloadFile(
                relative_path=path.relative_to(root).as_posix(),
                size=path.stat().st_size,
                sha256=_sha256(path),
                mode=path.stat().st_mode & 0o7777,
            )
        )

    encoded = json.dumps(
        [(item.relative_path, item.size, item.sha256, item.mode) for item in files],
        separators=(",", ":"),
    ).encode("utf-8")
    return tuple(files), hashlib.sha256(encoded).hexdigest()


def _verify_materialized(
    runtime: VerifiedRuntime,
    destination: Path,
    *,
    sandbox_mode: int,
) -> None:
    actual, _ = _inventory(destination)
    expected = tuple(
        PayloadFile(
            relative_path=item.relative_path,
            size=item.size,
            sha256=item.sha256,
            mode=(
                sandbox_mode if item.relative_path == "chrome_sandbox" else item.mode
            ),
        )
        for item in runtime.files
    )
    if actual != expected:
        raise RuntimePayloadError(
            f"Materialized Linux runtime does not match verified payload: {destination}"
        )


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()
