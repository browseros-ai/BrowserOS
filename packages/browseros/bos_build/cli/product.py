#!/usr/bin/env python3
"""Product CLI - inspect and verify product definitions."""

from typing import Optional

import typer

from ..lib.paths import get_package_root
from ..lib.utils import log_error, log_info, log_success
from ..products import PRODUCTS
from ..products.doctor import diagnose

app = typer.Typer(
    help="Product definitions",
    no_args_is_help=True,
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


@app.command("list")
def list_products():
    """List registered products."""
    for product in PRODUCTS.values():
        log_info(f"  {product.id:14} {product.display_name} — {product.summary}")


@app.command("doctor")
def doctor(
    product_id: Optional[str] = typer.Argument(
        None, help="Product to check (default: all)"
    ),
):
    """Verify product identity data and on-disk assets."""
    if product_id and product_id not in PRODUCTS:
        log_error(
            f"Unknown product '{product_id}'. Valid: {', '.join(sorted(PRODUCTS))}"
        )
        raise typer.Exit(1)

    findings = diagnose(get_package_root(), product_id)
    if findings:
        log_error(f"Product doctor found {len(findings)} problem(s):")
        for finding in findings:
            log_error(f"  - {finding}")
        raise typer.Exit(1)

    scope = product_id or f"all products ({', '.join(PRODUCTS)})"
    log_success(f"Product doctor: {scope} healthy")


if __name__ == "__main__":
    app()
