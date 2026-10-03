#!/usr/bin/env python3
"""Source lifecycle CLI: hold checkout ownership across each complete mutation.

CI normally uses `source ensure --root ... --reset`. Legacy split checkout/sync
commands remain available, but only the complete operation prepares source.
"""

from pathlib import Path
from typing import Optional

import typer

from ..core.checkout_lock import ChromiumCheckoutLock
from ..core.context import Context
from ..core.products import get_product_descriptor
from ..steps.setup.clean import CleanModule, clean_ci_outputs
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
        help="all prepares source; checkout/sync are legacy partial operations",
    ),
    version_file: Optional[Path] = typer.Option(
        None,
        "--version-file",
        help="CHROMIUM_VERSION pin file (default: package root)",
    ),
    reset: bool = typer.Option(
        False, "--reset", help="Discard source and dependency changes before syncing"
    ),
    lock_wait: bool = typer.Option(
        False, "--lock-wait", help="Wait for the checkout owner"
    ),
    repair_cached_depot_tools: bool = typer.Option(
        False,
        "--repair-cached-depot-tools",
        help=(
            "Repair line-ending-only tracked changes in an existing depot_tools "
            "checkout; intended for disposable CI caches"
        ),
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
        # Build adapters already hold this same lock. The source-only CLI owns
        # it here so stale-cache repair cannot race a build or another prepare.
        with ChromiumCheckoutLock(root / "src", product="source", wait=lock_wait):
            ensure(
                root.expanduser().resolve(),
                version,
                strategy=strategy,
                step_name=step,
                repair_cached_depot_tools=repair_cached_depot_tools,
                reset=reset,
            )
    except Exception as e:
        log_error(f"Provisioning failed: {e}")
        raise typer.Exit(1)


@app.command("clean-outputs")
def clean_outputs_cmd(
    root: Path = typer.Option(..., "--root", help="gclient root"),
    product: str = typer.Option("browseros", "--product"),
    arch: str = typer.Option("arm64", "--arch"),
    all_products: bool = typer.Option(
        False, "--all-products", help="Remove known CI product outputs and checkpoints"
    ),
    lock_wait: bool = typer.Option(False, "--lock-wait"),
):
    """Clean product output/resource state while preserving synchronized source."""
    src = root.expanduser().resolve() / "src"
    try:
        if arch not in ("arm64", "x64", "universal"):
            raise ValueError(f"Unsupported architecture: {arch}")
        ctx = Context(
            chromium_src=src,
            product=get_product_descriptor(product),
            build_type="release",
            architecture="arm64" if arch == "universal" else arch,
            plan_architectures=(arch,),
        )
        with ChromiumCheckoutLock(src, product=product, wait=lock_wait):
            if all_products:
                clean_ci_outputs(src)
            # Orphan resources live in the BrowserOS repository, outside the
            # Chromium cache; product context preserves the existing pruning.
            CleanModule().clean_outputs(ctx)
    except Exception as exc:
        log_error(f"Output cleanup failed: {exc}")
        raise typer.Exit(1)


def _products(value: str) -> tuple[str, ...]:
    if value == "all":
        return ("browseros", "browserclaw")
    if value in ("browseros", "browserclaw"):
        return (value,)
    raise ValueError(f"Unknown product set: {value}")


@app.command("workspace")
def workspace_cmd(
    base_src: Path = typer.Option(..., "--base-src"),
    products: str = typer.Option("browseros", "--products"),
    version_file: Optional[Path] = typer.Option(None, "--version-file"),
    runner_temp: Path = typer.Option(..., "--runner-temp"),
    run_tag: str = typer.Option(..., "--run-tag"),
):
    """Refresh the CI base once and copy every product before handing off."""
    from ..steps.source.workspace import prepare_workspaces

    try:
        version = read_pinned_version(
            version_file or get_package_root() / "CHROMIUM_VERSION"
        )
        prepare_workspaces(base_src, version, _products(products), runner_temp, run_tag)
    except Exception as exc:
        log_error(f"Workspace preparation failed: {exc}")
        raise typer.Exit(1)


@app.command("cleanup-workspaces")
def cleanup_workspaces_cmd(
    runner_temp: Path = typer.Option(..., "--runner-temp"),
    run_tag: str = typer.Option(..., "--run-tag"),
    products: str = typer.Option("all", "--products"),
):
    """Remove this run's marker-owned workspaces, including partial copies."""
    from ..steps.source.workspace import cleanup_workspaces

    try:
        cleanup_workspaces(runner_temp, run_tag, _products(products))
    except Exception as exc:
        log_error(f"Workspace cleanup failed: {exc}")
        raise typer.Exit(1)


@cache_app.command("restore")
def cache_restore(
    key: str = typer.Option(..., "--key", help="cache key (no prefix/extension)"),
    root: Path = typer.Option(..., "--root", help="chromium gclient root dir"),
):
    """Restore the checkout cache; degrades to cache-miss, never fails."""
    # Archive extraction must exclude the active lock directory (including for
    # older cache objects), or another process could lock a replacement inode.
    with ChromiumCheckoutLock(root / "src", product="source-cache"):
        source_cache.restore(key, root)


@cache_app.command("save")
def cache_save(
    key: str = typer.Option(..., "--key", help="cache key (no prefix/extension)"),
    root: Path = typer.Option(..., "--root", help="chromium gclient root dir"),
):
    """Save the checkout as a cache object (skips if the key exists)."""
    with ChromiumCheckoutLock(root / "src", product="source-cache"):
        source_cache.save(key, root)


if __name__ == "__main__":
    app()
