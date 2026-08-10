#!/usr/bin/env python3
"""CRX packaging via chrome --pack-extension. Port of the actions repo's
packager.py; command assembly and process execution are split so tests
never need a real chrome."""

import os
import platform
import subprocess
import tempfile
import hashlib
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


def _read_varint(data: bytes, offset: int) -> tuple[int, int]:
    value = 0
    shift = 0
    while offset < len(data) and shift < 70:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value, offset
        shift += 7
    raise ValueError("Invalid CRX protobuf varint")


def _protobuf_bytes_field(data: bytes, wanted: int) -> bytes:
    offset = 0
    while offset < len(data):
        key, offset = _read_varint(data, offset)
        field = key >> 3
        wire_type = key & 0x7
        if wire_type == 0:
            _, offset = _read_varint(data, offset)
            continue
        if wire_type == 1:
            offset += 8
            continue
        if wire_type == 5:
            offset += 4
            continue
        if wire_type != 2:
            raise ValueError("Unsupported CRX protobuf wire type")
        length, offset = _read_varint(data, offset)
        end = offset + length
        if end > len(data):
            raise ValueError("Truncated CRX protobuf field")
        value = data[offset:end]
        if field == wanted:
            return value
        offset = end
    raise ValueError(f"CRX protobuf field {wanted} is missing")


def _extension_id(raw_id: bytes) -> str:
    if len(raw_id) != 16:
        raise ValueError("CRX extension id must contain 16 bytes")
    return "".join(chr(ord("a") + nibble) for byte in raw_id for nibble in divmod(byte, 16))


def read_crx_extension_id(data: bytes) -> str:
    """Return the extension id bound into a CRX2 or CRX3 file."""
    if len(data) < 12 or data[:4] != b"Cr24":
        raise ValueError("Invalid CRX header")
    version = int.from_bytes(data[4:8], "little")
    if version == 2:
        public_key_size = int.from_bytes(data[8:12], "little")
        signature_size = int.from_bytes(data[12:16], "little") if len(data) >= 16 else 0
        end = 16 + public_key_size + signature_size
        if public_key_size <= 0 or end > len(data):
            raise ValueError("Invalid CRX2 header")
        public_key = data[16 : 16 + public_key_size]
        return _extension_id(hashlib.sha256(public_key).digest()[:16])
    if version != 3:
        raise ValueError(f"Unsupported CRX version {version}")
    header_size = int.from_bytes(data[8:12], "little")
    if header_size <= 0 or 12 + header_size > len(data):
        raise ValueError("Invalid CRX3 header size")
    header = data[12 : 12 + header_size]
    signed_header = _protobuf_bytes_field(header, 10000)
    return _extension_id(_protobuf_bytes_field(signed_header, 1))


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
