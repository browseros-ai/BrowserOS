#!/usr/bin/env python3
"""Sparkle Ed25519 signing for auto-update

Cross-platform Ed25519 signing compatible with Sparkle framework.
Uses Python cryptography library - works on macOS, Windows, and Linux.
"""

import base64
from pathlib import Path
from typing import Optional, Tuple

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from .env import EnvConfig
from .utils import log_error


def _parse_sparkle_private_key(key_data: str) -> Optional[Ed25519PrivateKey]:
    """Parse Sparkle Ed25519 private key from various formats

    Sparkle key formats:
    - Raw 64-byte key (32-byte seed + 32-byte public key)
    - Raw 32-byte seed
    - Base64 encoded versions of above

    Returns:
        Ed25519PrivateKey or None on failure
    """
    try:
        # Try base64 decode first (env var might be base64 encoded)
        try:
            key_bytes = base64.b64decode(key_data)
        except Exception:
            # Not base64, try as raw bytes
            key_bytes = key_data.encode("latin-1")

        # Sparkle uses 64-byte format: 32-byte seed + 32-byte public key
        if len(key_bytes) == 64:
            seed = key_bytes[:32]
            return Ed25519PrivateKey.from_private_bytes(seed)
        elif len(key_bytes) == 32:
            return Ed25519PrivateKey.from_private_bytes(key_bytes)
        else:
            log_error(f"Invalid Sparkle key length: {len(key_bytes)} bytes (expected 32 or 64)")
            return None

    except Exception as e:
        log_error(f"Failed to parse Sparkle private key: {e}")
        return None


def sparkle_sign_file(
    file_path: Path,
    env: Optional[EnvConfig] = None,
) -> Tuple[Optional[str], int]:
    """Sign a file with Sparkle Ed25519 key

    Args:
        file_path: Path to file to sign (typically a zip or dmg)
        env: Environment config with Sparkle key

    Returns:
        (signature, length) tuple, or (None, 0) on failure
    """
    if env is None:
        env = EnvConfig()

    if not env.has_sparkle_key():
        log_error("SPARKLE_PRIVATE_KEY not set")
        return None, 0

    key_data = env.sparkle_private_key
    if not key_data:
        log_error("SPARKLE_PRIVATE_KEY is empty")
        return None, 0

    private_key = _parse_sparkle_private_key(key_data)
    if not private_key:
        return None, 0

    try:
        file_data = file_path.read_bytes()
        file_length = len(file_data)

        signature_bytes = private_key.sign(file_data)
        signature_b64 = base64.b64encode(signature_bytes).decode("ascii")

        return signature_b64, file_length

    except Exception as e:
        log_error(f"Error signing {file_path.name}: {e}")
        return None, 0
