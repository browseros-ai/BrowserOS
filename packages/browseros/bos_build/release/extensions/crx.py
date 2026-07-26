#!/usr/bin/env python3
"""CRX packaging via chrome --pack-extension. Port of the actions repo's
packager.py; command assembly and process execution are split so tests
never need a real chrome."""

import os
import platform
import subprocess
import tempfile
from pathlib import Path
from typing import Callable, List, Optional

from ...lib.utils import log_info, log_success

_DARWIN_CANDIDATES = (
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    # User-level installs (e.g. the self-hosted mac runner keeps Chrome
    # in ~/Applications).
    "~/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "~/Applications/Chromium.app/Contents/MacOS/Chromium",
)
_LINUX_CANDIDATES = (
    "google-chrome-stable",
    "google-chrome",
    "chromium-browser",
    "chromium",
)
_WINDOWS_CANDIDATES = (
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    "chrome",
)


def _is_valid_binary(path: str) -> bool:
    p = Path(path).expanduser()
    if p.exists() and p.is_file():
        return os.access(p, os.X_OK)
    return subprocess.run(["which", path], capture_output=True).returncode == 0


def find_chrome_binary(
    preferred: Optional[str] = None,
    is_valid: Callable[[str], bool] = _is_valid_binary,
    platform_name: Optional[str] = None,
) -> str:
    """Resolve the chrome binary: explicit flag > CHROME_BINARY > candidates.

    An explicit flag that does not validate is an error, not a fallback —
    silently packing with a different chrome than the operator asked for is
    worse than failing.
    """
    if preferred:
        if is_valid(preferred):
            return preferred
        raise RuntimeError(f"Requested chrome binary is not usable: {preferred}")

    env_binary = os.environ.get("CHROME_BINARY")
    if env_binary and is_valid(env_binary):
        return env_binary

    system = platform_name or platform.system()
    if system == "Darwin":
        candidates = _DARWIN_CANDIDATES
    elif system == "Linux":
        candidates = _LINUX_CANDIDATES
    elif system == "Windows":
        candidates = _WINDOWS_CANDIDATES
    else:
        raise RuntimeError(f"Unsupported platform for CRX packing: {system}")

    for binary in candidates:
        expanded = str(Path(binary).expanduser())
        if is_valid(expanded):
            return expanded

    raise RuntimeError(
        "Chrome/Chromium binary not found — install Chrome, set CHROME_BINARY, "
        "or pass --chrome-binary"
    )


def pack_extension_command(
    chrome_binary: str, dist_dir: Path, key_path: Path
) -> List[str]:
    return [
        chrome_binary,
        f"--pack-extension={dist_dir.absolute()}",
        f"--pack-extension-key={key_path}",
    ]


def _run(cmd: List[str]) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True)


def pack_crx(
    dist_dir: Path,
    signing_key_contents: str,
    chrome_binary: str,
    output_path: Path,
    run: Callable[[List[str]], subprocess.CompletedProcess] = _run,
) -> Path:
    """Pack dist_dir into output_path with the given PEM key contents.

    The key lands in a mode-0600 temp file only for the duration of the
    chrome call; chrome writes <dist_dir>.crx next to the source, which is
    moved to output_path.
    """
    if not dist_dir.exists():
        raise FileNotFoundError(f"Distribution directory not found: {dist_dir}")
    if not (dist_dir / "manifest.json").exists():
        raise FileNotFoundError(f"No manifest.json in {dist_dir}")

    log_info(f"Packing CRX from {dist_dir} with {chrome_binary}")

    with tempfile.NamedTemporaryFile(mode="w", suffix=".pem", delete=False) as key_file:
        key_file.write(signing_key_contents)
        key_path = Path(key_file.name)

    try:
        result = run(pack_extension_command(chrome_binary, dist_dir, key_path))
        if result.returncode != 0:
            raise RuntimeError(
                f"chrome --pack-extension failed ({result.returncode}): {result.stderr}"
            )

        generated = Path(f"{dist_dir}.crx")
        if not generated.exists():
            raise RuntimeError(f"Expected crx not found after packing: {generated}")

        output_path.parent.mkdir(parents=True, exist_ok=True)
        generated.replace(output_path)
        log_success(
            f"CRX created: {output_path} ({output_path.stat().st_size / 1024:.1f} KB)"
        )
        return output_path
    finally:
        key_path.unlink(missing_ok=True)
