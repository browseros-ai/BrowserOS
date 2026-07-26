#!/usr/bin/env python3
"""Source CLI - Chromium checkout provisioning and caching.

On a fresh remote runner the whole provisioning story is:

    browseros source ensure --root /work/chromium --step checkout
    browseros build --modules clean --chromium-src /work/chromium/src ...
    browseros source ensure --root /work/chromium --step sync

(checkout and sync are split so clean can run between them; clean
deletes hook-managed toolchains that sync restores.)
"""

from pathlib import Path
from typing import Optional

import typer

from ..lib.paths import get_package_root
from ..lib.utils import log_error, log_info
from ..steps.source import cache as source_cache
from ..steps.source.provision import STRATEGIES, ensure, read_pinned_version

app = typer.Typer(
    help="Chromium source provisioning",
    no_args_is_help=True,
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)

cache_app = typer.Typer(
    help="Checkout cache in R2 (for runners without WarpCache)",
    no_args_is_help=True,
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)
app.add_typer(cache_app, name="cache")


@app.command("ensure")
def ensure_cmd(
    root: Path = typer.Option(
        ...,
        "--root",
        help="gclient root: holds depot_tools/, .gclient and src/",
    ),
    strategy: str = typer.Option(
        "shallow",
        "--strategy",
        help=f"Checkout strategy: {', '.join(STRATEGIES)}",
    ),
    step: str = typer.Option(
        "all",
        "--step",
        help="checkout, sync, or all (split so clean can run between)",
    ),
    version_file: Optional[Path] = typer.Option(
        None,
        "--version-file",
        help="CHROMIUM_VERSION pin file (default: package root)",
    ),
):
    """Idempotently provision depot_tools + chromium src at the pinned tag."""
    if step not in ("checkout", "sync", "all"):
        log_error(f"Invalid --step '{step}'. Valid: checkout, sync, all")
        raise typer.Exit(1)
    if strategy not in STRATEGIES:
        log_error(f"Invalid --strategy '{strategy}'. Valid: {', '.join(STRATEGIES)}")
        raise typer.Exit(1)

    pin_file = version_file or get_package_root() / "CHROMIUM_VERSION"
    if not pin_file.exists():
        log_error(f"Version pin file not found: {pin_file}")
        raise typer.Exit(1)

    version = read_pinned_version(pin_file)
    log_info(f"Pinned Chromium version: {version}")
    log_info(f"Chromium root: {root.resolve()}")

    try:
        ensure(root.resolve(), version, strategy=strategy, step_name=step)
    except Exception as e:
        log_error(f"Provisioning failed: {e}")
        raise typer.Exit(1)


@cache_app.command("restore")
def cache_restore(
    key: str = typer.Option(..., "--key", help="cache key (no prefix/extension)"),
    root: Path = typer.Option(..., "--root", help="chromium gclient root dir"),
):
    """Restore the checkout cache; degrades to cache-miss, never fails."""
    source_cache.restore(key, root)


@cache_app.command("save")
def cache_save(
    key: str = typer.Option(..., "--key", help="cache key (no prefix/extension)"),
    root: Path = typer.Option(..., "--root", help="chromium gclient root dir"),
):
    """Save the checkout as a cache object (skips if the key exists)."""
    source_cache.save(key, root)


if __name__ == "__main__":
    app()
