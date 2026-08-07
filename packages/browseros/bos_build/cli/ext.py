#!/usr/bin/env python3
"""Extension packaging and release commands."""

import json
import os
from pathlib import Path
from typing import List, Optional

import typer

from ..core.context import Context
from ..core.runner import StepExecutionError, run as run_steps
from ..lib.notify import slack_subscriber
from ..lib.paths import get_package_root
from ..lib.r2 import get_r2_client
from ..lib.utils import log_error
from ..release.extensions.release import (
    build_pipeline,
    extension_names,
    resolve_extension_version,
    verify_versioned_crx_objects,
)
from ..release.extensions.specs import EXTENSION_SPECS

app = typer.Typer(
    help="Extension packaging & release",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)

_NAMES = ", ".join(spec.name for spec in EXTENSION_SPECS)


def _create_context(version: str) -> Context:
    root = get_package_root()
    try:
        ctx = Context(
            root_dir=root,
            chromium_src=root,
            architecture="",
            build_type="release",
            product="browseros",
        )
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)
    ctx.release_version = version
    return ctx


def _execute(ctx: Context, steps: List) -> None:
    try:
        run_steps(ctx, steps, name="ext-release", subscribers=(slack_subscriber(ctx),))
    except StepExecutionError as e:
        log_error(str(e))
        raise typer.Exit(1)
    except KeyboardInterrupt:
        raise typer.Exit(130)


@app.command("release")
def release(
    version: str = typer.Option(
        ..., "--version", "-v", help="Version stamped on every selected extension"
    ),
    source_sha: str = typer.Option(
        ..., "--source-sha", help="Prepared source commit bound to the CRX"
    ),
    name: Optional[str] = typer.Option(
        None, "--name", "-n", help=f"One extension ({_NAMES}); default: all"
    ),
    branch: Optional[str] = typer.Option(
        None,
        "--branch",
        help="Branch override for external-repo extensions "
        "(in-repo extensions build the current working tree)",
    ),
    chrome_binary: Optional[str] = typer.Option(
        None, "--chrome-binary", help="Chrome binary for --pack-extension"
    ),
):
    """Build and upload CRXs; update feeds with browseros release extensions."""
    try:
        steps = build_pipeline(
            version=version,
            name=name,
            branch=branch,
            chrome_binary=chrome_binary,
            source_sha=source_sha,
        )
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)

    _execute(_create_context(version), steps)


@app.command("resolve-version")
def resolve_version(
    name: str = typer.Option(..., "--name", "-n", help=f"Extension ({_NAMES}, all)"),
    source_sha: str = typer.Option(..., "--source-sha"),
    release_records: Path = typer.Option(..., "--release-records"),
    version: str = typer.Option("", "--version", "-v"),
    manifest: Optional[List[Path]] = typer.Option(None, "--manifest"),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
):
    """Resolve a safe extension version and emit reusable-workflow outputs."""
    try:
        records = json.loads(release_records.read_text(encoding="utf-8"))
        if not isinstance(records, list) or not all(
            isinstance(record, dict) for record in records
        ):
            raise ValueError("release records must be a JSON array of objects")
        manifests = [path.read_text(encoding="utf-8") for path in (manifest or [])]
        resolved = resolve_extension_version(
            extension=name,
            requested_version=version,
            release_sha=source_sha,
            release_records=records,
            manifest_contents=manifests,
        )
        names = extension_names(name)
    except (OSError, json.JSONDecodeError, ValueError) as e:
        log_error(str(e))
        raise typer.Exit(1)

    values = {
        "version": resolved,
        "tag": f"ext-{name}/v{resolved}" if len(names) == 1 else "",
        "release_sha": source_sha,
        "names": " ".join(names),
    }
    output_path = github_output
    if output_path is None and os.environ.get("GITHUB_OUTPUT"):
        output_path = Path(os.environ["GITHUB_OUTPUT"])
    if output_path is None:
        typer.echo(json.dumps(values, sort_keys=True))
        return
    with open(output_path, "a", encoding="utf-8") as output:
        for key, value in values.items():
            output.write(f"{key}={value}\n")


@app.command("verify")
def verify(
    version: str = typer.Option(..., "--version", "-v"),
    name: str = typer.Option("all", "--name", "-n", help=f"Extension ({_NAMES}, all)"),
    source_sha: str = typer.Option(..., "--source-sha"),
    output_dir: Optional[Path] = typer.Option(None, "--output-dir"),
):
    """Verify that selected versioned CRX objects exist in R2."""
    try:
        names = extension_names(name)
        ctx = _create_context(version)
        client = get_r2_client(ctx.env)
        if client is None:
            raise RuntimeError("Failed to create R2 client")
        verify_versioned_crx_objects(
            client,
            ctx.env.r2_bucket,
            version,
            names,
            source_sha,
            output_dir,
        )
    except (ValueError, RuntimeError) as e:
        log_error(str(e))
        raise typer.Exit(1)
