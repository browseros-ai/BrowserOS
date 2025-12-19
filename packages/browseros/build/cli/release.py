#!/usr/bin/env python3
"""Release CLI - Modular release automation for BrowserOS"""

from pathlib import Path
from typing import Optional

import typer

from ..common.context import Context
from ..common.module import ValidationError
from ..common.utils import log_info, log_error, log_success

from ..modules.release import (
    AVAILABLE_MODULES,
    ListModule,
    AppcastModule,
    GithubModule,
    PublishModule,
    DownloadModule,
)

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


def create_release_context(
    version: str,
    repo: Optional[str] = None,
) -> Context:
    """Create Context for release operations"""
    ctx = Context(
        root_dir=Path.cwd(),
        chromium_src=Path.cwd(),  # Not used for release ops
        architecture="",
        build_type="release",
    )
    ctx.release_version = version
    ctx.github_repo = repo or ""
    return ctx


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


@app.callback(invoke_without_command=True)
def main(
    ctx: typer.Context,
    version: Optional[str] = typer.Option(
        None, "--version", "-v", help="Version to operate on (e.g., 0.31.0)"
    ),
    list_artifacts: bool = typer.Option(
        False, "--list", "-l", help="List artifacts for version from R2"
    ),
    appcast: bool = typer.Option(
        False, "--appcast", "-a", help="Generate appcast XML snippets"
    ),
    publish: bool = typer.Option(
        False, "--publish", "-p", help="Publish to download/ paths (make live)"
    ),
    download: bool = typer.Option(
        False, "--download", "-d", help="Download artifacts to temp directory"
    ),
    os_filter: Optional[str] = typer.Option(
        None, "--os", help="Filter by OS: macos, windows, linux"
    ),
    show_modules: bool = typer.Option(
        False, "--show-modules", help="Show available modules and exit"
    ),
):
    """Release automation for BrowserOS

    \b
    Quick Operations (Flags):
      browseros release --list                        # List all available versions
      browseros release --list --version 0.31.0       # List artifacts for version
      browseros release --version 0.31.0 --appcast    # Generate appcast XML
      browseros release --version 0.31.0 --publish    # Publish to download/ paths
      browseros release --version 0.31.0 --download   # Download all artifacts
      browseros release --version 0.31.0 --download --os macos  # Download macOS only

    \b
    GitHub Release (Sub-command):
      browseros release github create --version 0.31.0
      browseros release github create --version 0.31.0 --publish

    \b
    Show Available Modules:
      browseros release --show-modules
    """
    if show_modules:
        log_info("\n📦 Available Release Modules:")
        log_info("-" * 50)
        for name, module_class in AVAILABLE_MODULES.items():
            log_info(f"  {name}: {module_class.description}")
        log_info("-" * 50)
        return

    # If subcommand invoked, let it handle things
    if ctx.invoked_subcommand is not None:
        return

    # Check if any flags specified
    has_flags = any([list_artifacts, appcast, publish, download])

    if not has_flags:
        typer.echo(
            "Error: Specify a flag (--list, --appcast, --publish, --download) or use a sub-command\n"
        )
        typer.echo("Use --help for usage information")
        typer.echo("Use --show-modules to see available modules")
        raise typer.Exit(1)

    # Version is required for all flags except --list
    requires_version = any([appcast, publish, download])
    if requires_version and not version:
        log_error("--version is required for this operation")
        raise typer.Exit(1)

    # Create context
    release_ctx = create_release_context(version or "")

    # Execute requested modules
    if list_artifacts:
        if version:
            log_info(f"📋 Listing artifacts for v{version}")
        else:
            log_info("📋 Listing all available releases")
        execute_module(release_ctx, ListModule())

    if appcast:
        log_info(f"📝 Generating appcast for v{version}")
        execute_module(release_ctx, AppcastModule())

    if publish:
        log_info(f"🚀 Publishing v{version} to download/ paths")
        execute_module(release_ctx, PublishModule())

    if download:
        log_info(f"📥 Downloading artifacts for v{version}")
        execute_module(release_ctx, DownloadModule(os_filter=os_filter))


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
):
    """Create GitHub release from R2 artifacts

    \b
    Examples:
      browseros release github create --version 0.31.0
      browseros release github create --version 0.31.0 --publish  # Also publish to download/
      browseros release github create --version 0.31.0 --no-draft # Create published release
    """
    ctx = create_release_context(version, repo)

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
