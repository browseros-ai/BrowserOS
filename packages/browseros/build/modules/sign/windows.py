#!/usr/bin/env python3
"""Windows signing module for BrowserOS"""

import subprocess
from pathlib import Path
from typing import List, Optional
from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.env import EnvConfig
from ...common.utils import (
    log_info,
    log_error,
    log_success,
    log_warning,
    join_paths,
    IS_WINDOWS,
)

BROWSEROS_SERVER_BINARIES: List[str] = [
    "browseros_server.exe",
    "codex.exe",
]


class WindowsSignModule(CommandModule):
    produces = ["signed_installer"]
    requires = ["built_app"]
    description = "Sign Windows binaries and create signed installer"

    def validate(self, ctx: Context) -> None:
        if not IS_WINDOWS():
            raise ValidationError("Windows signing requires Windows")

        build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
        if not build_output_dir.exists():
            raise ValidationError(f"Build output directory not found: {build_output_dir}")

        env = ctx.env
        if not env.code_sign_tool_path:
            raise ValidationError("CODE_SIGN_TOOL_PATH environment variable not set")

        missing = []
        if not env.esigner_username:
            missing.append("ESIGNER_USERNAME")
        if not env.esigner_password:
            missing.append("ESIGNER_PASSWORD")
        if not env.esigner_totp_secret:
            missing.append("ESIGNER_TOTP_SECRET")

        if missing:
            raise ValidationError(f"Missing environment variables: {', '.join(missing)}")

    def execute(self, ctx: Context) -> None:
        log_info("\n🔏 Signing Windows binaries...")

        build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)

        self._sign_executables(build_output_dir, ctx.env)
        self._build_mini_installer(ctx)
        mini_installer_path = self._sign_installer(build_output_dir, ctx.env)

        ctx.artifact_registry.add("signed_installer", mini_installer_path)
        log_success("✅ All binaries signed successfully!")

    def _sign_executables(self, build_output_dir: Path, env: EnvConfig) -> None:
        log_info("\nStep 1/3: Signing executables before packaging...")
        binaries_to_sign_first = [build_output_dir / "chrome.exe"]
        binaries_to_sign_first.extend(get_browseros_server_binary_paths(build_output_dir))

        existing_binaries = []
        for binary in binaries_to_sign_first:
            if binary.exists():
                existing_binaries.append(binary)
                log_info(f"Found binary to sign: {binary.name}")
            else:
                log_warning(f"Binary not found: {binary}")

        if not existing_binaries:
            raise RuntimeError("No binaries found to sign")

        if not sign_with_codesigntool(existing_binaries, env):
            raise RuntimeError("Failed to sign executables")

    def _build_mini_installer(self, ctx: Context) -> None:
        log_info("\nStep 2/3: Building mini_installer with signed binaries...")
        if not build_mini_installer(ctx):
            raise RuntimeError("Failed to build mini_installer")

    def _sign_installer(self, build_output_dir: Path, env: EnvConfig) -> Path:
        log_info("\nStep 3/3: Signing mini_installer.exe...")
        mini_installer_path = build_output_dir / "mini_installer.exe"
        if not mini_installer_path.exists():
            raise RuntimeError(f"mini_installer.exe not found at: {mini_installer_path}")

        if not sign_with_codesigntool([mini_installer_path], env):
            raise RuntimeError("Failed to sign mini_installer.exe")

        return mini_installer_path


def get_browseros_server_binary_paths(build_output_dir: Path) -> List[Path]:
    """Return absolute paths to BrowserOS Server binaries for signing."""
    server_dir = build_output_dir / "BrowserOSServer" / "default" / "resources" / "bin"
    return [server_dir / binary for binary in BROWSEROS_SERVER_BINARIES]


def build_mini_installer(ctx: Context) -> bool:
    """Build the mini_installer.exe"""
    from ..compile import build_target
    log_info("Building mini_installer target...")
    return build_target(ctx, "mini_installer")


def sign_with_codesigntool(
    binaries: List[Path],
    env: Optional[EnvConfig] = None,
) -> bool:
    """Sign binaries using SSL.com CodeSignTool

    Args:
        binaries: List of binary paths to sign
        env: Optional EnvConfig instance. If not provided, creates a new one.
    """
    log_info("Using SSL.com CodeSignTool for signing...")

    if env is None:
        env = EnvConfig()

    # Prefer CODE_SIGN_TOOL_EXE (direct path to executable), fall back to CODE_SIGN_TOOL_PATH + .bat
    if env.code_sign_tool_exe:
        codesigntool_path = Path(env.code_sign_tool_exe)
    elif env.code_sign_tool_path:
        codesigntool_path = Path(env.code_sign_tool_path) / "CodeSignTool.bat"
    else:
        log_error("CODE_SIGN_TOOL_EXE or CODE_SIGN_TOOL_PATH not set in .env file")
        log_error("Set CODE_SIGN_TOOL_EXE=/path/to/CodeSignTool.sh (macOS/Linux)")
        log_error("Or CODE_SIGN_TOOL_PATH=C:/src/CodeSignTool-v1.3.2-windows (Windows)")
        return False

    if not codesigntool_path.exists():
        log_error(f"CodeSignTool not found at: {codesigntool_path}")
        return False

    if not all([env.esigner_username, env.esigner_password, env.esigner_totp_secret]):
        log_error("Missing required eSigner environment variables in .env:")
        log_error("  ESIGNER_USERNAME=your-email")
        log_error("  ESIGNER_PASSWORD=your-password")
        log_error("  ESIGNER_TOTP_SECRET=your-totp-secret")
        if not env.esigner_credential_id:
            log_warning("  ESIGNER_CREDENTIAL_ID is recommended but optional")
        return False

    all_success = True
    for binary in binaries:
        try:
            log_info(f"Signing {binary.name}...")

            temp_output_dir = binary.parent / "signed_temp"
            temp_output_dir.mkdir(exist_ok=True)

            cmd = [
                str(codesigntool_path),
                "sign",
                "-username",
                env.esigner_username,
                "-password",
                f'"{env.esigner_password}"',
            ]

            if env.esigner_credential_id:
                cmd.extend(["-credential_id", env.esigner_credential_id])

            cmd.extend(
                [
                    "-totp_secret",
                    env.esigner_totp_secret,
                    "-input_file_path",
                    str(binary),
                    "-output_dir_path",
                    str(temp_output_dir),
                    "-override",
                ]
            )

            cmd_str = " ".join(cmd)
            log_info(f"Running: {cmd_str}")

            result = subprocess.run(
                cmd_str,
                shell=True,
                capture_output=True,
                text=True,
                cwd=str(codesigntool_path.parent),
            )

            if result.stdout:
                for line in result.stdout.split("\n"):
                    if line.strip():
                        log_info(line.strip())
            if result.stderr:
                for line in result.stderr.split("\n"):
                    if line.strip() and "WARNING" not in line:
                        log_error(line.strip())

            if result.stdout and "Error:" in result.stdout:
                log_error(
                    f"✗ Failed to sign {binary.name} - Authentication or signing error"
                )
                all_success = False
                continue

            signed_file = temp_output_dir / binary.name
            if signed_file.exists():
                import shutil
                shutil.move(str(signed_file), str(binary))
                log_info(f"Moved signed {binary.name} to original location")

            try:
                temp_output_dir.rmdir()
            except Exception:
                pass

            verify_cmd = [
                "powershell",
                "-Command",
                f"(Get-AuthenticodeSignature '{binary}').Status",
            ]
            try:
                verify_result = subprocess.run(
                    verify_cmd, capture_output=True, text=True
                )
                if "Valid" in verify_result.stdout:
                    log_success(f"✓ {binary.name} signed and verified successfully")
                else:
                    log_error(
                        f"✗ {binary.name} signing verification failed - Status: {verify_result.stdout.strip()}"
                    )
                    all_success = False
            except Exception:
                log_warning(f"Could not verify signature for {binary.name}")

        except Exception as e:
            log_error(f"Failed to sign {binary.name}: {e}")
            all_success = False

    return all_success


def sign_universal(contexts: List[Context]) -> bool:
    """Windows doesn't support universal binaries"""
    log_warning("Universal signing is not supported on Windows")
    return True


def check_signing_environment(env: Optional[EnvConfig] = None) -> bool:
    """Check if Windows signing environment is properly configured

    Args:
        env: Optional EnvConfig instance. If not provided, creates a new one.
    """
    if env is None:
        env = EnvConfig()

    if not env.code_sign_tool_exe and not env.code_sign_tool_path:
        log_error("CODE_SIGN_TOOL_EXE or CODE_SIGN_TOOL_PATH not set")
        return False

    missing = []
    if not env.esigner_username:
        missing.append("ESIGNER_USERNAME")
    if not env.esigner_password:
        missing.append("ESIGNER_PASSWORD")
    if not env.esigner_totp_secret:
        missing.append("ESIGNER_TOTP_SECRET")

    if missing:
        log_error(f"Missing environment variables: {', '.join(missing)}")
        return False

    return True
