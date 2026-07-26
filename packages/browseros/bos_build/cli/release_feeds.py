#!/usr/bin/env python3
"""Feed-publisher release commands.

Kept in their own module (cli/release.py only calls register()) so the
parallel per-product release-CLI rework doesn't collide with these
additions. Context/execute helpers are intentionally local for the same
reason — importing them from cli/release.py would be a circular import.
"""

from typing import List

import typer

from ..core.context import Context
from ..lib.env import EnvConfig
from ..lib.notify import slack_subscriber
from ..lib.paths import get_package_root
from ..core.runner import StepExecutionError, run as run_steps
from ..lib.utils import log_error
from ..release.appcast import AppcastModule
from ..release.extensions.manifests import ExtensionsFeedModule, parse_set_options
from ..release.feeds.publisher import FeedPublisher

feeds_app = typer.Typer(
    help="Update-feed inspection",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


def _create_context(version: str = "", product: str = "browseros") -> Context:
    root = get_package_root()
    try:
        ctx = Context(
            root_dir=root,
            chromium_src=root,
            architecture="",
            build_type="release",
            product=product,
        )
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)
    ctx.release_version = version
    return ctx


def _execute(ctx: Context, module) -> None:
    try:
        run_steps(ctx, [module], name="release", subscribers=(slack_subscriber(ctx),))
    except StepExecutionError as e:
        log_error(str(e))
        raise typer.Exit(1)
    except KeyboardInterrupt:
        raise typer.Exit(130)


def appcast_command(
    version: str = typer.Option(
        ..., "--version", "-v", help="Release version to feed (e.g., 0.47.0.2)"
    ),
    product: str = typer.Option(
        "browseros", "--product", help="Product whose browser feeds to generate"
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
    """Generate complete browser appcast feeds from R2 release metadata.

    \b
    Dry run (prints full XML + diff vs live):
      browseros release appcast --version 0.47.0.2

    \b
    Publish (backs up live feeds to feeds-history/ first):
      browseros release appcast --version 0.47.0.2 --publish
    """
    ctx = _create_context(version, product)
    _execute(
        ctx,
        AppcastModule(
            product_id=product, publish=publish, allow_downgrade=allow_downgrade
        ),
    )


def extensions_command(
    channel: str = typer.Option(
        ..., "--channel", "-c", help="Target channel: alpha or prod"
    ),
    set_versions: List[str] = typer.Option(
        [],
        "--set",
        help="Pin an extension version as name=version (repeatable); "
        "extensions not set carry over from the live manifests",
    ),
    publish: bool = typer.Option(
        False,
        "--publish",
        help="Write to R2 (default is a dry run: full files + diff vs live)",
    ),
    allow_downgrade: bool = typer.Option(
        False, "--allow-downgrade", help="Override the version-downgrade guard"
    ),
):
    """Regenerate update-manifest, extensions.json and bundled-manifest coherently.

    \b
    Bump the agent on alpha (dry run):
      browseros release extensions --channel alpha --set agent=0.0.118

    \b
    Publish, pinning two extensions:
      browseros release extensions --channel alpha --set agent=0.0.118 \\
        --set bugreporter=54.0.0.0 --publish
    """
    try:
        versions = parse_set_options(set_versions)
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)

    ctx = _create_context()
    _execute(
        ctx,
        ExtensionsFeedModule(
            channel=channel,
            set_versions=versions,
            publish=publish,
            allow_downgrade=allow_downgrade,
        ),
    )


@feeds_app.command("status")
def feeds_status():
    """Show every feed key with its live version and last-published backup.

    \b
      browseros release feeds status
    """
    env = EnvConfig()
    if not env.has_r2_config():
        log_error(
            "R2 configuration not set. Required env vars: R2_ACCOUNT_ID, "
            "R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY"
        )
        raise typer.Exit(1)

    statuses = FeedPublisher(env=env).collect_status()

    header = (
        f"{'KEY':<42} {'KIND':<10} {'CHANNEL':<7} "
        f"{'LIVE VERSION':<58} LAST PUBLISHED"
    )
    print(header)
    print("-" * len(header))
    for status in statuses:
        spec = status.spec
        key = spec.key + ("" if spec.publishable else " (unpublishable)")
        channel = spec.channel or "-"
        live = status.live_version if status.live_version is not None else "absent"
        published = status.last_published or "-"
        print(f"{key:<42} {spec.kind:<10} {channel:<7} {live:<58} {published}")


def register(app: typer.Typer) -> None:
    """Attach the feed-publisher commands to the release CLI."""
    app.command("appcast")(appcast_command)
    app.command("extensions")(extensions_command)
    app.add_typer(feeds_app, name="feeds")
