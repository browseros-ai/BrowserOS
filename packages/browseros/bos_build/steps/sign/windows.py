#!/usr/bin/env python3
"""Windows signing module for BrowserOS"""

from pathlib import Path
from typing import List
from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.env import EnvConfig
from ...products.server_binaries import (
    all_server_bundles,
    expected_windows_bundle_binary_paths,
    server_bundles_for_product,
)
from ...lib.utils import log_info, log_success, join_paths, IS_WINDOWS
from ...lib.windows_signing import sign_windows_files
from ...lib.windows_signing.config import (
    browser_providers,
    required_signing_env,
    validate_signing,
)

# Keep the old import surface for local callers while the adapter lives below
# both browser packaging and server OTA packaging.
from ...lib.windows_signing.sslcom import (
    sign_with_codesigntool as sign_with_codesigntool,
    check_signing_environment as check_signing_environment,
)


@step(
    "sign_windows",
    phase="sign",
    platforms=("windows",),
)
class WindowsSignModule(Step):
    produces = ["signed_installer"]
    requires = ["built_app"]
    description = "Sign Windows binaries and create signed installer"

    def validate(self, context: Context) -> None:
        ctx = context
        if not IS_WINDOWS():
            raise ValidationError("Windows signing requires Windows")

        build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
        if not build_output_dir.exists():
            raise ValidationError(
                f"Build output directory not found: {build_output_dir}"
            )

        for provider in browser_providers(ctx.env):
            validate_signing(provider, ctx.env)

    @classmethod
    def required_env(cls) -> tuple[str, ...]:
        env = EnvConfig()
        return required_signing_env(browser_providers(env), env)

    def preflight(self, context: Context) -> None:
        for provider in browser_providers(context.env):
            validate_signing(provider, context.env)

    def execute(self, context: Context) -> None:
        ctx = context
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

        # The installer embeds these bytes. Route the server override here as
        # well as in OTA, before mini_installer consumes the resource tree.
        browser_provider, server_provider = browser_providers(env)
        if not sign_windows_files([chrome_path], env, browser_provider):
            raise RuntimeError("Failed to sign browser executable")
        servers = get_existing_browseros_server_binary_paths(
            build_output_dir, ctx.product.id
        )
        if not sign_windows_files(servers, env, server_provider):
            raise RuntimeError("Failed to sign bundled servers")

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

        if not sign_windows_files(
            [mini_installer_path], env, env.windows_signing_provider
        ):
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
