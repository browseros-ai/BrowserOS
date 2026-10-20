#!/usr/bin/env python3
"""`browseros release docker`: build or publish the BrowserOS neo Docker image."""

import os
import subprocess
from pathlib import Path
from typing import List, Optional

import typer

from ..lib.utils import log_error, log_success
from ..release.docker_image import (
    DEFAULT_REPOSITORY,
    DockerOperations,
    ImageError,
    ImageRequest,
    build_image,
    image_context,
)


def docker_command(
    version: str = typer.Option(
        "", "--version", help="Neo version with Linux packages on the CDN, e.g. 0.52.0.1"
    ),
    push: bool = typer.Option(
        False, "--push", help="Build amd64 + arm64 and push them under one tag"
    ),
    latest: bool = typer.Option(
        False, "--latest", help="Also tag :latest; only for a promoted version"
    ),
    platform: Optional[List[str]] = typer.Option(
        None, "--platform", help="Platform override, e.g. linux/amd64 (repeatable)"
    ),
    deb: Optional[Path] = typer.Option(
        None, "--deb", help="Local .deb to test instead of the CDN; loaded locally, never pushed"
    ),
    repository: str = typer.Option(DEFAULT_REPOSITORY, "--repository"),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
) -> None:
    """Build the Neo Docker image: locally for this machine, or published for both architectures."""
    request = ImageRequest(
        version=version,
        repository=repository,
        push=push,
        latest=latest,
        platforms=tuple(platform or ()),
        deb=deb.resolve() if deb else None,
    )
    try:
        result = build_image(request, DockerOperations(), image_context())
    except (ImageError, OSError, subprocess.CalledProcessError) as exc:
        log_error(str(exc))
        raise typer.Exit(1)
    verb = "Pushed" if push else "Loaded into local Docker"
    log_success(f"{verb}: {', '.join(result.tags)} ({', '.join(result.platforms)})")
    if result.digest:
        log_success(f"Digest: {result.digest}")
    target = github_output or (Path(os.environ["GITHUB_OUTPUT"]) if os.environ.get("GITHUB_OUTPUT") else None)
    if target is not None:
        with open(target, "a", encoding="utf-8") as stream:
            stream.write(f"tags={','.join(result.tags)}\n")
            stream.write(f"digest={result.digest}\n")


def register(app: typer.Typer) -> None:
    """Attach the Docker image command to the release CLI."""
    app.command("docker")(docker_command)
