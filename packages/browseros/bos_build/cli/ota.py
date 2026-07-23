#!/usr/bin/env python3
"""OTA CLI - Over-The-Air update automation for BrowserOS"""

from pathlib import Path
from typing import Optional

import typer

from ..core.context import Context
from ..lib.env import EnvConfig
from ..lib.notify import slack_subscriber
from ..core.runner import StepExecutionError, run as run_steps
from ..lib.sparkle import sparkle_sign_file
from ..lib.utils import log_info, log_error, log_success

from ..release.ota import ServerOTAModule
from ..release.ota.common import (
    get_appcast_path,
    promote_appcast_content,
    SERVER_PLATFORMS,
)
from ..release.feeds.publisher import FeedPublisher
from ..release.feeds.render import (
    extract_appcast_item_count,
    extract_appcast_version,
)
from ..release.feeds.spec import server_feed
from ..products import SERVER_BUNDLES
from ..products.server_binaries import server_ota_bundles_for_product

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
    """Run a single OTA step through the shared runner"""
    try:
        run_steps(ctx, [module], name="ota", subscribers=(slack_subscriber(ctx),))
    except StepExecutionError as e:
        log_error(str(e))
        raise typer.Exit(1)
    except KeyboardInterrupt:
        raise typer.Exit(130)


def _server_bundle_id(product: str) -> str:
    bundles = server_ota_bundles_for_product(product)
    if not bundles:
        log_error(f"Product '{product}' has no server bundle")
        raise typer.Exit(1)
    return bundles[0].id


def _feed_publisher() -> FeedPublisher:
    env = EnvConfig()
    if not env.has_r2_config():
        log_error(
            "R2 configuration not set. Required env vars: R2_ACCOUNT_ID, "
            "R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY"
        )
        raise typer.Exit(1)
    return FeedPublisher(env=env)


def _log_alternate_product_appcasts(
    selected_product: str, selected_bundle_id: str, channel: str
) -> None:
    """Suggest versioned staging appcasts owned by a different product."""
    for bundle in SERVER_BUNDLES:
        if bundle.id == selected_bundle_id:
            continue
        alternate_products = tuple(
            product_id
            for product_id in bundle.product_ids
            if product_id != selected_product
        )
        if not alternate_products:
            continue

        candidate_path = get_appcast_path(channel, bundle.id).resolve()
        try:
            version = extract_appcast_version(candidate_path.read_text())
        except (OSError, UnicodeError):
            continue
        if version is None:
            continue

        candidate_key = server_feed(bundle.id, channel).key
        for product_id in alternate_products:
            log_error(
                f"Did you mean --product {product_id}? "
                f"{candidate_key} carries {version}"
            )


@server_app.command("release")
def server_release(
    version: str = typer.Option(
        ..., "--version", "-v", help="Version to release (e.g., 0.0.69)"
    ),
    channel: str = typer.Option(
        "alpha", "--channel", "-c", help="Release channel: alpha or prod"
    ),
    platform: Optional[str] = typer.Option(
        None, "--platform", "-p",
        help="Platform(s) to process, comma-separated (darwin_arm64, darwin_x64, linux_arm64, linux_x64, windows_x64)"
    ),
    product: str = typer.Option(
        "browseros", "--product", help="Product whose server bundle to release"
    ),
):
    """Publish BrowserOS server OTA update

    Downloads server binaries from R2 (artifacts/server/latest/),
    signs them, creates Sparkle update packages, and uploads to R2.

    \b
    Full Release (all platforms):
      browseros ota server release --version 0.0.69 --channel alpha

    \b
    Single Platform:
      browseros ota server release --version 0.0.69 --platform darwin_arm64

    \b
    Multiple Platforms:
      browseros ota server release --version 0.0.69 --platform darwin_arm64,darwin_x64
    """
    log_info(f"🚀 BrowserOS Server OTA v{version}")
    log_info("=" * 70)

    ctx = create_ota_context()

    module = ServerOTAModule(
        version=version,
        channel=channel,
        platform_filter=platform,
        product_id=product,
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
    product: str = typer.Option(
        "browseros", "--product", help="Product whose server appcast to publish"
    ),
    publish: bool = typer.Option(
        False,
        "--publish",
        help="Write to R2 (default is a dry run: full XML + diff vs live)",
    ),
    allow_downgrade: bool = typer.Option(
        False, "--allow-downgrade", help="Override the version-downgrade guard"
    ),
):
    """Publish appcast XML to make the release live

    This is the final step after 'release' uploads artifacts. Runs through
    the feed publisher rails: title/link must match the channel, every
    enclosure is HEAD-checked, the live feed is backed up to feeds-history/
    before the write, and downgrades are refused.

    \b
    Preview the alpha appcast (dry run):
      browseros ota server release-appcast --channel alpha

    \b
    Publish it:
      browseros ota server release-appcast --channel alpha --publish
    """
    bundle_id = _server_bundle_id(product)
    try:
        spec = server_feed(bundle_id, channel)
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)

    source_path = (appcast_file or get_appcast_path(channel, bundle_id)).resolve()
    log_info(
        f"Resolved appcast: product={product} bundle={bundle_id} "
        f"channel={channel} spec={spec.key} source={source_path}"
    )
    if not source_path.exists():
        log_error(f"Appcast file not found: {source_path}")
        if not appcast_file:
            log_error(
                "Run 'browseros ota server release' first to generate the appcast"
            )
        raise typer.Exit(1)

    content = source_path.read_text()
    if extract_appcast_item_count(content) == 0:
        log_error(f"{spec.key} has no <item> entries: {source_path}")
        if appcast_file is None:
            _log_alternate_product_appcasts(product, bundle_id, channel)
        raise typer.Exit(1)

    publisher = _feed_publisher()
    if not publisher.publish(
        spec,
        content,
        publish=publish,
        allow_downgrade=allow_downgrade,
    ):
        raise typer.Exit(1)


@server_app.command("promote")
def server_promote(
    product: str = typer.Option(
        "browseros", "--product", help="Product whose server appcast to promote"
    ),
    publish: bool = typer.Option(
        False,
        "--publish",
        help="Write to R2 (default is a dry run: full XML + diff vs live)",
    ),
    allow_downgrade: bool = typer.Option(
        False, "--allow-downgrade", help="Override the version-downgrade guard"
    ),
):
    """Promote the live alpha server appcast to prod.

    Re-renders the alpha item with the prod feed's title/link (never a byte
    copy — the historical bug where prod carried alpha metadata), keeping
    version, pubDate and enclosures, then publishes through the rails.

    \b
    Preview (dry run):
      browseros ota server promote

    \b
    Promote for real:
      browseros ota server promote --publish
    """
    bundle_id = _server_bundle_id(product)
    alpha_spec = server_feed(bundle_id, "alpha")
    prod_spec = server_feed(bundle_id, "prod")

    publisher = _feed_publisher()
    live_alpha = publisher.fetch_live(alpha_spec.key)
    if live_alpha is None:
        log_error(f"No live {alpha_spec.key} to promote")
        raise typer.Exit(1)

    try:
        content = promote_appcast_content(live_alpha, prod_spec)
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)

    log_info(f"Promoting {alpha_spec.key} -> {prod_spec.key}")
    if not publisher.publish(
        prod_spec, content, publish=publish, allow_downgrade=allow_downgrade
    ):
        raise typer.Exit(1)
    if publish:
        log_success(f"✅ Promoted to {prod_spec.url}")


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

    log_info("\n🔐 Testing Sparkle Ed25519 signing")
    log_info(f"File: {file_path}")
    log_info("-" * 60)

    sig, length = sparkle_sign_file(file_path, env)
    if not sig:
        log_error("Signing failed")
        raise typer.Exit(1)

    log_success("✅ Signed successfully")
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
