#!/usr/bin/env python3
"""
Shared utilities for the build system
"""

import os
import sys
import subprocess
import yaml
import shutil
from pathlib import Path
from typing import Optional, List, Dict, Union

# Import logging functions from logger module - re-exported for other modules
from .logger import (  # noqa: F401
    log_info,
    log_error,
    log_warning,
    log_success,
    _log_to_file,
)


# Platform detection functions
def IS_WINDOWS() -> bool:
    """Check if running on Windows"""
    return sys.platform == "win32"


def IS_MACOS() -> bool:
    """Check if running on macOS"""
    return sys.platform == "darwin"


def IS_LINUX() -> bool:
    """Check if running on Linux"""
    return sys.platform.startswith("linux")


def run_command(
    cmd: List[str],
    cwd: Optional[Path] = None,
    env: Optional[Dict] = None,
    check: bool = True,
) -> subprocess.CompletedProcess:
    """Run a command with real-time streaming output and full capture"""
    cmd_str = " ".join(cmd)
    _log_to_file(f"RUN_COMMAND: 🔧 Running: {cmd_str}")
    log_info(f"🔧 Running: {cmd_str}")

    try:
        # Always use Popen for real-time streaming and capturing
        process = subprocess.Popen(
            cmd,
            cwd=cwd,
            env=env or os.environ,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,  # Merge stderr into stdout
            text=True,
            # Native Windows tools emit ANSI-codepage bytes; strict decode
            # would raise mid-stream and kill a multi-hour build.
            errors="replace",
            bufsize=1,
            universal_newlines=True,
        )

        stdout_lines = []

        # Stream output line by line
        for line in iter(process.stdout.readline, ""):
            line = line.rstrip()
            if line:
                print(line)  # Print to console in real-time
                _log_to_file(f"RUN_COMMAND: STDOUT: {line}")  # Log to file
                stdout_lines.append(line)

        # Wait for process to complete
        process.wait()

        _log_to_file(
            f"RUN_COMMAND: ✅ Command completed with exit code: {process.returncode}"
        )

        # Create a CompletedProcess object with captured output
        result = subprocess.CompletedProcess(
            cmd,
            process.returncode,
            stdout="\n".join(stdout_lines) if stdout_lines else "",
            stderr="",
        )

        if check and process.returncode != 0:
            raise subprocess.CalledProcessError(
                process.returncode, cmd, result.stdout, result.stderr
            )

        return result

    except subprocess.CalledProcessError as e:
        _log_to_file(f"RUN_COMMAND: ❌ Command failed: {cmd_str}")
        _log_to_file(f"RUN_COMMAND: ❌ Exit code: {e.returncode}")

        if e.stdout:
            for line in e.stdout.strip().split("\n"):
                if line.strip():
                    _log_to_file(f"RUN_COMMAND: STDOUT: {line}")

        if e.stderr:
            for line in e.stderr.strip().split("\n"):
                if line.strip():
                    _log_to_file(f"RUN_COMMAND: STDERR: {line}")

        if check:
            log_error(f"Command failed: {cmd_str}")
            if e.stderr:
                log_error(f"Error: {e.stderr}")
            raise
        return e
    except Exception as e:
        _log_to_file(f"RUN_COMMAND: ❌ Unexpected error: {str(e)}")
        if check:
            log_error(f"Unexpected error running command: {cmd_str}")
            log_error(f"Error: {str(e)}")
        raise


def load_config(config_path: Path) -> Dict:
    """Load configuration from YAML file"""
    if not config_path.exists():
        log_error(f"Config file not found: {config_path}")
        raise FileNotFoundError(f"Config file not found: {config_path}")

    with open(config_path, "r") as f:
        config = yaml.safe_load(f)

    return config


# Platform-specific utilities
def get_platform() -> str:
    """Get platform name in a consistent format"""
    if IS_WINDOWS():
        return "windows"
    elif IS_MACOS():
        return "macos"
    elif IS_LINUX():
        return "linux"
    return "unknown"


def get_platform_arch() -> str:
    """Get default architecture for current platform"""
    if IS_WINDOWS():
        return "x64"
    elif IS_MACOS():
        # macOS can be arm64 or x64
        import platform

        return "arm64" if platform.machine() == "arm64" else "x64"
    elif IS_LINUX():
        # Linux can be x64 or arm64
        import platform

        machine = platform.machine()
        if machine in ["x86_64", "AMD64"]:
            return "x64"
        elif machine in ["aarch64", "arm64"]:
            return "arm64"
        else:
            # Default to x64 for unknown architectures
            return "x64"
    return "x64"


def get_executable_extension() -> str:
    """Get executable file extension for current platform"""
    return ".exe" if IS_WINDOWS() else ""


def get_app_extension() -> str:
    """Get application bundle extension for current platform"""
    if IS_MACOS():
        return ".app"
    elif IS_WINDOWS():
        return ".exe"
    return ""


def normalize_path(path: Union[str, Path]) -> Path:
    """Normalize path for current platform"""
    path = Path(path)
    if IS_WINDOWS():
        # Convert forward slashes to backslashes on Windows
        return Path(str(path).replace("/", "\\"))
    return path


def join_paths(*paths: Union[str, Path]) -> Path:
    """Join paths in a platform-aware way"""
    if not paths:
        return Path()

    result = Path(paths[0])
    for p in paths[1:]:
        result = result / p

    return normalize_path(result)


def safe_rmtree(path: Union[str, Path]) -> None:
    """Safely remove directory tree, handling Windows symlinks and junction points"""
    path = Path(path)

    if not path.exists():
        return

    if IS_WINDOWS():
        # On Windows, use rmdir for junctions and symlinks
        import stat

        def handle_remove_readonly(func, path, exc):
            """Error handler for Windows readonly files"""
            if os.path.exists(path):
                os.chmod(path, stat.S_IWRITE)
                func(path)

        # Try to remove as a junction/symlink first
        try:
            if path.is_symlink() or (path.is_dir() and os.path.islink(str(path))):
                path.unlink()
                return
        except Exception:
            pass

        # Fall back to rmtree with error handler
        shutil.rmtree(path, onerror=handle_remove_readonly)
    else:
        # On Unix-like systems, regular rmtree works fine
        shutil.rmtree(path)
