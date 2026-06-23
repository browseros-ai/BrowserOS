#!/usr/bin/env python3
"""
Logging utilities for the build system
Provides consistent logging with Typer output and file logging
"""

import io
import sys
from contextlib import suppress

import typer
from datetime import datetime

# Windows pipes default to the ANSI codepage (cp1252), which cannot encode
# the emoji these helpers emit — the first log line killed the Windows CI
# build with UnicodeEncodeError. Degrade to escape sequences instead
# (backslashreplace matches stderr's native fallback behavior).
for _stream in (sys.stdout, sys.stderr):
    if isinstance(_stream, io.TextIOWrapper):
        with suppress(Exception):
            _stream.reconfigure(errors="backslashreplace")

# Global log file handle
_log_file = None


def _ensure_log_file():
    """Ensure log file is created with timestamp"""
    global _log_file
    if _log_file is None:
        from .paths import get_package_root

        # Create logs directory if it doesn't exist
        log_dir = get_package_root() / "logs"
        log_dir.mkdir(exist_ok=True)

        # Create log file with timestamp
        timestamp = datetime.now().strftime("%Y-%m-%d_%H-%M-%S")
        log_file_path = log_dir / f"build_{timestamp}.log"
        # Open with UTF-8 encoding to handle any characters
        _log_file = open(log_file_path, "w", encoding="utf-8")
        _log_file.write(
            f"BrowserOS Build Log - Started at {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}\n"
        )
        _log_file.write("=" * 80 + "\n\n")
    return _log_file


def _log_to_file(message: str):
    """Write message to log file with timestamp"""
    log_file = _ensure_log_file()
    timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    log_file.write(f"[{timestamp}] {message}\n")
    log_file.flush()


def log_info(message: str):
    """Print info message using Typer"""
    typer.echo(message)
    _log_to_file(f"INFO: {message}")


def log_warning(message: str):
    """Print warning message with color"""
    typer.secho(f"⚠️  {message}", fg=typer.colors.YELLOW)
    _log_to_file(f"WARNING: {message}")


def log_error(message: str):
    """Print error message to stderr with color"""
    typer.secho(f"❌ {message}", fg=typer.colors.RED, err=True)
    _log_to_file(f"ERROR: {message}")


def log_success(message: str):
    """Print success message with color"""
    typer.secho(f"✅ {message}", fg=typer.colors.GREEN)
    _log_to_file(f"SUCCESS: {message}")


def log_debug(message: str, enabled: bool = False):
    """Print debug message if enabled"""
    if enabled:
        typer.secho(f"🔍 {message}", fg=typer.colors.BLUE, dim=True)
        _log_to_file(f"DEBUG: {message}")


def close_log_file():
    """Close the log file if it's open"""
    global _log_file
    if _log_file:
        _log_file.close()
        _log_file = None


# Export all logging functions
__all__ = [
    'log_info',
    'log_warning',
    'log_error',
    'log_success',
    'log_debug',
    'close_log_file',
    '_log_to_file',  # Internal use by utils.run_command
]