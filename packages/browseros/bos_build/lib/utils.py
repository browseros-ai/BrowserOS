#!/usr/bin/env python3
"""
Shared utilities for the build system
"""

import os
import sys
import json
import shlex
import subprocess
import shutil
from collections.abc import Iterable, Mapping, Sequence
from pathlib import Path
from typing import Optional, List, Dict, Union

from .env import SENSITIVE_ENV_VARS

# Import logging functions from logger module - re-exported for other modules
from .logger import (  # noqa: F401
    log_info,
    log_error,
    log_warning,
    log_success,
    _log_to_file,
)


REDACTION_MARKER = "***"
SENSITIVE_COMMAND_FLAGS: frozenset[str] = frozenset(
    {
        "-p",
        "-credential_id",
        "-password",
        "--password",
        "-passphrase",
        "--passphrase",
        "-totp_secret",
        "--totp_secret",
        "--totp-secret",
        "-secret",
        "--secret",
        "-token",
        "--token",
        "--api-key",
        "--api_key",
        "--private-key",
        "--private_key",
    }
)


class _RedactedCalledProcessError(subprocess.CalledProcessError):
    """Keep raw failure metadata while making implicit display safe."""

    def __init__(
        self,
        returncode: int,
        cmd: Sequence[str],
        redacted_cmd: str,
        output: Optional[str] = None,
        stderr: Optional[str] = None,
        redacted_output: Optional[str] = None,
        redacted_stderr: Optional[str] = None,
    ) -> None:
        super().__init__(returncode, cmd, output, stderr)
        self.redacted_cmd = redacted_cmd
        self.redacted_output = redacted_output
        self.redacted_stderr = redacted_stderr

    def _display_exception(self) -> subprocess.CalledProcessError:
        return subprocess.CalledProcessError(
            self.returncode,
            self.redacted_cmd,
            self.redacted_output,
            self.redacted_stderr,
        )

    def __str__(self) -> str:
        return str(self._display_exception())

    def __repr__(self) -> str:
        return repr(self._display_exception())


def _without_wrapping_quotes(value: str) -> str:
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
        return value[1:-1]
    return value


def get_command_secret_values(cmd: Sequence[str]) -> tuple[str, ...]:
    """Return values associated with known credential flags in ``cmd``."""
    values: list[str] = []
    redact_next = False

    for part in cmd:
        token = str(part)
        if redact_next:
            values.extend((token, _without_wrapping_quotes(token)))
            redact_next = False
            continue

        flag, separator, value = token.partition("=")
        if separator and flag.casefold() in SENSITIVE_COMMAND_FLAGS:
            values.extend((value, _without_wrapping_quotes(value)))
        elif token.casefold() in SENSITIVE_COMMAND_FLAGS:
            redact_next = True

    return tuple(value for value in values if value)


def redact_sensitive_text(
    text: str,
    additional_secrets: Iterable[str] = (),
    env: Optional[Mapping[str, str]] = None,
) -> str:
    """Mask exact credential values in text without changing its source data."""
    environment = env if env is not None else os.environ
    secret_values = list(additional_secrets)
    secret_values.extend(
        value
        for name in SENSITIVE_ENV_VARS
        if (value := environment.get(name))
    )

    normalized_values: set[str] = set()
    for value in secret_values:
        value = str(value)
        if not value:
            continue
        for candidate in {value, _without_wrapping_quotes(value)}:
            if not candidate:
                continue
            components = {candidate, *(line for line in candidate.splitlines() if line)}
            for component in components:
                normalized_values.update(
                    {
                        component,
                        component.encode("unicode_escape").decode("ascii"),
                        json.dumps(component)[1:-1],
                        repr(component),
                        shlex.quote(component),
                        subprocess.list2cmdline([component]),
                    }
                )

    redacted = str(text)
    for value in sorted(normalized_values, key=len, reverse=True):
        redacted = redacted.replace(value, REDACTION_MARKER)
    return redacted


def redact_command(
    cmd: Sequence[str],
    env: Optional[Mapping[str, str]] = None,
) -> str:
    """Build a command string for logs while leaving executable argv untouched."""
    displayed: list[str] = []
    redact_next = False

    for part in cmd:
        token = str(part)
        if redact_next:
            displayed.append(REDACTION_MARKER)
            redact_next = False
            continue

        flag, separator, _ = token.partition("=")
        if separator and flag.casefold() in SENSITIVE_COMMAND_FLAGS:
            displayed.append(f"{flag}={REDACTION_MARKER}")
        else:
            displayed.append(token)
            redact_next = token.casefold() in SENSITIVE_COMMAND_FLAGS

    return redact_sensitive_text(
        " ".join(displayed),
        get_command_secret_values(cmd),
        env,
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
    process_env = env or os.environ
    secret_values = get_command_secret_values(cmd)
    cmd_str = redact_command(cmd, process_env)
    _log_to_file(f"RUN_COMMAND: 🔧 Running: {cmd_str}")
    log_info(f"🔧 Running: {cmd_str}")

    try:
        # Always use Popen for real-time streaming and capturing
        process = subprocess.Popen(
            cmd,
            cwd=cwd,
            env=process_env,
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
                safe_line = redact_sensitive_text(line, secret_values, process_env)
                print(safe_line)  # Print to console in real-time
                _log_to_file(f"RUN_COMMAND: STDOUT: {safe_line}")  # Log to file
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
            safe_output = redact_sensitive_text(
                result.stdout, secret_values, process_env
            )
            safe_stderr = redact_sensitive_text(
                result.stderr, secret_values, process_env
            )
            raise _RedactedCalledProcessError(
                process.returncode,
                cmd,
                cmd_str,
                result.stdout,
                result.stderr,
                safe_output,
                safe_stderr,
            )

        return result

    except subprocess.CalledProcessError as e:
        _log_to_file(f"RUN_COMMAND: ❌ Command failed: {cmd_str}")
        _log_to_file(f"RUN_COMMAND: ❌ Exit code: {e.returncode}")

        if e.stdout:
            for line in e.stdout.strip().split("\n"):
                if line.strip():
                    safe_line = redact_sensitive_text(line, secret_values, process_env)
                    _log_to_file(f"RUN_COMMAND: STDOUT: {safe_line}")

        if e.stderr:
            for line in e.stderr.strip().split("\n"):
                if line.strip():
                    safe_line = redact_sensitive_text(line, secret_values, process_env)
                    _log_to_file(f"RUN_COMMAND: STDERR: {safe_line}")

        if check:
            log_error(f"Command failed: {cmd_str}")
            if e.stderr:
                safe_error = redact_sensitive_text(
                    e.stderr, secret_values, process_env
                )
                log_error(f"Error: {safe_error}")
            raise
        return e
    except Exception as e:
        safe_error = redact_sensitive_text(str(e), secret_values, process_env)
        _log_to_file(f"RUN_COMMAND: ❌ Unexpected error: {safe_error}")
        if check:
            log_error(f"Unexpected error running command: {cmd_str}")
            log_error(f"Error: {safe_error}")
        raise


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
