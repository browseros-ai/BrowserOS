#!/usr/bin/env python3
"""Platform-specific binary signing for OTA binaries"""

import subprocess
import tempfile
from pathlib import Path
from typing import Optional

from ...common.env import EnvConfig
from ...common.utils import (
    log_info,
    log_error,
    log_success,
    log_warning,
    IS_MACOS,
    IS_WINDOWS,
)


def sign_macos_binary(
    binary_path: Path,
    env: Optional[EnvConfig] = None,
    entitlements_path: Optional[Path] = None,
) -> bool:
    """Sign a macOS binary with codesign

    Args:
        binary_path: Path to binary to sign
        env: Environment config with certificate name
        entitlements_path: Optional path to entitlements plist

    Returns:
        True on success, False on failure
    """
    if not IS_MACOS():
        log_error("macOS signing requires macOS")
        return False

    if env is None:
        env = EnvConfig()

    certificate_name = env.macos_certificate_name
    if not certificate_name:
        log_error("MACOS_CERTIFICATE_NAME not set")
        return False

    log_info(f"Signing {binary_path.name}...")

    cmd = [
        "codesign",
        "--sign", certificate_name,
        "--force",
        "--timestamp",
        "--identifier", f"com.browseros.{binary_path.stem}",
        "--options", "runtime",
    ]

    if entitlements_path and entitlements_path.exists():
        cmd.extend(["--entitlements", str(entitlements_path)])

    cmd.append(str(binary_path))

    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if result.returncode != 0:
            log_error(f"codesign failed: {result.stderr}")
            return False

        log_success(f"Signed {binary_path.name}")
        return True

    except Exception as e:
        log_error(f"Signing failed: {e}")
        return False


def verify_macos_signature(binary_path: Path) -> bool:
    """Verify macOS binary signature"""
    if not IS_MACOS():
        return False

    try:
        result = subprocess.run(
            ["codesign", "--verify", "--verbose=2", str(binary_path)],
            capture_output=True,
            text=True,
            check=False,
        )
        return result.returncode == 0
    except Exception:
        return False


def notarize_macos_binary(
    binary_path: Path,
    env: Optional[EnvConfig] = None,
) -> bool:
    """Notarize a macOS binary with Apple

    The binary must be zipped for notarization submission.

    Args:
        binary_path: Path to binary to notarize (will be zipped internally)
        env: Environment config with notarization credentials

    Returns:
        True on success, False on failure
    """
    if not IS_MACOS():
        log_error("macOS notarization requires macOS")
        return False

    if env is None:
        env = EnvConfig()

    apple_id = env.macos_notarization_apple_id
    team_id = env.macos_notarization_team_id
    password = env.macos_notarization_password

    if not all([apple_id, team_id, password]):
        log_error("Missing notarization credentials:")
        if not apple_id:
            log_error("  PROD_MACOS_NOTARIZATION_APPLE_ID not set")
        if not team_id:
            log_error("  PROD_MACOS_NOTARIZATION_TEAM_ID not set")
        if not password:
            log_error("  PROD_MACOS_NOTARIZATION_PWD not set")
        return False

    log_info(f"Notarizing {binary_path.name}...")

    notarize_zip = None
    try:
        fd, tmp_path = tempfile.mkstemp(suffix=".zip")
        import os
        os.close(fd)
        notarize_zip = Path(tmp_path)

        result = subprocess.run(
            ["ditto", "-c", "-k", "--keepParent", str(binary_path), str(notarize_zip)],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            log_error(f"Failed to create zip: {result.stderr}")
            return False

        assert apple_id is not None
        assert team_id is not None
        assert password is not None
        subprocess.run(
            [
                "xcrun", "notarytool", "store-credentials", "notarytool-profile",
                "--apple-id", apple_id,
                "--team-id", team_id,
                "--password", password,
            ],
            capture_output=True,
            text=True,
            check=False,
        )

        log_info("Submitting for notarization (this may take a while)...")
        result = subprocess.run(
            [
                "xcrun", "notarytool", "submit", str(notarize_zip),
                "--keychain-profile", "notarytool-profile",
                "--wait",
            ],
            capture_output=True,
            text=True,
            check=False,
        )

        if result.returncode != 0:
            log_error(f"Notarization failed: {result.stderr}")
            log_error(result.stdout)
            return False

        if "status: Accepted" not in result.stdout:
            log_error("Notarization was not accepted")
            log_error(result.stdout)
            return False

        log_success(f"Notarized {binary_path.name}")
        return True

    except Exception as e:
        log_error(f"Notarization failed: {e}")
        return False
    finally:
        if notarize_zip and notarize_zip.exists():
            notarize_zip.unlink()


def sign_windows_binary(
    binary_path: Path,
    env: Optional[EnvConfig] = None,
) -> bool:
    """Sign a Windows binary with SSL.com CodeSignTool

    Args:
        binary_path: Path to binary to sign
        env: Environment config with eSigner credentials

    Returns:
        True on success, False on failure
    """
    if env is None:
        env = EnvConfig()

    # Prefer CODE_SIGN_TOOL_EXE (direct path), fall back to CODE_SIGN_TOOL_PATH + .bat
    if env.code_sign_tool_exe:
        codesigntool_path = Path(env.code_sign_tool_exe)
    elif env.code_sign_tool_path:
        codesigntool_path = Path(env.code_sign_tool_path) / "CodeSignTool.bat"
    else:
        log_warning("CODE_SIGN_TOOL_EXE not set - skipping Windows signing")
        return True

    if not codesigntool_path.exists():
        log_error(f"CodeSignTool not found at: {codesigntool_path}")
        return False

    if not all([env.esigner_username, env.esigner_password, env.esigner_totp_secret]):
        log_error("Missing eSigner credentials")
        return False

    log_info(f"Signing {binary_path.name}...")

    try:
        temp_output_dir = binary_path.parent / "signed_temp"
        temp_output_dir.mkdir(exist_ok=True)

        cmd = [
            str(codesigntool_path),
            "sign",
            "-username", env.esigner_username,
            "-password", f'"{env.esigner_password}"',
        ]

        if env.esigner_credential_id:
            cmd.extend(["-credential_id", env.esigner_credential_id])

        cmd.extend([
            "-totp_secret", env.esigner_totp_secret,
            "-input_file_path", str(binary_path),
            "-output_dir_path", str(temp_output_dir),
            "-override",
        ])

        result = subprocess.run(
            " ".join(cmd),
            shell=True,
            capture_output=True,
            text=True,
            cwd=str(codesigntool_path.parent),
        )

        if result.stdout and "Error:" in result.stdout:
            log_error(f"Signing failed: {result.stdout}")
            return False

        signed_file = temp_output_dir / binary_path.name
        if signed_file.exists():
            import shutil
            shutil.move(str(signed_file), str(binary_path))

        try:
            temp_output_dir.rmdir()
        except Exception:
            pass

        # Verify signature on Windows only (PowerShell not available on macOS/Linux)
        if IS_WINDOWS():
            verify_cmd = [
                "powershell", "-Command",
                f"(Get-AuthenticodeSignature '{binary_path}').Status",
            ]
            verify_result = subprocess.run(verify_cmd, capture_output=True, text=True)
            if "Valid" in verify_result.stdout:
                log_success(f"Signed and verified {binary_path.name}")
            else:
                log_error(f"Signature verification failed: {verify_result.stdout.strip()}")
                return False
        else:
            log_success(f"Signed {binary_path.name} (verification skipped on non-Windows)")

        return True

    except Exception as e:
        log_error(f"Signing failed: {e}")
        return False


def get_entitlements_path(root_dir: Path) -> Optional[Path]:
    """Get path to server binary entitlements file"""
    candidates = [
        root_dir / "resources" / "entitlements" / "browseros-executable-entitlements.plist",
        root_dir / "packages" / "browseros" / "resources" / "entitlements" / "browseros-executable-entitlements.plist",
    ]

    for candidate in candidates:
        if candidate.exists():
            return candidate

    return None
