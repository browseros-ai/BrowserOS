#!/usr/bin/env python3
"""Standard single-architecture build module for BrowserOS"""

import os
import sys
import tempfile
import shutil
from pathlib import Path
from typing import List, Mapping, Optional
from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.utils import (
    run_command,
    log_info,
    log_success,
    log_warning,
    join_paths,
    IS_WINDOWS,
)

GB_PER_COMPILE_JOB = 4


def _windows_total_memory_gb() -> Optional[float]:
    """Total physical RAM in GB via GlobalMemoryStatusEx; None when unavailable."""
    if sys.platform != "win32":
        return None
    try:
        import ctypes

        class MEMORYSTATUSEX(ctypes.Structure):
            _fields_ = [
                ("dwLength", ctypes.c_uint32),
                ("dwMemoryLoad", ctypes.c_uint32),
                ("ullTotalPhys", ctypes.c_uint64),
                ("ullAvailPhys", ctypes.c_uint64),
                ("ullTotalPageFile", ctypes.c_uint64),
                ("ullAvailPageFile", ctypes.c_uint64),
                ("ullTotalVirtual", ctypes.c_uint64),
                ("ullAvailVirtual", ctypes.c_uint64),
                ("ullAvailExtendedVirtual", ctypes.c_uint64),
            ]

        status = MEMORYSTATUSEX()
        status.dwLength = ctypes.sizeof(MEMORYSTATUSEX)
        if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
            return None
        return status.ullTotalPhys / (1024**3)
    except Exception:
        return None


def compute_ninja_jobs(env: Optional[Mapping[str, str]] = None) -> Optional[int]:
    """Resolve the -j value: env override, else Windows RAM cap, else None (autoninja default)."""
    if env is None:
        env = os.environ

    override = env.get("BROWSEROS_NINJA_JOBS")
    if override is not None:
        try:
            jobs = int(override)
        except ValueError:
            jobs = 0
        if jobs > 0:
            log_info(f"Ninja parallelism: -j {jobs} (BROWSEROS_NINJA_JOBS override)")
            return jobs
        log_warning(f"Ignoring invalid BROWSEROS_NINJA_JOBS={override!r}")

    if not IS_WINDOWS():
        return None

    total_gb = _windows_total_memory_gb()
    if total_gb is None:
        log_warning(
            "Could not query physical memory; using autoninja default parallelism"
        )
        return None

    # Windows has no overcommit: official+ThinLTO clang-cl jobs peak ~4 GB each,
    # and one-job-per-core exhausts commit (LLVM ERROR: out of memory).
    jobs = max(1, int(total_gb) // GB_PER_COMPILE_JOB)
    cpus = os.cpu_count()
    if cpus:
        jobs = min(jobs, cpus)
    log_info(
        f"Ninja parallelism: -j {jobs} (capped by {int(total_gb)} GB RAM / "
        f"{GB_PER_COMPILE_JOB} GB per job; override with BROWSEROS_NINJA_JOBS)"
    )
    return jobs


def autoninja_command(
    out_dir: str, targets: List[str], env: Optional[Mapping[str, str]] = None
) -> List[str]:
    """Assemble the autoninja argv with the resolved -j parallelism applied."""
    cmd = ["autoninja.bat" if IS_WINDOWS() else "autoninja", "-C", out_dir]
    jobs = compute_ninja_jobs(env)
    if jobs is not None:
        cmd += ["-j", str(jobs)]
    else:
        log_info("Ninja parallelism: autoninja default")
    return cmd + list(targets)


@step("compile", phase="build")
class CompileModule(Step):
    produces = ["built_app"]
    requires = []
    description = "Build BrowserOS using autoninja"

    def validate(self, ctx: Context) -> None:
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

        if not ctx.browseros_chromium_version:
            raise ValidationError("BrowserOS chromium version not set")

        args_file = ctx.get_gn_args_file()
        if not args_file.exists():
            raise ValidationError(f"Build not configured - args.gn not found: {args_file}")

    def execute(self, ctx: Context) -> None:
        log_info("\n🔨 Building BrowserOS (this will take a while)...")

        self._create_version_file(ctx)

        run_command(
            autoninja_command(ctx.out_dir, ["chrome", "chromedriver"]),
            cwd=ctx.chromium_src,
        )

        app_path = ctx.get_chromium_app_path()
        built_app_path = app_path
        if not IS_WINDOWS():
            built_app_path = ctx.get_app_path()
            if app_path.exists() and not built_app_path.exists():
                shutil.move(str(app_path), str(built_app_path))

        ctx.artifact_registry.add("built_app", built_app_path)

        log_success("Build complete!")

    def _create_version_file(self, ctx: Context) -> None:
        parts = ctx.browseros_chromium_version.split(".")
        if len(parts) != 4:
            log_warning(f"Invalid version format: {ctx.browseros_chromium_version}")
            return

        version_content = f"MAJOR={parts[0]}\nMINOR={parts[1]}\nBUILD={parts[2]}\nPATCH={parts[3]}"

        with tempfile.NamedTemporaryFile(mode="w", delete=False) as temp_file:
            temp_file.write(version_content)
            temp_path = temp_file.name

        chrome_version_path = join_paths(ctx.chromium_src, "chrome", "VERSION")
        shutil.copy2(temp_path, chrome_version_path)
        Path(temp_path).unlink()

        log_info(f"Created VERSION file: {ctx.browseros_chromium_version}")


def build_target(ctx: Context, target: str) -> bool:
    """Build a specific target (e.g., mini_installer)"""
    log_info(f"\n🔨 Building target: {target}")

    run_command(autoninja_command(ctx.out_dir, [target]), cwd=ctx.chromium_src)

    log_success(f"Target {target} built successfully")
    return True
