#!/usr/bin/env python3
"""Build and publish the BrowserOS neo Docker image from released Linux packages.

The image source lives in packages/browseros/docker/neo. Its Dockerfile picks
the .deb for each architecture from the version's public release.json, so it
builds without this module. What a release adds around it lives here: the
preflight that every requested architecture was actually published, the tag
policy, and the buildx invocation. The CLI and the release workflow are thin
callers of build_image.

Tag policy: a release installs the browser package for `<version>` plus the
claw-server and packaging of one repository commit, so the browser version alone
does not name an image. `<version>-<commit>` names one build and is never reused
for different source; `<version>` and `latest` are aliases that move to the newest
build. Shipping a server or packaging change for an already released browser is
another run of the workflow on the newer commit, with no new browser build.
"""

import json
import os
import re
import shutil
import subprocess
import tempfile
import urllib.error
import urllib.request
from contextlib import contextmanager, nullcontext
from dataclasses import dataclass, replace
from pathlib import Path
from typing import ContextManager, Iterator, Optional, Protocol

from ..lib.paths import get_package_root

DEFAULT_REPOSITORY = "ghcr.io/browseros-ai/neo"
# GitHub links a ghcr package to its repository from this annotation on the
# pushed index; the Dockerfile's matching LABEL only reaches the per-platform
# image configs. An unlinked package rejects pushes from the workflow's
# GITHUB_TOKEN.
IMAGE_SOURCE = "https://github.com/browseros-ai/BrowserOS"
RELEASE_JSON_URL = "https://cdn.browseros.com/releases/browserclaw/{version}/linux/release.json"

# Docker platform -> artifact key in a Linux release.json. Docker says amd64
# where the release says x64.
PLATFORM_PACKAGES = {
    "linux/amd64": "x64_deb",
    "linux/arm64": "arm64_deb",
}

_VERSION = re.compile(r"^\d+(\.\d+){1,3}$")
_REVISION = re.compile(r"^[0-9a-f]{40}$")
# The commit tag uses GitHub's short form, so it matches the commit links people see.
_SHORT_REVISION = 7
# claw-server compiles this marker into its binary (VERSION_MARKER in
# apps/claw-server/src/main.rs). Reading it avoids running a binary that may be
# built for another architecture, and cannot disagree with the binary installed.
_SERVER_VERSION_MARKER = re.compile(rb"browseros-claw-server-version=([0-9A-Za-z.+-]+);")
# The Ubuntu base image sets the version label to its own release (24.04), so
# every build overrides it.
VERSION_LABEL = "org.opencontainers.image.version"
REVISION_LABEL = "org.opencontainers.image.revision"
SERVER_VERSION_LABEL = "com.browseros.claw-server.version"
# Release package names carry the version and architecture, e.g.
# BrowserOS_neo_v0.51.1.4_arm64.deb.
_DEB_NAME = re.compile(r"^BrowserOS_neo_v(?P<version>\d+(?:\.\d+){1,3})_(?P<arch>amd64|arm64)\.deb$")


class ImageError(ValueError):
    """A request that cannot produce a correct image."""


@dataclass(frozen=True)
class ImageRequest:
    """What to build. Without push the image is loaded into local Docker,
    which holds one platform, so it defaults to the host's architecture. An
    explicit platform (or a .deb's own) may differ from the host on purpose,
    e.g. to test an amd64 package on Apple silicon under emulation. Optional
    server binaries override the package's server. `revision` is the full sha of
    the commit those servers and the Docker context come from. A revision
    requires the servers and pushing requires both, so every commit tag runs
    that commit's server. Without `version_tag` only that commit tag is
    published, so the shared <version> alias keeps pointing where it did. Key
    paths are BuildKit inputs; their contents are intentionally installed in the
    image, never read here."""

    version: str = ""
    repository: str = DEFAULT_REPOSITORY
    push: bool = False
    latest: bool = False
    version_tag: bool = True
    platforms: tuple[str, ...] = ()
    deb: Optional[Path] = None
    server_dir: Optional[Path] = None
    revision: str = ""
    capsolver_key_file: Optional[Path] = None
    twocaptcha_key_file: Optional[Path] = None


@dataclass(frozen=True)
class ImageResult:
    tags: tuple[str, ...]
    platforms: tuple[str, ...]
    digest: str  # registry digest of a pushed image; empty when loaded locally


class ImageOperations(Protocol):
    """The outside world build_image needs: the CDN, local Docker, buildx."""

    def fetch_release(self, version: str) -> Optional[dict]:
        """The version's Linux release.json, or None if it was never published."""
        ...

    def host_platform(self) -> str:
        """The Docker daemon's platform, e.g. linux/arm64."""
        ...

    def staged_deb(self, deb: Path) -> ContextManager[Path]:
        """A directory holding only `deb`, alive for the duration of the build."""
        ...

    def run_buildx(self, argv: list[str]) -> dict:
        """Runs `docker buildx build <argv>` and returns buildx's metadata."""
        ...


def _resolve_deb(request: ImageRequest, deb: Path) -> ImageRequest:
    """Fills version and platform from a release-named .deb, so the tag and
    platform cannot disagree with the package actually installed."""
    if request.push:
        raise ImageError("A local .deb is for testing and is never pushed; push a published version")
    match = _DEB_NAME.match(deb.name)
    if not match:
        if not request.version:
            raise ImageError(f"Cannot read a version from {deb.name}; pass --version")
        return request
    version, platform = match["version"], f"linux/{match['arch']}"
    if request.version and request.version != version:
        raise ImageError(f"{deb.name} is Neo {version}, not {request.version}")
    if request.platforms and request.platforms != (platform,):
        raise ImageError(f"{deb.name} is for {platform}")
    return replace(request, version=version, platforms=(platform,))


def _platforms(request: ImageRequest, ops: ImageOperations) -> tuple[str, ...]:
    if request.platforms:
        platforms = request.platforms
    elif request.push:
        platforms = tuple(PLATFORM_PACKAGES)
    else:
        platforms = (ops.host_platform(),)
    unknown = [p for p in platforms if p not in PLATFORM_PACKAGES]
    if unknown:
        raise ImageError(
            f"Unsupported platform {', '.join(unknown)}; "
            f"Neo ships {', '.join(PLATFORM_PACKAGES)}"
        )
    if not request.push and len(platforms) != 1:
        raise ImageError("Local Docker loads one platform at a time; pass one platform or --push")
    return platforms


def _require_published(version: str, platforms: tuple[str, ...], ops: ImageOperations) -> None:
    """Fail before any build minutes are spent on a package that does not exist."""
    release = ops.fetch_release(version)
    if release is None:
        raise ImageError(f"Neo {version} has no Linux release on the CDN")
    published = release.get("artifacts", {})
    missing = [PLATFORM_PACKAGES[p] for p in platforms if PLATFORM_PACKAGES[p] not in published]
    if missing:
        raise ImageError(f"Neo {version} has no {', '.join(missing)} on the CDN")


def _server_version(server_dir: Path, platforms: tuple[str, ...]) -> str:
    """The claw-server version every requested architecture's binary declares."""
    versions = {}
    for platform in platforms:
        arch = platform.split("/")[1]
        match = _SERVER_VERSION_MARKER.search((server_dir / arch / "browseros-claw-server-rs").read_bytes())
        if not match:
            raise ImageError(f"{arch}/browseros-claw-server-rs has no claw-server version marker")
        versions[arch] = match[1].decode()
    if len(set(versions.values())) != 1:
        found = ", ".join(f"{arch} {version}" for arch, version in versions.items())
        raise ImageError(f"Server binaries were built from different versions: {found}")
    return next(iter(versions.values()))


def build_image(request: ImageRequest, ops: ImageOperations, context: Path) -> ImageResult:
    """Builds the image for `request` from the Docker context at `context`.

    Raises ImageError, before running buildx, for a request that would publish
    the wrong thing: a malformed version, an architecture the version never
    shipped, :latest on an image that is not being pushed, or a commit tag or
    push without that commit's servers and full revision. A local .deb replaces the CDN lookup
    and is only ever loaded into local Docker.
    """
    if request.deb is not None:
        request = _resolve_deb(request, request.deb)
    if not _VERSION.match(request.version):
        raise ImageError(f"Invalid Neo version '{request.version}'; expected e.g. 0.52.0.1")
    if request.latest and not request.push:
        raise ImageError("--latest requires --push")
    if request.latest and not request.version_tag:
        raise ImageError("--latest also needs the version tag")
    if not request.version_tag and not request.revision:
        raise ImageError("An image without the version tag needs --revision for its commit tag")
    if (request.push or request.revision) and request.server_dir is None:
        raise ImageError("A commit-tagged build needs that commit's servers; pass --server-dir")
    if (request.push or request.revision) and not _REVISION.match(request.revision):
        raise ImageError(
            f"Invalid revision '{request.revision}'; pass the full 40-character commit sha with --revision"
        )
    platforms = _platforms(request, ops)
    server_version = ""
    if request.server_dir is not None:
        for platform in platforms:
            arch = platform.split("/")[1]
            if not (request.server_dir / arch / "browseros-claw-server-rs").is_file():
                raise ImageError(f"Server directory has no {arch}/browseros-claw-server-rs")
        server_version = _server_version(request.server_dir, platforms)
    keys = [
        (name, key_file)
        for name, key_file in (
            ("capsolver_api_key", request.capsolver_key_file),
            ("twocaptcha_api_key", request.twocaptcha_key_file),
        )
        if key_file is not None
    ]
    for name, key_file in keys:
        if not key_file.is_file():
            raise ImageError(f"{name} key file does not exist: {key_file}")
    if request.deb is None:
        _require_published(request.version, platforms, ops)

    image_version = request.version
    tags: tuple[str, ...] = ()
    if request.revision:
        image_version += f"-{request.revision[:_SHORT_REVISION]}"
        tags += (f"{request.repository}:{image_version}",)
    if request.version_tag:
        tags += (f"{request.repository}:{request.version}",)
    if request.latest:
        tags += (f"{request.repository}:latest",)
    argv = ["--platform", ",".join(platforms)]
    argv += ["--build-arg", f"NEO_VERSION={request.version}"]
    for tag in tags:
        argv += ["--tag", tag]
    # Labels only reach the final image config, so they never invalidate the
    # cached browser installation.
    argv += ["--label", f"{VERSION_LABEL}={image_version}"]
    if request.revision:
        argv += ["--label", f"{REVISION_LABEL}={request.revision}"]
    if server_version:
        argv += ["--label", f"{SERVER_VERSION_LABEL}={server_version}"]
    if request.push:
        # Index annotations fail on an export without an index, so request the
        # provenance attestation explicitly: it makes even a single-platform
        # push an index. Local loads stay plain images.
        argv += [
            "--push",
            "--provenance", "mode=min",
            "--annotation", f"index:org.opencontainers.image.source={IMAGE_SOURCE}",
        ]
    else:
        argv.append("--load")
    if request.server_dir is not None:
        argv += ["--build-context", f"neo-server={request.server_dir}"]
    # Re-run the final credentials stage even for keyless builds: secret
    # contents (and their removal) are not reliable cache invalidators. Keep
    # the expensive browser installation cached in the preceding runtime stage.
    argv += ["--no-cache-filter", "neo"]
    for name, key_file in keys:
        argv += ["--secret", f"id={name},src={key_file}"]
    staging = ops.staged_deb(request.deb) if request.deb is not None else nullcontext(None)
    with staging as deb_dir:
        if deb_dir is not None:
            # Overrides the Dockerfile's empty `neo-deb` stage.
            argv += ["--build-context", f"neo-deb={deb_dir}"]
        argv.append(str(context))
        metadata = ops.run_buildx(argv)
    digest = metadata.get("containerimage.digest", "") if request.push else ""
    return ImageResult(tags, platforms, digest)


def image_context() -> Path:
    """The Docker build context shipped in this repository."""
    return get_package_root() / "docker" / "neo"


class DockerOperations:
    """ImageOperations backed by the public CDN and the docker CLI."""

    def fetch_release(self, version: str) -> Optional[dict]:
        # Cloudflare rejects urllib's default User-Agent with 403 (error 1010),
        # so only 404 means "never published"; anything else is a real failure.
        request = urllib.request.Request(
            RELEASE_JSON_URL.format(version=version),
            headers={"User-Agent": "browseros-build"},
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as exc:
            if exc.code == 404:
                return None
            raise

    def host_platform(self) -> str:
        result = subprocess.run(
            ["docker", "version", "--format", "{{.Server.Os}}/{{.Server.Arch}}"],
            check=True,
            capture_output=True,
            text=True,
        )
        return result.stdout.strip()

    @contextmanager
    def staged_deb(self, deb: Path) -> Iterator[Path]:
        # buildx sends the whole named context to the builder, so it gets a
        # directory with only the package rather than the .deb's folder.
        with tempfile.TemporaryDirectory(prefix="neo-deb-") as directory:
            target = Path(directory) / deb.name
            try:
                os.link(deb, target)
            except OSError:
                shutil.copy2(deb, target)
            yield Path(directory)

    def run_buildx(self, argv: list[str]) -> dict:
        with tempfile.TemporaryDirectory(prefix="neo-buildx-") as directory:
            metadata = Path(directory) / "metadata.json"
            *flags, context = argv
            subprocess.run(
                ["docker", "buildx", "build", *flags, "--metadata-file", str(metadata), context],
                check=True,
            )
            return json.loads(metadata.read_text()) if metadata.exists() else {}
