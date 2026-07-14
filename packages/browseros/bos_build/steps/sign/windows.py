#!/usr/bin/env python3
"""Windows signing module for BrowserOS"""

import os
import shutil
import subprocess
from pathlib import Path
from typing import List, Optional
from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.env import EnvConfig
from ...products.server_binaries import (
    all_server_bundles,
    expected_windows_bundle_binary_paths,
    server_bundles_for_product,
)
from ...lib.utils import (
    log_info,
    log_error,
    log_success,
    log_warning,
    join_paths,
    IS_WINDOWS,
    get_command_secret_values,
    redact_command,
    redact_sensitive_text,
)


@step(
    "sign_windows",
    phase="sign",
    platforms=("windows",),
    env=(
        "CODE_SIGN_TOOL_PATH",
        "ESIGNER_USERNAME",
        "ESIGNER_PASSWORD",
        "ESIGNER_TOTP_SECRET",
    ),
)
class WindowsSignModule(Step):
    produces = ["signed_installer"]
    requires = ["built_app"]
    description = "Sign Windows binaries and create signed installer"

    def validate(self, ctx: Context) -> None:
        if not IS_WINDOWS():
            raise ValidationError("Windows signing requires Windows")

        build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
        if not build_output_dir.exists():
            raise ValidationError(
                f"Build output directory not found: {build_output_dir}"
            )

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
            raise ValidationError(
                f"Missing environment variables: {', '.join(missing)}"
            )

        _warn_about_suspicious_esigner_values(env)

    def execute(self, ctx: Context) -> None:
        log_info("\n🔏 Signing Windows binaries...")

        build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)

        self._sign_executables(build_output_dir, ctx)
        self._build_mini_installer(ctx)
        mini_installer_path = self._sign_installer(build_output_dir, ctx.env)

        ctx.artifact_registry.add("signed_installer", mini_installer_path)
        log_success("✅ All binaries signed successfully!")

    def _sign_executables(self, build_output_dir: Path, ctx: Context) -> None:
        log_info("\nStep 1/3: Signing executables before packaging...")
        env = ctx.env
        chrome_path = build_output_dir / "chrome.exe"
        if not chrome_path.exists():
            raise RuntimeError(f"Missing primary browser executable: {chrome_path}")

        missing = get_missing_required_browseros_server_binary_paths(
            build_output_dir, ctx.product.id
        )
        if missing:
            raise RuntimeError(
                "Missing bundled server binaries: "
                + ", ".join(str(path) for path in missing)
            )

        binaries_to_sign = [chrome_path]
        binaries_to_sign.extend(
            get_existing_browseros_server_binary_paths(build_output_dir, ctx.product.id)
        )
        for binary in binaries_to_sign:
            log_info(f"Found binary to sign: {binary.name}")

        if not sign_with_codesigntool(binaries_to_sign, env):
            raise RuntimeError("Failed to sign executables")

    def _build_mini_installer(self, ctx: Context) -> None:
        log_info("\nStep 2/3: Building mini_installer with signed binaries...")
        if not build_mini_installer(ctx):
            raise RuntimeError("Failed to build mini_installer")

    def _sign_installer(self, build_output_dir: Path, env: EnvConfig) -> Path:
        log_info("\nStep 3/3: Signing mini_installer.exe...")
        mini_installer_path = build_output_dir / "mini_installer.exe"
        if not mini_installer_path.exists():
            raise RuntimeError(
                f"mini_installer.exe not found at: {mini_installer_path}"
            )

        if not sign_with_codesigntool([mini_installer_path], env):
            raise RuntimeError("Failed to sign mini_installer.exe")

        return mini_installer_path


def get_browseros_server_binary_paths(
    build_output_dir: Path,
    product_id: str | None = None,
) -> List[Path]:
    """Return absolute paths to bundled server binaries for signing."""
    return expected_windows_bundle_binary_paths(build_output_dir, product_id)


def get_existing_browseros_server_binary_paths(
    build_output_dir: Path,
    product_id: str | None = None,
) -> List[Path]:
    """Return bundled server binary paths that exist in a build output dir."""
    return [
        path
        for path in expected_windows_bundle_binary_paths(build_output_dir, product_id)
        if path.exists()
    ]


def get_missing_required_browseros_server_binary_paths(
    build_output_dir: Path,
    product_id: str | None = None,
) -> List[Path]:
    """Return missing bundled server binaries that should already be packaged."""
    missing: List[Path] = []
    bundles = (
        server_bundles_for_product(product_id) if product_id else all_server_bundles()
    )
    for bundle in bundles:
        bundle_root = build_output_dir / bundle.windows_bundle_resources_root
        should_exist = (
            product_id is not None
            or bundle.required_in_chromium_output
            or bundle_root.exists()
        )
        if not should_exist:
            continue
        for rel in bundle.windows_binaries:
            path = bundle_root / "bin" / rel
            if not path.exists():
                missing.append(path)
    return missing


def build_mini_installer(ctx: Context) -> bool:
    """Build the mini_installer.exe"""
    from ..compile import build_target

    log_info("Building mini_installer target...")
    return build_target(ctx, "mini_installer")


def _legacy_codesigntool_path(env: EnvConfig) -> Optional[Path]:
    """Return the configured legacy CodeSignTool launcher path."""
    if env.code_sign_tool_exe:
        return Path(env.code_sign_tool_exe)
    if env.code_sign_tool_path:
        return Path(env.code_sign_tool_path) / "CodeSignTool.bat"
    return None


def _resolve_codesigntool_java_invocation(
    env: EnvConfig,
) -> Optional[tuple[list[str], Path]]:
    """Resolve SSL.com CodeSignTool to its bundled Java and jar invocation."""
    if env.code_sign_tool_exe:
        tool_root = Path(env.code_sign_tool_exe).parent
    elif env.code_sign_tool_path:
        tool_root = Path(env.code_sign_tool_path)
    else:
        return None

    jars = sorted(
        (tool_root / "jar").glob("code_sign_tool*.jar"),
        key=lambda path: path.name,
    )
    if not jars:
        return None
    jar = jars[-1]

    java_candidates = []
    for name in ("java.exe", "java"):
        java_candidates.extend(sorted(tool_root.glob(f"jdk-*/bin/{name}")))

    java_home = os.environ.get("JAVA_HOME")
    if java_home:
        java_home_path = Path(java_home)
        java_candidates.extend(
            [
                java_home_path / "bin" / "java.exe",
                java_home_path / "bin" / "java",
            ]
        )

    java_from_path = shutil.which("java")
    if java_from_path:
        java_candidates.append(Path(java_from_path))

    java = next(
        (candidate for candidate in java_candidates if candidate.exists()), None
    )
    if java is None:
        return None

    return ([str(java), "-jar", str(jar)], tool_root)


def _warn_about_suspicious_esigner_values(env: EnvConfig) -> None:
    """Warn when eSigner env values look copied with shell-only decoration."""
    for name, value in (
        ("ESIGNER_USERNAME", env.esigner_username),
        ("ESIGNER_PASSWORD", env.esigner_password),
        ("ESIGNER_TOTP_SECRET", env.esigner_totp_secret),
    ):
        if not value:
            continue
        if value != value.strip():
            log_warning(f"{name} has leading or trailing whitespace")
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {"'", '"'}:
            log_warning(f"{name} appears to include wrapping quote characters")


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

    direct_invocation = _resolve_codesigntool_java_invocation(env)
    codesigntool_path = _legacy_codesigntool_path(env)

    if direct_invocation is None and codesigntool_path is None:
        log_error("CODE_SIGN_TOOL_EXE or CODE_SIGN_TOOL_PATH not set in .env file")
        log_error("Set CODE_SIGN_TOOL_EXE=/path/to/CodeSignTool.sh (macOS/Linux)")
        log_error("Or CODE_SIGN_TOOL_PATH=C:/src/CodeSignTool-v1.3.2-windows (Windows)")
        return False

    if direct_invocation is None:
        assert codesigntool_path is not None
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

    if direct_invocation is None:
        log_warning(
            "Could not resolve CodeSignTool Java and jar; falling back to "
            "CodeSignTool.bat via cmd.exe. Passwords containing cmd.exe "
            "metacharacters may be mangled."
        )

    all_success = True
    for binary in binaries:
        secret_values: tuple[str, ...] = ()
        try:
            log_info(f"Signing {binary.name}...")

            temp_output_dir = binary.parent / "signed_temp"
            temp_output_dir.mkdir(exist_ok=True)

            if direct_invocation is not None:
                cmd_prefix, tool_root = direct_invocation
                cmd = [
                    *cmd_prefix,
                    "sign",
                    "-username",
                    env.esigner_username,
                    "-password",
                    env.esigner_password,
                ]
            else:
                assert codesigntool_path is not None
                tool_root = codesigntool_path.parent
                # Direct Java is the invariant: credentials must never pass
                # through cmd.exe parsing. This fallback preserves the
                # previous working shape only for unexpected layouts; .bat
                # dispatch reparses %*.
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

            secret_values = get_command_secret_values(cmd)
            log_info(f"Running: {redact_command(cmd)}")

            if direct_invocation is not None:
                result = subprocess.run(
                    cmd,
                    shell=False,
                    capture_output=True,
                    text=True,
                    cwd=str(tool_root),
                    env={**os.environ, "CODE_SIGN_TOOL_PATH": str(tool_root)},
                )
            else:
                cmd_str = " ".join(cmd)
                result = subprocess.run(
                    cmd_str,
                    shell=True,
                    capture_output=True,
                    text=True,
                    cwd=str(tool_root),
                )

            if result.stdout:
                for line in result.stdout.split("\n"):
                    if line.strip():
                        log_info(redact_sensitive_text(line.strip(), secret_values))
            if result.stderr:
                for line in result.stderr.split("\n"):
                    if line.strip() and "WARNING" not in line:
                        log_error(redact_sensitive_text(line.strip(), secret_values))

            if getattr(result, "returncode", 0) != 0 or (
                result.stdout and "Error:" in result.stdout
            ):
                log_error(
                    f"✗ Failed to sign {binary.name} - Authentication or signing error"
                )
                all_success = False
                continue

            signed_file = temp_output_dir / binary.name
            if signed_file.exists():
                shutil.move(str(signed_file), str(binary))
                log_info(f"Moved signed {binary.name} to original location")

            try:
                temp_output_dir.rmdir()
            except Exception:
                pass

            escaped_binary_path = str(binary).replace("'", "''")
            verify_cmd = [
                "powershell",
                "-Command",
                f"(Get-AuthenticodeSignature -LiteralPath '{escaped_binary_path}').Status",
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
            safe_error = redact_sensitive_text(str(e), secret_values)
            log_error(f"Failed to sign {binary.name}: {safe_error}")
            all_success = False

    return all_success


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

    _warn_about_suspicious_esigner_values(env)

    return True
