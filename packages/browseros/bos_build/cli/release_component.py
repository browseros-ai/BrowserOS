#!/usr/bin/env python3
"""Standalone component release planning and stamping commands."""

import json
import os
import subprocess
from pathlib import Path
from typing import List, Optional

import requests
import typer

from ..lib.env import EnvConfig
from ..lib.paths import get_package_root
from ..lib.r2 import get_r2_client
from ..lib.utils import log_error
from ..release.component_release import (
    GitComponentReleaseOperations,
    StandaloneReleaseRequest,
    resolve_standalone_release,
)
from ..release.components import AllocationRecord, stamp_component
from ..release.extensions.specs import spec_by_name
from ..release.feeds.render import extract_manifest_versions


app = typer.Typer(
    help="Resolve and stamp standalone component releases",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


def _repo_root(path: Optional[Path]) -> Path:
    return (path or get_package_root().parent.parent).resolve()


def _read_manifest(location: str) -> str:
    if location.startswith(("https://", "http://")):
        for attempt in range(3):
            try:
                response = requests.get(location, timeout=30)
                response.raise_for_status()
                return response.text
            except requests.RequestException:
                if attempt == 2:
                    raise
    return Path(location).read_text(encoding="utf-8")


def _manifest_allocations(
    component: str, locations: List[str]
) -> tuple[AllocationRecord, ...]:
    if not locations:
        return ()
    extension = spec_by_name(component)
    allocations = []
    for location in locations:
        versions = extract_manifest_versions(_read_manifest(location))
        version = versions.get(extension.extension_id)
        if version:
            allocations.append(
                AllocationRecord(
                    component=component,
                    version=version,
                    kind="release",
                    reference=location,
                    public=True,
                )
            )
    return tuple(allocations)


def _optional_output(path: Optional[Path], environment_name: str) -> Optional[Path]:
    if path is not None:
        return path
    value = os.environ.get(environment_name)
    return Path(value) if value else None


def _append(path: Optional[Path], content: str) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "a", encoding="utf-8") as stream:
        stream.write(content)


@app.command("resolve")
def resolve(
    component: str = typer.Option(..., "--component"),
    event_name: str = typer.Option(..., "--event-name"),
    default_branch: str = typer.Option(..., "--default-branch"),
    ref_name: str = typer.Option("", "--ref-name"),
    requested_version: str = typer.Option("", "--requested-version"),
    release_ref: str = typer.Option("", "--release-ref"),
    repo: Optional[str] = typer.Option(None, "--repo"),
    repo_root: Optional[Path] = typer.Option(None, "--repo-root"),
    remote: str = typer.Option("origin", "--remote"),
    manifest: Optional[List[str]] = typer.Option(None, "--manifest"),
    r2_allocations: bool = typer.Option(False, "--r2-allocations"),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
    github_summary: Optional[Path] = typer.Option(None, "--github-summary"),
) -> None:
    """Resolve one safe version and immutable source commit."""
    try:
        repository = repo or os.environ.get("GITHUB_REPOSITORY", "")
        if not repository:
            raise ValueError("--repo or GITHUB_REPOSITORY is required")
        operation_options = {}
        if r2_allocations:
            env = EnvConfig()
            client = get_r2_client(env)
            if client is None:
                raise RuntimeError("R2 client is required for immutable allocations")
            operation_options = {
                "r2_client": client,
                "r2_bucket": env.r2_bucket,
            }
        record = resolve_standalone_release(
            StandaloneReleaseRequest(
                component=component,
                event_name=event_name,
                default_branch=default_branch,
                ref_name=ref_name,
                requested_version=requested_version,
                release_ref=release_ref,
            ),
            GitComponentReleaseOperations(
                _repo_root(repo_root), repository, remote, **operation_options
            ),
            _manifest_allocations(component, manifest or []),
        )
        values = {
            **record.to_dict(),
            "names": component,
        }
        output = _optional_output(github_output, "GITHUB_OUTPUT")
        if output is None:
            typer.echo(json.dumps(values, sort_keys=True))
        else:
            _append(
                output,
                "".join(f"{name}={value}\n" for name, value in values.items()),
            )
        _append(
            _optional_output(github_summary, "GITHUB_STEP_SUMMARY"), record.summary()
        )
    except (
        OSError,
        ValueError,
        RuntimeError,
        requests.RequestException,
        subprocess.CalledProcessError,
    ) as exc:
        log_error(str(exc))
        raise typer.Exit(1)


@app.command("stamp")
def stamp(
    component: str = typer.Option(..., "--component"),
    version: str = typer.Option(..., "--version"),
    repo_root: Optional[Path] = typer.Option(None, "--repo-root"),
) -> None:
    """Stamp only the selected component manifest and lockfile."""
    try:
        changed = stamp_component(_repo_root(repo_root), component, version)
        typer.echo(
            json.dumps(
                {"changed": [str(path) for path in changed]}, sort_keys=True
            )
        )
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        log_error(str(exc))
        raise typer.Exit(1)
