"""Orchestrate the all-or-nothing Linux packaging lifecycle.

This module owns correlation between AppImage and Debian outputs. In-process
builds use the artifact registry; deliberately sliced runs may recover only the
two exact product-owned filenames from disk.
"""

from __future__ import annotations

import os
import stat
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

from .appimage import (
    AppImagePackagingError,
    build_appimage,
    verify_appimage_package,
)
from .debian import (
    DebianPackagingError,
    build_debian_package,
    verify_debian_package,
)
from .policy import PolicyAssetError, linux_identifier
from .runtime import RuntimePayloadError, assemble_verified_runtime
from .system import SubprocessLinuxToolchain, ToolExecutionError

if TYPE_CHECKING:
    from ....core.context import Context


class LinuxPackagingError(RuntimeError):
    """A payload, format, verification, publication, or recovery failure."""


@dataclass(frozen=True)
class LinuxArtifactPair:
    """The indivisible successful result of Linux release packaging."""

    appimage: Path
    deb: Path

    @property
    def paths(self) -> tuple[Path, Path]:
        """Return artifacts in stable release-metadata order."""
        return (self.appimage, self.deb)


_TOOLCHAIN = SubprocessLinuxToolchain()


def build_linux_artifacts(ctx: Context) -> LinuxArtifactPair:
    """Build, verify, publish, and register one complete Linux package pair."""
    dist_dir = Path(ctx.get_dist_dir())
    try:
        dist_dir.mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        raise LinuxPackagingError(
            f"workspace: could not create Linux distribution directory: {exc}"
        ) from exc
    expected = _expected_pair(ctx)
    try:
        with tempfile.TemporaryDirectory(
            dir=dist_dir,
            prefix=".linux-package-",
        ) as temporary:
            workspace = Path(temporary)
            runtime = assemble_verified_runtime(
                ctx,
                workspace / "runtime",
                _TOOLCHAIN,
            )
            candidates = LinuxArtifactPair(
                appimage=workspace / expected.appimage.name,
                deb=workspace / expected.deb.name,
            )
            try:
                build_appimage(
                    ctx,
                    runtime,
                    workspace,
                    candidates.appimage,
                    _TOOLCHAIN,
                )
            except (
                AppImagePackagingError,
                PolicyAssetError,
                RuntimePayloadError,
                ToolExecutionError,
                OSError,
            ) as exc:
                raise LinuxPackagingError(f"appimage: {exc}") from exc
            try:
                build_debian_package(
                    ctx,
                    runtime,
                    workspace,
                    candidates.deb,
                    _TOOLCHAIN,
                )
            except (
                DebianPackagingError,
                PolicyAssetError,
                RuntimePayloadError,
                ToolExecutionError,
                OSError,
            ) as exc:
                raise LinuxPackagingError(f"deb: {exc}") from exc

            _publish_pair_with_rollback(candidates, expected, workspace)
    except RuntimePayloadError as exc:
        raise LinuxPackagingError(f"payload: {exc}") from exc
    except OSError as exc:
        raise LinuxPackagingError(f"workspace: {exc}") from exc

    # The existing two keys are a compatibility interface for resume and later
    # steps. They are written only after both final paths have been committed.
    ctx.artifact_registry.add("appimage", expected.appimage)
    ctx.artifact_registry.add("deb", expected.deb)
    return expected


def require_linux_artifacts(ctx: Context) -> LinuxArtifactPair:
    """Return a complete exact pair from the registry or deliberate disk recovery.

    Registry absence is the build runner's established signal for a sliced
    invocation. A partial registry is therefore corruption, not permission to
    mix an in-process result with an unrelated file found on disk.
    """
    registry = ctx.artifact_registry
    registered_appimage = registry.get("appimage")
    registered_deb = registry.get("deb")
    # Key presence—not value truthiness—distinguishes a deliberate sliced run
    # from corrupt in-process state. Present invalid values must fail locally.
    registered = (registry.has("appimage"), registry.has("deb"))

    if any(registered) and not all(registered):
        present = "appimage" if registered[0] else "deb"
        missing = "deb" if registered[0] else "appimage"
        raise LinuxPackagingError(
            "Linux artifact registry is partial: "
            f"found {present}, missing {missing}; refusing disk fallback"
        )

    expected = _expected_pair(ctx)
    if all(registered):
        pair = LinuxArtifactPair(
            appimage=_registered_path("appimage", registered_appimage),
            deb=_registered_path("deb", registered_deb),
        )
        _validate_exact_pair(pair, expected)
        return pair

    _validate_exact_pair(expected, expected)
    try:
        with tempfile.TemporaryDirectory(
            dir=expected.appimage.parent,
            prefix=".linux-recovery-",
        ) as temporary:
            workspace = Path(temporary)
            verify_appimage_package(
                ctx,
                expected.appimage,
                workspace,
                _TOOLCHAIN,
            )
            verify_debian_package(
                ctx,
                expected.deb,
                workspace,
                _TOOLCHAIN,
            )
    except (
        AppImagePackagingError,
        DebianPackagingError,
        RuntimePayloadError,
        ToolExecutionError,
        OSError,
    ) as exc:
        raise LinuxPackagingError(
            f"Linux disk recovery verification failed: {exc}"
        ) from exc

    # Registration is intentionally the last handoff. No downstream step can
    # observe a recovered half-pair while validation is still in progress.
    ctx.artifact_registry.add("appimage", expected.appimage)
    ctx.artifact_registry.add("deb", expected.deb)
    return expected


def _expected_pair(ctx: Context) -> LinuxArtifactPair:
    dist_dir = Path(ctx.get_dist_dir())
    return LinuxArtifactPair(
        appimage=dist_dir / _artifact_filename(ctx, "appimage"),
        deb=dist_dir / _artifact_filename(ctx, "deb"),
    )


def _artifact_filename(ctx: Context, kind: str) -> str:
    filename = ctx.get_artifact_name(kind)
    valid = isinstance(filename, str) and Path(filename).name == filename
    if valid:
        try:
            linux_identifier(filename, f"Linux {kind} artifact name")
        except PolicyAssetError:
            valid = False
    if not valid:
        raise LinuxPackagingError(
            f"Linux {kind} artifact name must be one safe filename, got {filename!r}"
        )
    assert isinstance(filename, str)
    return filename


def _registered_path(name: str, value: object) -> Path:
    if not isinstance(value, Path):
        raise LinuxPackagingError(
            f"Linux artifact registry value '{name}' must be a Path, "
            f"got {type(value).__name__}"
        )
    return value


def _validate_exact_pair(
    pair: LinuxArtifactPair,
    expected: LinuxArtifactPair,
) -> None:
    problems: list[str] = []
    for kind, candidate, expected_path in (
        ("AppImage", pair.appimage, expected.appimage),
        ("DEB", pair.deb, expected.deb),
    ):
        if candidate.is_symlink():
            problems.append(f"{kind} is missing or not a regular file: {candidate}")
            continue
        if expected_path.is_symlink():
            problems.append(f"{kind} expected path is a symbolic link: {expected_path}")
            continue
        try:
            actual_location = candidate.resolve(strict=False)
            expected_location = expected_path.resolve(strict=False)
        except (OSError, RuntimeError) as exc:
            problems.append(f"{kind} path cannot be resolved: {candidate}: {exc}")
            continue
        if actual_location != expected_location:
            problems.append(
                f"{kind} path is {candidate}; expected exact path {expected_path}"
            )
            continue
        if not candidate.is_file():
            problems.append(f"{kind} is missing or not a regular file: {candidate}")
            continue
        if candidate.stat().st_size == 0:
            problems.append(f"{kind} is empty: {candidate}")
        executable_bits = candidate.stat().st_mode & (
            stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
        )
        mode = candidate.stat().st_mode & 0o777
        if kind == "AppImage" and mode != 0o755:
            problems.append(
                f"AppImage mode is {mode:o}; expected executable mode 755: {candidate}"
            )
        if kind == "DEB" and executable_bits:
            problems.append(f"DEB must not be executable: {candidate}")
        elif kind == "DEB" and mode != 0o644:
            problems.append(
                f"DEB mode is {mode:o}; expected archive mode 644: {candidate}"
            )

    if problems:
        raise LinuxPackagingError(
            "Linux artifact pair is invalid:\n- " + "\n- ".join(problems)
        )


def _publish_pair_with_rollback(
    candidates: LinuxArtifactPair,
    final: LinuxArtifactPair,
    workspace: Path,
) -> None:
    """Replace two final files with rollback for ordinary process failures.

    A filesystem has no two-path atomic rename. We therefore build and verify
    both candidates first, keep old files in a same-filesystem backup, and roll
    back exceptions or cooperative cancellation. A hard process or power loss
    can still interrupt between renames; `require_linux_artifacts` detects and
    rejects that partial state.
    """
    # Format inspectors perform richer checks; this last generic gate ensures
    # no link, empty file, or wrong mode can trigger mutation of an older pair.
    _validate_exact_pair(candidates, candidates)
    backup_root = workspace / "publication-backup"
    backup_root.mkdir()
    backups: list[tuple[Path, Path]] = []
    published: list[Path] = []
    try:
        for destination in final.paths:
            if destination.exists():
                if not destination.is_file():
                    raise LinuxPackagingError(
                        f"Linux artifact destination is not a file: {destination}"
                    )
                backup = backup_root / destination.name
                os.replace(destination, backup)
                backups.append((backup, destination))

        for candidate, destination in zip(candidates.paths, final.paths, strict=True):
            if not candidate.is_file():
                raise LinuxPackagingError(
                    f"Verified Linux package candidate disappeared: {candidate}"
                )
            os.replace(candidate, destination)
            published.append(destination)

        # Keep the backups live until the exact final paths themselves pass.
        # Validation after this transaction would report corruption correctly
        # but would be too late to put an older release pair back.
        _validate_exact_pair(final, final)
    # TemporaryDirectory deletes the backup while unwinding. Catch
    # BaseException so KeyboardInterrupt/SystemExit cannot bypass restoration;
    # cancellation is re-raised after the old pair is safe again.
    except BaseException as exc:
        rollback_problems: list[str] = []
        for destination in published:
            try:
                destination.unlink(missing_ok=True)
            except OSError as rollback_exc:
                rollback_problems.append(
                    f"could not remove new {destination}: {rollback_exc}"
                )
        for backup, destination in backups:
            try:
                if backup.exists():
                    os.replace(backup, destination)
            except OSError as rollback_exc:
                rollback_problems.append(
                    f"could not restore {destination}: {rollback_exc}"
                )
        if rollback_problems:
            raise LinuxPackagingError(
                f"publication failed ({exc}) and rollback was incomplete:\n- "
                + "\n- ".join(rollback_problems)
            ) from exc
        if isinstance(exc, LinuxPackagingError):
            raise
        if not isinstance(exc, Exception):
            raise
        raise LinuxPackagingError(f"publication: {exc}") from exc
