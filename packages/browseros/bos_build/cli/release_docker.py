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
    PLATFORM_PACKAGES,
    DockerOperations,
    ImageError,
    ImageRequest,
    build_image,
    image_context,
    publish_index,
)


def docker_command(
    version: str = typer.Option(
        "", "--version", help="Neo version with Linux packages on the CDN, e.g. 0.52.0.1"
    ),
    push: bool = typer.Option(
        False, "--push", help="Build amd64 + arm64 and push them under one tag"
    ),
    push_by_digest: bool = typer.Option(
        False, "--push-by-digest",
        help="Push the one --platform image just built here, untagged, from the build cache; "
             "prints its digest for --from-digests",
    ),
    from_digests: Optional[Path] = typer.Option(
        None, "--from-digests",
        help="With --push: tag one index over images already pushed by digest, "
             "read from <dir>/amd64 and <dir>/arm64, instead of building",
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
    server_dir: Optional[Path] = typer.Option(
        None, "--server-dir",
        help="Override servers from <dir>/<arch>/browseros-claw-server-rs; required with --push",
    ),
    revision: str = typer.Option(
        "", "--revision",
        help="Full commit sha the servers and Docker context come from; adds the <version>-<commit> tag. Needs --server-dir; required with --push",
    ),
    capsolver_key_file: Optional[Path] = typer.Option(
        None, "--capsolver-key-file", envvar="BROWSEROS_CAPSOLVER_API_KEY_FILE",
        help="CapSolver key file to embed in the image (readable by image users)",
    ),
    twocaptcha_key_file: Optional[Path] = typer.Option(
        None, "--twocaptcha-key-file", envvar="BROWSEROS_2CAPTCHA_API_KEY_FILE",
        help="2Captcha key file to embed in the image (readable by image users)",
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
        server_dir=server_dir.resolve() if server_dir else None,
        revision=revision,
        capsolver_key_file=capsolver_key_file.resolve() if capsolver_key_file else None,
        twocaptcha_key_file=twocaptcha_key_file.resolve() if twocaptcha_key_file else None,
        push_by_digest=push_by_digest,
    )
    try:
        if from_digests is not None:
            if not push:
                raise ImageError("--from-digests tags a published index; pass --push")
            digests = {
                platform: (from_digests / platform.split("/")[1]).read_text(encoding="utf-8").strip()
                for platform in PLATFORM_PACKAGES
                if (from_digests / platform.split("/")[1]).is_file()
            }
            result = publish_index(request, digests, DockerOperations())
        else:
            result = build_image(request, DockerOperations(), image_context())
    except (ImageError, OSError, subprocess.CalledProcessError) as exc:
        log_error(str(exc))
        raise typer.Exit(1)
    if push_by_digest:
        log_success(f"Pushed by digest, untagged ({', '.join(result.platforms)})")
    else:
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
