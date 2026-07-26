#!/usr/bin/env python3
"""Release CLI - Modular release automation for BrowserOS"""

from pathlib import Path
from typing import Optional

import typer

from ..core.context import Context
from ..core.products import ProductDescriptor, get_product_descriptor
from ..lib.notify import slack_subscriber
from ..lib.paths import get_package_root
from ..core.runner import StepExecutionError, run as run_steps
from ..lib.utils import log_info, log_error
from ..products import PRODUCTS

from ..release import (
    AVAILABLE_MODULES,
    ListModule,
    GithubModule,
    PublishModule,
    DownloadModule,
)
from ..release.list import DEFAULT_LIST_LIMIT
from . import release_feeds

app = typer.Typer(
    help="Release automation commands",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)

# GitHub sub-app for complex operations
github_app = typer.Typer(
    help="GitHub release operations",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)
app.add_typer(github_app, name="github")

release_feeds.register(app)

_PRODUCT_HELP = f"Product to operate on ({', '.join(PRODUCTS)})"


def _resolve_product(product_id: Optional[str]) -> ProductDescriptor:
    """Resolve --product to a descriptor with a CLI-friendly error."""
    try:
        return get_product_descriptor(product_id)
    except ValueError:
        log_error(
            f"Unknown product '{product_id}'. Valid: {', '.join(sorted(PRODUCTS))}"
        )
        raise typer.Exit(1)


def create_release_context(
    version: str,
    repo: Optional[str] = None,
    product: Optional[str] = None,
) -> Context:
    """Create Context for release operations.

    Anchored on the package root (not cwd) so release commands work from
    any directory; chromium_src is unused by release steps.
    """
    root = get_package_root()
    ctx = Context(
        root_dir=root,
        chromium_src=root,
        architecture="",
        build_type="release",
        product=_resolve_product(product),
    )
    ctx.release_version = version
    ctx.github_repo = repo or ""
    return ctx


def execute_module(ctx: Context, module) -> None:
    """Run a single release step through the shared runner"""
    try:
        run_steps(ctx, [module], name="release", subscribers=(slack_subscriber(ctx),))
    except StepExecutionError as e:
        log_error(str(e))
        raise typer.Exit(1)
    except KeyboardInterrupt:
        raise typer.Exit(130)


@app.callback(invoke_without_command=True)
def main(
    ctx: typer.Context,
    show_modules: bool = typer.Option(
        False, "--show-modules", help="Show available modules and exit"
    ),
):
    """Release automation for BrowserOS

    \b
    Commands:
      browseros release list                           # Newest releases per product
      browseros release list 0.31.0                    # Artifacts for a version
      browseros release appcast --version 0.31.0       # Generate appcast XML
      browseros release publish --version 0.31.0       # Publish to download/ paths
      browseros release download --version 0.31.0      # Download all artifacts
      browseros release github create --version 0.31.0

    Use --product to target a specific product (default: browseros).
    """
    if show_modules:
        log_info("\n📦 Available Release Modules:")
        log_info("-" * 50)
        for name, module_class in AVAILABLE_MODULES.items():
            log_info(f"  {name}: {module_class.description}")
        log_info("-" * 50)
        return

    if ctx.invoked_subcommand is None:
        typer.echo(ctx.get_help())
        raise typer.Exit(0)


@app.command("list")
def list_releases(
    version_arg: Optional[str] = typer.Argument(
        None, metavar="[VERSION]", help="Show artifact details for this version"
    ),
    version: Optional[str] = typer.Option(
        None, "--version", "-v", help="Show artifact details for this version"
    ),
    product: Optional[str] = typer.Option(
        None, "--product", help=f"{_PRODUCT_HELP}; default: all products"
    ),
    limit: int = typer.Option(
        DEFAULT_LIST_LIMIT, "--limit", "-n", min=1, help="Versions shown per product"
    ),
    show_all: bool = typer.Option(False, "--all", help="Show every version"),
):
    """List releases from R2 (newest first), or artifacts for one version.

    \b
    Examples:
      browseros release list                       # Newest 5 per product
      browseros release list --all                 # Every version
      browseros release list -n 10                 # Newest 10 per product
      browseros release list --product browserclaw # One product only
      browseros release list 0.31.0                # Artifact details
    """
    if version_arg and version and version_arg != version:
        log_error(f"Conflicting versions: '{version_arg}' vs --version '{version}'")
        raise typer.Exit(1)
    resolved_version = version_arg or version

    if resolved_version:
        release_ctx = create_release_context(resolved_version, product=product)
        log_info(f"📋 Listing artifacts for v{resolved_version}")
        execute_module(release_ctx, ListModule())
        return

    products = [_resolve_product(product)] if product else list(PRODUCTS.values())
    release_ctx = create_release_context("", product=product)
    log_info("📋 Listing available releases")
    execute_module(
        release_ctx,
        ListModule(products=products, limit=None if show_all else limit),
    )


@app.command("publish")
def publish(
    version: str = typer.Option(
        ..., "--version", "-v", help="Version to operate on (e.g., 0.31.0)"
    ),
    product: Optional[str] = typer.Option(None, "--product", help=_PRODUCT_HELP),
):
    """Publish versioned artifacts to download/ paths (make live)."""
    release_ctx = create_release_context(version, product=product)
    log_info(f"🚀 Publishing v{version} to download/ paths")
    execute_module(release_ctx, PublishModule())


@app.command("download")
def download(
    version: str = typer.Option(
        ..., "--version", "-v", help="Version to operate on (e.g., 0.31.0)"
    ),
    os_filter: Optional[str] = typer.Option(
        None, "--os", help="Filter by OS: macos, windows, linux"
    ),
    output: Optional[Path] = typer.Option(
        None, "--output", "-o", help="Output directory for downloads (default: temp dir)"
    ),
    product: Optional[str] = typer.Option(None, "--product", help=_PRODUCT_HELP),
):
    """Download release artifacts to a local directory.

    \b
    Examples:
      browseros release download --version 0.31.0
      browseros release download --version 0.31.0 --os macos
      browseros release download --version 0.31.0 --output ./downloads
    """
    release_ctx = create_release_context(version, product=product)
    log_info(f"📥 Downloading artifacts for v{version}")
    execute_module(release_ctx, DownloadModule(os_filter=os_filter, output_dir=output))


@github_app.command("create")
def github_create(
    version: str = typer.Option(
        ..., "--version", "-v", help="Version to release (e.g., 0.31.0)"
    ),
    draft: bool = typer.Option(
        True, "--draft/--publish", help="Create as draft (default: draft)"
    ),
    repo: Optional[str] = typer.Option(
        None, "--repo", "-r", help="GitHub repo (owner/name)"
    ),
    skip_upload: bool = typer.Option(
        False, "--skip-upload", help="Skip uploading artifacts to GitHub"
    ),
    title: Optional[str] = typer.Option(
        None, "--title", "-t", help="Release title (default: v{version})"
    ),
    publish_to_download: bool = typer.Option(
        False, "--publish", "-p", help="Also publish to download/ paths after creating release"
    ),
    product: Optional[str] = typer.Option(None, "--product", help=_PRODUCT_HELP),
):
    """Create GitHub release from R2 artifacts

    \b
    Examples:
      browseros release github create --version 0.31.0
      browseros release github create --version 0.31.0 --publish  # Also publish to download/
      browseros release github create --version 0.31.0 --no-draft # Create published release
    """
    ctx = create_release_context(version, repo, product)

    log_info(f"🚀 Creating GitHub release for v{version}")
    module = GithubModule(
        draft=draft,
        skip_upload=skip_upload,
        title=title,
    )
    execute_module(ctx, module)

    if publish_to_download:
        log_info(f"\n🚀 Publishing v{version} to download/ paths")
        execute_module(ctx, PublishModule())


if __name__ == "__main__":
    app()
