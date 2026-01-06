#!/usr/bin/env python3
"""OTA CLI - Over-The-Air update automation for BrowserOS"""

from pathlib import Path
from typing import Optional

import typer

from ..common.context import Context
from ..common.env import EnvConfig
from ..common.module import ValidationError
from ..common.sparkle import sparkle_sign_file
from ..common.utils import log_info, log_error, log_success

from ..modules.ota import ServerOTAModule
from ..modules.ota.common import (
    get_appcast_path,
    SERVER_PLATFORMS,
)
from ..modules.upload import get_r2_client, upload_file_to_r2

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


def create_ota_context() -> Context:
    """Create Context for OTA operations"""
    return Context(
        chromium_src=Path(),
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
        help="Platform(s) to process, comma-separated (darwin_arm64, darwin_x64, linux_arm64, linux_x64, windows_x64)"
    ),
):
    """Release BrowserOS Server OTA update

    \b
    Full Release (all platforms):
      browseros ota server release --version 0.0.36 --channel alpha

    \b
    Single Platform:
      browseros ota server release --version 0.0.36 --platform darwin_arm64

    \b
    Multiple Platforms:
      browseros ota server release --version 0.0.36 --platform darwin_arm64,darwin_x64
    """
    log_info(f"🚀 BrowserOS Server OTA v{version}")
    log_info("=" * 70)

    ctx = create_ota_context()

    module = ServerOTAModule(
        version=version,
        channel=channel,
        binaries_dir=binaries,
        platform_filter=platform,
    )

    execute_module(ctx, module)


@server_app.command("release-appcast")
def server_release_appcast(
    channel: str = typer.Option(
        "alpha", "--channel", "-c", help="Release channel: alpha or prod"
    ),
    appcast_file: Optional[Path] = typer.Option(
        None, "--file", "-f", help="Custom appcast file to upload"
    ),
):
    """Publish appcast XML to make the release live

    This is the final step after 'release' uploads artifacts.
    Publishing the appcast makes the update available to clients.

    \b
    Release alpha appcast:
      browseros ota server release-appcast --channel alpha

    \b
    Release production appcast:
      browseros ota server release-appcast --channel prod

    \b
    Release custom appcast file:
      browseros ota server release-appcast --file /path/to/appcast.xml
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
        r2_key = "appcast-server.alpha.xml"
    else:
        r2_key = "appcast-server.xml"

    log_info(f"📤 Uploading {source_path.name} to {r2_key}...")

    env = EnvConfig()
    if not env.has_r2_config():
        log_error("R2 configuration not set. Required env vars: R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY")
        raise typer.Exit(1)

    r2_client = get_r2_client(env)
    if not r2_client:
        log_error("Failed to create R2 client")
        raise typer.Exit(1)

    if upload_file_to_r2(r2_client, source_path, r2_key, env.r2_bucket):
        cdn_url = f"https://cdn.browseros.com/{r2_key}"
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


@app.command("test-signing")
def test_signing(
    file_path: Path = typer.Argument(..., help="File to sign for testing"),
):
    """Test Sparkle Ed25519 signing on a file

    \b
    Example:
      browseros ota test-signing /path/to/file.zip
    """
    if not file_path.exists():
        log_error(f"File not found: {file_path}")
        raise typer.Exit(1)

    env = EnvConfig()
    if not env.has_sparkle_key():
        log_error("SPARKLE_PRIVATE_KEY not set")
        raise typer.Exit(1)

    log_info(f"\n🔐 Testing Sparkle Ed25519 signing")
    log_info(f"File: {file_path}")
    log_info("-" * 60)

    sig, length = sparkle_sign_file(file_path, env)
    if not sig:
        log_error("Signing failed")
        raise typer.Exit(1)

    log_success(f"✅ Signed successfully")
    log_info(f"   Signature: {sig[:50]}...")
    log_info(f"   Length: {length}")


@server_app.callback(invoke_without_command=True)
def server_main(ctx: typer.Context):
    """BrowserOS Server OTA commands

    \b
    Release (upload artifacts):
      browseros ota server release --version 0.0.36

    \b
    Release Appcast (make live):
      browseros ota server release-appcast --channel alpha

    \b
    List Platforms:
      browseros ota server list-platforms
    """
    if ctx.invoked_subcommand is None:
        typer.echo("Use --help for usage information")
        typer.echo("Available commands: release, release-appcast, list-platforms")
        raise typer.Exit(0)


@app.callback(invoke_without_command=True)
def main(ctx: typer.Context):
    """OTA update automation for BrowserOS

    \b
    Server OTA:
      browseros ota server release --version 0.0.36
      browseros ota server release-appcast --channel alpha
      browseros ota server list-platforms
    """
    if ctx.invoked_subcommand is None:
        typer.echo("Use --help for usage information")
        typer.echo("Available subcommands: server")
        raise typer.Exit(0)


if __name__ == "__main__":
    app()
