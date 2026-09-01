"""Production adapter for Linux packaging subprocesses.

The packaging policy is deliberately kept out of this adapter. It only
translates domain-shaped operations into local commands so tests can replace
the process boundary without replacing our staging or verification logic.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path
from typing import Mapping, Protocol, Sequence

from ....lib.utils import get_platform_arch


class ToolExecutionError(RuntimeError):
    """A local packaging executable failed or returned unusable output."""


class LinuxToolchain(Protocol):
    """Domain operations supplied by Linux packaging executables."""

    def can_execute_target(self, architecture: str) -> bool: ...

    def unresolved_libraries(
        self,
        executable: Path,
        library_dir: Path,
    ) -> tuple[str, ...]: ...

    def build_appimage(
        self,
        tool: Path,
        appdir: Path,
        output: Path,
        architecture: str,
    ) -> None: ...

    def extract_appimage(self, package: Path, destination: Path) -> None: ...

    def build_deb(self, root: Path, output: Path) -> None: ...

    def deb_fields(
        self,
        package: Path,
        fields: Sequence[str],
    ) -> Mapping[str, str]: ...

    def extract_deb_data(self, package: Path, destination: Path) -> None: ...

    def extract_deb_control(self, package: Path, destination: Path) -> None: ...


class SubprocessLinuxToolchain:
    """Execute the real host tools used to build and inspect packages."""

    def can_execute_target(self, architecture: str) -> bool:
        return sys.platform.startswith("linux") and get_platform_arch() == architecture

    def unresolved_libraries(
        self,
        executable: Path,
        library_dir: Path,
    ) -> tuple[str, ...]:
        env = os.environ.copy()
        current = env.get("LD_LIBRARY_PATH", "")
        env["LD_LIBRARY_PATH"] = (
            f"{library_dir}:{current}" if current else str(library_dir)
        )
        # Missing-library detection parses loader text. Pinning the C locale
        # keeps that protocol stable on developer hosts with translated output.
        env["LC_ALL"] = "C"
        result = self._run(("ldd", str(executable)), env=env)
        output = f"{result.stdout}\n{result.stderr}"
        return tuple(
            line.strip() for line in output.splitlines() if "not found" in line.lower()
        )

    def build_appimage(
        self,
        tool: Path,
        appdir: Path,
        output: Path,
        architecture: str,
    ) -> None:
        env = os.environ.copy()
        env["ARCH"] = architecture
        # The pinned 1.9.1 tools bundle a zstd-only mksquashfs implementation.
        self._run(
            (str(tool), "--comp", "zstd", str(appdir), str(output)),
            env=env,
        )

    def extract_appimage(self, package: Path, destination: Path) -> None:
        work_dir = destination.parent
        extracted = work_dir / "squashfs-root"
        if extracted.exists():
            raise ToolExecutionError(
                f"AppImage extraction target already exists: {extracted}"
            )
        self._run((str(package), "--appimage-extract"), cwd=work_dir)
        if not extracted.is_dir():
            raise ToolExecutionError(
                f"AppImage did not create expected extraction root: {extracted}"
            )
        extracted.replace(destination)

    def build_deb(self, root: Path, output: Path) -> None:
        self._run(
            (
                "dpkg-deb",
                "--build",
                "--root-owner-group",
                str(root),
                str(output),
            )
        )

    def deb_fields(
        self,
        package: Path,
        fields: Sequence[str],
    ) -> Mapping[str, str]:
        values: dict[str, str] = {}
        for field in fields:
            result = self._run(("dpkg-deb", "--field", str(package), field))
            values[field] = result.stdout.strip()
        return values

    def extract_deb_data(self, package: Path, destination: Path) -> None:
        destination.mkdir(parents=True, exist_ok=False)
        self._run(("dpkg-deb", "--extract", str(package), str(destination)))

    def extract_deb_control(self, package: Path, destination: Path) -> None:
        destination.mkdir(parents=True, exist_ok=False)
        self._run(("dpkg-deb", "--control", str(package), str(destination)))

    @staticmethod
    def _run(
        command: Sequence[str],
        *,
        cwd: Path | None = None,
        env: Mapping[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        try:
            result = subprocess.run(
                command,
                cwd=cwd,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
        except OSError as exc:
            raise ToolExecutionError(
                f"Could not execute {' '.join(command)}: {exc}"
            ) from exc
        if result.returncode != 0:
            detail = result.stderr.strip() or result.stdout.strip() or "no output"
            raise ToolExecutionError(
                f"Command failed ({result.returncode}): {' '.join(command)}\n{detail}"
            )
        return result
