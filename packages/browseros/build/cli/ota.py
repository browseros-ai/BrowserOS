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
    SERVER_PLATFORMS,
)

app = typer.Typer(
    help="OTA (Over-The-Air) update automation",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)

server_app = typer.Typer(
    help="BrowserOS Server OTA commands",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)
app.add_typer(server_app, name="server")


def create_ota_context(chromium_src: Optional[Path] = None) -> Context:
    """Create Context for OTA operations

    Args:
        chromium_src: Path to Chromium source directory (for sign_update tool)
    """
    return Context(
        chromium_src=chromium_src or Path(),
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


@server_app.command("release")
def server_release(
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
    chromium_src: Optional[Path] = typer.Option(
        None, "--chromium-src", "-S", help="Path to Chromium source directory"
    ),
):
    """Release BrowserOS Server OTA update

    \b
    Full Release (all platforms):
      browseros ota server release --version 0.0.36 --channel alpha -S /path/to/chromium

    \b
    Single Platform (for cross-platform signing):
      browseros ota server release --version 0.0.36 --platform darwin_arm64

    \b
    Skip Signing (for pre-signed binaries):
      browseros ota server release --version 0.0.36 --skip-sign

    \b
    Local Testing (no upload):
      browseros ota server release --version 0.0.36 --skip-upload
    """
    log_info(f"🚀 BrowserOS Server OTA v{version}")
    log_info("=" * 70)

    ctx = create_ota_context(chromium_src)

    module = ServerOTAModule(
        version=version,
        channel=channel,
        binaries_dir=binaries,
        skip_sign=skip_sign,
        skip_upload=skip_upload,
        platform_filter=platform,
    )

    execute_module(ctx, module)


@server_app.command("publish-appcast")
def server_publish_appcast(
    channel: str = typer.Option(
        "alpha", "--channel", "-c", help="Release channel: alpha or prod"
    ),
    appcast_file: Optional[Path] = typer.Option(
        None, "--file", "-f", help="Custom appcast file to upload"
    ),
):
    """Publish server appcast XML to CDN

    \b
    Upload alpha appcast:
      browseros ota server publish-appcast --channel alpha

    \b
    Upload production appcast:
      browseros ota server publish-appcast --channel prod

    \b
    Upload custom file:
      browseros ota server publish-appcast --file /path/to/appcast.xml
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
            log_error("Run 'browseros ota server release' first to generate the appcast")
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


@server_app.command("list-platforms")
def server_list_platforms():
    """List available server platforms"""
    log_info("\n📦 Available Server Platforms:")
    log_info("-" * 50)
    for p in SERVER_PLATFORMS:
        log_info(f"  {p['name']:<15} {p['os']:<10} {p['arch']}")
    log_info("-" * 50)


@server_app.callback(invoke_without_command=True)
def server_main(ctx: typer.Context):
    """BrowserOS Server OTA commands

    \b
    Release:
      browseros ota server release --version 0.0.36

    \b
    Publish Appcast:
      browseros ota server publish-appcast --channel alpha

    \b
    List Platforms:
      browseros ota server list-platforms
    """
    if ctx.invoked_subcommand is None:
        typer.echo("Use --help for usage information")
        typer.echo("Available commands: release, publish-appcast, list-platforms")
        raise typer.Exit(0)


@app.callback(invoke_without_command=True)
def main(ctx: typer.Context):
    """OTA update automation for BrowserOS

    \b
    Server OTA:
      browseros ota server release --version 0.0.36
      browseros ota server publish-appcast --channel alpha
      browseros ota server list-platforms
    """
    if ctx.invoked_subcommand is None:
        typer.echo("Use --help for usage information")
        typer.echo("Available subcommands: server")
        raise typer.Exit(0)


if __name__ == "__main__":
    app()
