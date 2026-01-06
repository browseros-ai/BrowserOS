#!/usr/bin/env python3
"""OTA CLI - Over-The-Air update automation for BrowserOS"""

from pathlib import Path
from typing import Optional

import typer

from ..common.context import Context
from ..common.module import ValidationError
from ..common.utils import log_info, log_error, log_success

from ..modules.ota import ServerOTAModule
from ..modules.ota.common import (
    upload_to_r2,
    get_appcast_path,
)

app = typer.Typer(
    help="OTA (Over-The-Air) update automation",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


def create_ota_context(root_dir: Optional[Path] = None) -> Context:
    """Create Context for OTA operations"""
    if root_dir is None:
        root_dir = Path.cwd()

    return Context(
        root_dir=root_dir,
        chromium_src=root_dir,
        architecture="",
        build_type="release",
    )


def execute_module(ctx: Context, module) -> None:
    """Execute a single module with validation"""
    try:
        module.validate(ctx)
        module.execute(ctx)
    except ValidationError as e:
        log_error(f"Validation failed: {e}")
        raise typer.Exit(1)
    except Exception as e:
        log_error(f"Module failed: {e}")
        raise typer.Exit(1)


@app.command("server")
def server_ota(
    version: str = typer.Option(
        ..., "--version", "-v", help="Version to release (e.g., 0.0.36)"
    ),
    channel: str = typer.Option(
        "alpha", "--channel", "-c", help="Release channel: alpha or prod"
    ),
    binaries: Optional[Path] = typer.Option(
        None, "--binaries", "-b", help="Directory containing server binaries"
    ),
    platform: Optional[str] = typer.Option(
        None, "--platform", "-p",
        help="Only process specific platform (darwin_arm64, darwin_x64, linux_arm64, linux_x64, windows_x64)"
    ),
    skip_sign: bool = typer.Option(
        False, "--skip-sign", help="Skip binary signing (for pre-signed binaries)"
    ),
    skip_upload: bool = typer.Option(
        False, "--skip-upload", help="Skip R2 upload (for local testing)"
    ),
    root_dir: Optional[Path] = typer.Option(
        None, "--root", "-r", help="Project root directory"
    ),
):
    """Release BrowserOS Server OTA update

    \b
    Full Release (all platforms):
      browseros ota server --version 0.0.36 --channel alpha

    \b
    Single Platform (for cross-platform signing):
      browseros ota server --version 0.0.36 --platform darwin_arm64

    \b
    Skip Signing (for pre-signed binaries):
      browseros ota server --version 0.0.36 --skip-sign

    \b
    Local Testing (no upload):
      browseros ota server --version 0.0.36 --skip-upload

    \b
    Available Platforms:
      darwin_arm64  - macOS Apple Silicon
      darwin_x64    - macOS Intel
      linux_arm64   - Linux ARM64
      linux_x64     - Linux x86_64
      windows_x64   - Windows x86_64
    """
    log_info(f"🚀 BrowserOS Server OTA v{version}")
    log_info("=" * 70)

    ctx = create_ota_context(root_dir)

    module = ServerOTAModule(
        version=version,
        channel=channel,
        binaries_dir=binaries,
        skip_sign=skip_sign,
        skip_upload=skip_upload,
        platform_filter=platform,
    )

    execute_module(ctx, module)


@app.command("publish-appcast")
def publish_appcast(
    channel: str = typer.Option(
        "alpha", "--channel", "-c", help="Release channel: alpha or prod"
    ),
    appcast_file: Optional[Path] = typer.Option(
        None, "--file", "-f", help="Custom appcast file to upload"
    ),
):
    """Publish appcast XML to CDN

    \b
    Upload alpha appcast:
      browseros ota publish-appcast --channel alpha

    \b
    Upload production appcast:
      browseros ota publish-appcast --channel prod

    \b
    Upload custom file:
      browseros ota publish-appcast --file /path/to/appcast.xml
    """
    if appcast_file:
        if not appcast_file.exists():
            log_error(f"Appcast file not found: {appcast_file}")
            raise typer.Exit(1)
        source_path = appcast_file
    else:
        source_path = get_appcast_path(channel)
        if not source_path.exists():
            log_error(f"Appcast file not found: {source_path}")
            log_error("Run 'browseros ota server' first to generate the appcast")
            raise typer.Exit(1)

    if channel == "alpha":
        r2_path = "browseros/appcast-server.alpha.xml"
    else:
        r2_path = "browseros/appcast-server.xml"

    log_info(f"📤 Uploading {source_path.name} to {r2_path}...")

    if upload_to_r2(source_path, r2_path):
        cdn_url = f"https://cdn.browseros.com/{r2_path.replace('browseros/', '')}"
        log_success(f"✅ Published: {cdn_url}")
    else:
        log_error("Upload failed")
        raise typer.Exit(1)


@app.command("list-platforms")
def list_platforms():
    """List available server platforms"""
    from ..modules.ota.common import SERVER_PLATFORMS

    log_info("\n📦 Available Server Platforms:")
    log_info("-" * 50)
    for p in SERVER_PLATFORMS:
        log_info(f"  {p['name']:<15} {p['os']:<10} {p['arch']}")
    log_info("-" * 50)


@app.callback(invoke_without_command=True)
def main(ctx: typer.Context):
    """OTA update automation for BrowserOS

    \b
    Server OTA:
      browseros ota server --version 0.0.36 --channel alpha

    \b
    Publish Appcast:
      browseros ota publish-appcast --channel alpha

    \b
    List Platforms:
      browseros ota list-platforms
    """
    if ctx.invoked_subcommand is None:
        typer.echo("Use --help for usage information")
        typer.echo("Available commands: server, publish-appcast, list-platforms")
        raise typer.Exit(0)


if __name__ == "__main__":
    app()
