#!/usr/bin/env python3
"""
BrowserOS Build System - Main Entry Point

Unified CLI for building, developing, and releasing BrowserOS browser.

Usage:
    # As installed command:
    browseros build --help

    # As module:
    python -m bos_build.browseros build --help
"""
import typer

from .cli import build, dev, ext, ota, product, release, source

app = typer.Typer(
    help="BrowserOS Build System",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)

build_app = typer.Typer(
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)
build_app.callback(invoke_without_command=True)(build.main)

app.add_typer(build_app, name="build", help="Build BrowserOS browser")
app.add_typer(source.app, name="source", help="Chromium checkout provisioning")
app.add_typer(product.app, name="product", help="Product definitions")
app.add_typer(dev.app, name="dev", help="Dev patch management")
app.add_typer(release.app, name="release", help="Release automation")
app.add_typer(ext.app, name="ext", help="Extension packaging & release")
app.add_typer(ota.app, name="ota", help="OTA update automation")


if __name__ == "__main__":
    app()
