#!/usr/bin/env python3
"""Platform-specific binary signing for OTA binaries"""

import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import List, Optional

from ...lib.env import EnvConfig
from ...products.server_binaries import (
    ServerBundle,
    expected_windows_binary_paths,
    macos_sign_spec_for,
)
from ...lib.utils import (
    log_info,
    log_error,
    log_success,
    log_warning,
    IS_MACOS,
    IS_WINDOWS,
    get_command_secret_values,
    redact_sensitive_text,
)


def sign_macos_binary(
    binary_path: Path,
    env: Optional[EnvConfig] = None,
    entitlements_path: Optional[Path] = None,
    *,
    identifier: Optional[str] = None,
    options: str = "runtime",
) -> bool:
    """Sign a macOS binary with codesign.

    ``identifier`` defaults to ``com.browseros.<stem>`` to preserve the
    previous single-binary signature shape. Callers that have a shared sign
    table (see ``common/server_binaries.py``) should pass identifier and
    options derived from that table so OTA-signed and Chromium-build-signed
    binaries share the same code identifier.
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

    resolved_identifier = identifier or f"com.browseros.{binary_path.stem}"
    cmd = [
        "codesign",
        "--sign", certificate_name,
        "--force",
        "--timestamp",
        "--identifier", resolved_identifier,
        "--options", options,
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


def _resolve_notarization_credentials(
    env: Optional[EnvConfig],
) -> Optional[EnvConfig]:
    if env is None:
        env = EnvConfig()

    missing: List[str] = []
    if not env.macos_notarization_apple_id:
        missing.append("PROD_MACOS_NOTARIZATION_APPLE_ID")
    if not env.macos_notarization_team_id:
        missing.append("PROD_MACOS_NOTARIZATION_TEAM_ID")
    if not env.macos_notarization_password:
        missing.append("PROD_MACOS_NOTARIZATION_PWD")
    if missing:
        log_error("Missing notarization credentials:")
        for name in missing:
            log_error(f"  {name} not set")
        return None
    return env


def _submit_notarization(submission_path: Path, env: EnvConfig) -> bool:
    assert env.macos_notarization_apple_id is not None
    assert env.macos_notarization_team_id is not None
    assert env.macos_notarization_password is not None

    subprocess.run(
        [
            "xcrun", "notarytool", "store-credentials", "notarytool-profile",
            "--apple-id", env.macos_notarization_apple_id,
            "--team-id", env.macos_notarization_team_id,
            "--password", env.macos_notarization_password,
        ],
        capture_output=True,
        text=True,
        check=False,
    )

    log_info("Submitting for notarization (this may take a while)...")
    result = subprocess.run(
        [
            "xcrun", "notarytool", "submit", str(submission_path),
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
    return True


def notarize_macos_binary(
    binary_path: Path,
    env: Optional[EnvConfig] = None,
) -> bool:
    """Notarize a single macOS binary with Apple.

    The binary is first wrapped in a zip via ``ditto --keepParent`` because
    ``notarytool`` does not accept bare executables. For an already-zipped
    Sparkle bundle, call :func:`notarize_macos_zip` instead — double-wrapping
    nests zips and notarytool does not descend into nested archives.
    """
    if not IS_MACOS():
        log_error("macOS notarization requires macOS")
        return False

    env = _resolve_notarization_credentials(env)
    if env is None:
        return False

    log_info(f"Notarizing {binary_path.name}...")
    notarize_zip: Optional[Path] = None
    try:
        fd, tmp_path = tempfile.mkstemp(suffix=".zip")
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

        if not _submit_notarization(notarize_zip, env):
            return False

        log_success(f"Notarized {binary_path.name}")
        return True

    except Exception as e:
        log_error(f"Notarization failed: {e}")
        return False
    finally:
        if notarize_zip and notarize_zip.exists():
            notarize_zip.unlink()


def notarize_macos_zip(zip_path: Path, env: Optional[EnvConfig] = None) -> bool:
    """Notarize a pre-built Sparkle bundle zip by submitting it directly.

    ``notarytool`` accepts ``.zip`` submissions and recursively scans the
    Mach-O binaries inside. No extra wrapping — passing this zip through
    ``ditto --keepParent`` would nest zips and Apple's service would not
    descend into the inner archive.
    """
    if not IS_MACOS():
        log_error("macOS notarization requires macOS")
        return False

    env = _resolve_notarization_credentials(env)
    if env is None:
        return False

    log_info(f"Notarizing {zip_path.name}...")
    try:
        if not _submit_notarization(zip_path, env):
            return False
        log_success(f"Notarized {zip_path.name}")
        return True
    except Exception as e:
        log_error(f"Notarization failed: {e}")
        return False


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

    secret_values: tuple[str, ...] = ()
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

        secret_values = get_command_secret_values(cmd)
        result = subprocess.run(
            " ".join(cmd),
            shell=True,
            capture_output=True,
            text=True,
            cwd=str(codesigntool_path.parent),
        )

        if result.stdout and "Error:" in result.stdout:
            safe_output = redact_sensitive_text(result.stdout, secret_values)
            log_error(f"Signing failed: {safe_output}")
            return False

        signed_file = temp_output_dir / binary_path.name
        if signed_file.exists():
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
        safe_error = redact_sensitive_text(str(e), secret_values)
        log_error(f"Signing failed: {safe_error}")
        return False


def sign_server_bundle_macos(
    resources_dir: Path,
    env: EnvConfig,
    entitlements_root: Path,
) -> bool:
    """Codesign every known binary under ``resources_dir/bin/**``.

    Unknown executables are a hard error: every regular file under
    ``resources/bin/`` must have an entry in ``MACOS_SERVER_BINARIES``.
    This prevents silently shipping an unsigned binary when a new
    third-party dep is added to the agent build without being registered
    in the shared sign table. The unknown-file check runs before any
    codesign call so a bad release fails in seconds rather than after
    several minutes of signing.
    """
    bin_dir = resources_dir / "bin"
    if not bin_dir.is_dir():
        log_error(f"bin dir not found: {bin_dir}")
        return False

    # Only Mach-O-style executables need signing; any future data/config file
    # shipped under resources/bin/ (plists, shell completion, etc.) is not a
    # codesign target and must not trigger the unknown-binary guard.
    executables = [
        p
        for p in sorted(bin_dir.rglob("*"))
        if p.is_file() and not p.is_symlink() and os.access(p, os.X_OK)
    ]
    unknowns = [p for p in executables if macos_sign_spec_for(p) is None]
    if unknowns:
        log_error(
            "Unknown executables found under resources/bin/ not registered in "
            "MACOS_SERVER_BINARIES (see build/common/server_binaries.py):"
        )
        for path in unknowns:
            log_error(f"  - {path.relative_to(resources_dir)}")
        return False

    for path in executables:
        spec = macos_sign_spec_for(path)
        assert spec is not None  # unknowns filtered above

        entitlements_path: Optional[Path] = None
        if spec.entitlements:
            entitlements_path = entitlements_root / spec.entitlements
            if not entitlements_path.exists():
                log_error(
                    f"Missing entitlements for {path.name}: {entitlements_path}"
                )
                return False

        if not sign_macos_binary(
            path,
            env,
            entitlements_path,
            identifier=f"com.browseros.{spec.identifier_suffix}",
            options=spec.options,
        ):
            return False

    return True


def sign_server_bundle_windows(
    resources_dir: Path, env: EnvConfig, bundle: ServerBundle
) -> bool:
    """Sign each Windows binary declared by a server bundle.

    A missing expected binary is a hard error: publishing an incomplete
    Windows bundle would ship a broken OTA update without a pipeline signal.
    Symmetric with the macOS bundle's unknown-file guard.
    """
    bin_dir = resources_dir / "bin"
    paths = expected_windows_binary_paths(bin_dir, bundle)
    for path in paths:
        if not path.exists():
            log_error(f"Windows binary missing (cannot sign): {path}")
            return False

    for path in paths:
        if not sign_windows_binary(path, env):
            return False
    return True
