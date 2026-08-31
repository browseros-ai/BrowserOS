#!/usr/bin/env python3
"""Prepared common-resource commands."""

import os
import subprocess
from pathlib import Path
from typing import List, Optional

import typer

from ..lib.paths import get_package_root
from ..lib.utils import log_error
from ..products.resource_sources import source_resources_for_product
from ..release.candidate import CandidateRecord
from ..release.components import read_component_version
from ..release.prepared_resources import (
    DEFAULT_BUNDLED_MANIFEST_URL,
    LocalPreparationOperations,
    PreparationRequest,
    prepare_common_resources,
)


app = typer.Typer(
    help="Prepare and validate portable browser resources",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


def _repo_root(path: Optional[Path]) -> Path:
    return (path or get_package_root().parent.parent).resolve()


def _parse_versions(values: Optional[List[str]]) -> dict[str, str]:
    versions = {}
    for value in values or []:
        name, separator, version = value.partition("=")
        if not separator or not name or not version or name in versions:
            raise ValueError(
                f"Invalid --component-version '{value}'; expected unique NAME=VERSION"
            )
        versions[name] = version
    return versions


def _write_outputs(path: Optional[Path], output: Path, digest: str) -> None:
    target = path
    if target is None and os.environ.get("GITHUB_OUTPUT"):
        target = Path(os.environ["GITHUB_OUTPUT"])
    if target is None:
        return
    with open(target, "a", encoding="utf-8") as stream:
        stream.write(f"prepared_resources={output.resolve()}\n")
        stream.write(f"manifest_sha256={digest}\n")


@app.command("prepare")
def prepare(
    output: Path = typer.Option(..., "--output"),
    candidate: Optional[Path] = typer.Option(None, "--candidate"),
    product: str = typer.Option("", "--product"),
    source_sha: str = typer.Option("", "--source-sha"),
    parent_sha: str = typer.Option("", "--parent-sha"),
    browser_version: str = typer.Option("", "--browser-version"),
    component_version: Optional[List[str]] = typer.Option(
        None, "--component-version"
    ),
    manifest_url: str = typer.Option(
        DEFAULT_BUNDLED_MANIFEST_URL, "--manifest-url"
    ),
    repo_root: Optional[Path] = typer.Option(None, "--repo-root"),
    chrome_binary: str = typer.Option("", "--chrome-binary"),
    rebuild: bool = typer.Option(False, "--rebuild"),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
) -> None:
    """Build or validate the common directory shared by browser lanes."""
    try:
        root = _repo_root(repo_root)
        if candidate is not None:
            if any(
                value
                for value in (
                    product,
                    source_sha,
                    parent_sha,
                    browser_version,
                    component_version,
                )
            ):
                raise ValueError(
                    "--candidate cannot be combined with explicit identity options"
                )
            record = CandidateRecord.from_path(candidate)
            product = record.product
            source_sha = record.candidate_sha
            parent_sha = record.parent_sha
            browser_version = record.browser_version
            versions = dict(record.component_versions)
        else:
            versions = _parse_versions(component_version)
            if not product or not source_sha or not browser_version:
                raise ValueError(
                    "Local preparation requires --product, --source-sha, and --browser-version"
                )
        onboarding_component = source_resources_for_product(product).onboarding_component
        if onboarding_component not in versions:
            versions[onboarding_component] = read_component_version(
                root, onboarding_component
            )
        request = PreparationRequest(
            product=product,
            parent_sha=parent_sha,
            source_sha=source_sha,
            browser_version=browser_version,
            component_versions=versions,
            output_dir=output,
            manifest_url=manifest_url,
        )
        operations = LocalPreparationOperations(root, chrome_binary)
        manifest = prepare_common_resources(request, operations, rebuild=rebuild)
        _write_outputs(github_output, output, manifest.digest())
        typer.echo(manifest.to_json(), nl=False)
    except (
        OSError,
        ValueError,
        RuntimeError,
        subprocess.CalledProcessError,
    ) as exc:
        log_error(str(exc))
        raise typer.Exit(1)
