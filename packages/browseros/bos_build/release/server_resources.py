#!/usr/bin/env python3
"""Target-aware local server resource providers."""

import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import AbstractSet, Callable, Mapping, Optional

from ..products.resource_sources import source_resources_for_product
from ..products.server_binaries import ServerBundle, server_bundles_for_product
from .components import normalize_component_version, read_component_version


@dataclass(frozen=True)
class ServerTarget:
    """One concrete server build target."""

    id: str
    os: str
    arch: str
    cargo_triple: str


@dataclass(frozen=True)
class ServerResourceResult:
    """Validated server resource output for one target."""

    product: str
    target: str
    version: str
    source_sha: str
    destination: Path
    manifest_sha256: str


TARGETS: Mapping[str, ServerTarget] = {
    "linux-x64": ServerTarget(
        id="linux-x64",
        os="linux",
        arch="x64",
        cargo_triple="x86_64-unknown-linux-gnu",
    ),
    "windows-x64": ServerTarget(
        id="windows-x64",
        os="windows",
        arch="x64",
        cargo_triple="x86_64-pc-windows-msvc",
    ),
    "darwin-arm64": ServerTarget(
        id="darwin-arm64",
        os="macos",
        arch="arm64",
        cargo_triple="aarch64-apple-darwin",
    ),
    "darwin-x64": ServerTarget(
        id="darwin-x64",
        os="macos",
        arch="x64",
        cargo_triple="x86_64-apple-darwin",
    ),
}


RunCommand = Callable[[tuple[str, ...], Path, Mapping[str, str]], None]
Which = Callable[[str], Optional[str]]
RustTargets = Callable[[], AbstractSet[str]]


def _default_run(
    command: tuple[str, ...], cwd: Path, environment: Mapping[str, str]
) -> None:
    subprocess.run(list(command), cwd=cwd, env=dict(environment), check=True)


def _default_rust_targets() -> AbstractSet[str]:
    result = subprocess.run(
        ["rustup", "target", "list", "--installed"],
        capture_output=True,
        text=True,
        check=True,
    )
    return frozenset(result.stdout.splitlines())


def _host_platform() -> str:
    names = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}
    return names.get(platform.system(), platform.system().lower())


def target_ids_for_lane(platform_name: str, architecture: str) -> tuple[str, ...]:
    """Resolve one browser lane to concrete server targets."""
    if platform_name == "macos" and architecture == "universal":
        return ("darwin-arm64", "darwin-x64")
    prefix = "darwin" if platform_name == "macos" else platform_name
    target = f"{prefix}-{architecture}"
    if target not in TARGETS:
        valid = ", ".join(sorted(TARGETS))
        raise ValueError(f"Unsupported server target {target}. Valid: {valid}")
    return (target,)


def _server_bundle(product: str) -> ServerBundle:
    bundles = server_bundles_for_product(product)
    if len(bundles) != 1:
        raise ValueError(f"Product {product} must own exactly one server bundle")
    return bundles[0]


def server_build_command(
    product: str,
    target_id: str,
    agent_root: Path,
    cargo_target_dir: Path,
) -> tuple[tuple[str, ...], Path]:
    """Return the source builder command and its expected output."""
    try:
        target = TARGETS[target_id]
    except KeyError as exc:
        raise ValueError(f"Unknown server target {target_id}") from exc
    bundle = _server_bundle(product)
    if bundle.source_builder == "bun":
        return (
            (
                "bun",
                "scripts/build/server.ts",
                f"--target={target.id}",
                "--no-upload",
            ),
            agent_root / "dist/prod/server" / target.id,
        )
    suffix = ".exe" if target.os == "windows" else ""
    binary = cargo_target_dir / target.cargo_triple / "release" / (
        f"browseros-claw-server-rs{suffix}"
    )
    return (
        (
            "cargo",
            "build",
            "--release",
            "--locked",
            "-p",
            "claw-server-rust",
            "--bin",
            "browseros-claw-server-rs",
            "--target",
            target.cargo_triple,
            "--manifest-path",
            str(agent_root / "Cargo.toml"),
        ),
        binary,
    )


def _metadata_digest(document: Mapping[str, object]) -> str:
    encoded = json.dumps(document, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _read_metadata(destination: Path) -> dict[str, object]:
    path = destination / "artifact-metadata.json"
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"Server artifact metadata is unreadable: {path}") from exc
    if not isinstance(document, dict):
        raise ValueError("Server artifact metadata must be an object")
    return document


def _validate_file_entry(destination: Path, entry: object) -> str:
    if not isinstance(entry, dict):
        raise ValueError("Server artifact file entries must be objects")
    relative = entry.get("path")
    size = entry.get("size")
    sha256 = entry.get("sha256")
    if not isinstance(relative, str) or "\\" in relative:
        raise ValueError("Server artifact file path is invalid")
    path = PurePosixPath(relative)
    if path.is_absolute() or ".." in path.parts or path == PurePosixPath("."):
        raise ValueError(f"Server artifact file path is unsafe: {relative}")
    absolute = destination.joinpath(*path.parts)
    if not absolute.is_file() or absolute.is_symlink():
        raise ValueError(f"Server artifact file is missing: {relative}")
    if not isinstance(size, int) or absolute.stat().st_size != size:
        raise ValueError(f"Server artifact size mismatch: {relative}")
    if not isinstance(sha256, str) or _sha256(absolute) != sha256:
        raise ValueError(f"Server artifact checksum mismatch: {relative}")
    return relative


def validate_server_resources(
    destination: Path,
    *,
    product: str,
    target: str,
    version: str,
    source_sha: str,
) -> ServerResourceResult:
    """Validate a staged server resource directory and its provenance."""
    target_spec = TARGETS.get(target)
    if target_spec is None:
        raise ValueError(f"Unknown server target {target}")
    source = source_resources_for_product(product)
    normalized_version = normalize_component_version(
        source.server_component, version
    )
    document = _read_metadata(destination)
    if document.get("version") != normalized_version:
        raise ValueError("Server artifact version does not match")
    if document.get("target") != target:
        raise ValueError("Server artifact target does not match")
    if document.get("sourceSha") != source_sha:
        raise ValueError("Server artifact source SHA does not match")
    files = document.get("files")
    if not isinstance(files, list) or not files:
        raise ValueError("Server artifact files must be a non-empty list")
    declared = {_validate_file_entry(destination, entry) for entry in files}
    actual = {
        path.relative_to(destination).as_posix()
        for path in destination.rglob("*")
        if path.is_file() and path.name != "artifact-metadata.json"
    }
    if declared != actual:
        raise ValueError("Server artifact contains undeclared or missing files")

    bundle = _server_bundle(product)
    suffix = ".exe" if target_spec.os == "windows" else ""
    runtime_path = f"resources/bin/{bundle.runtime_binary_name}{suffix}"
    if runtime_path not in declared:
        raise ValueError(f"Server artifact is missing runtime binary {runtime_path}")
    if target_spec.os != "windows":
        runtime = destination.joinpath(*PurePosixPath(runtime_path).parts)
        if not runtime.stat().st_mode & 0o100:
            raise ValueError(f"Server runtime is not executable: {runtime_path}")
    if product == "browserclaw":
        if "resources/skills/browserclaw/SKILL.md" not in declared:
            raise ValueError("BrowserOS neo server artifact is missing its skill")
        if any("browseros-claw-server-rs" in path for path in declared):
            raise ValueError("BrowserOS neo staged the legacy -rs runtime name")
    return ServerResourceResult(
        product=product,
        target=target,
        version=normalized_version,
        source_sha=source_sha,
        destination=destination,
        manifest_sha256=_metadata_digest(document),
    )


class ServerResourceBuilder:
    """Build, stage, and validate one product's server targets."""

    def __init__(
        self,
        repo_root: Path,
        *,
        host_platform: str = "",
        run: RunCommand = _default_run,
        which: Which = shutil.which,
        rust_targets: RustTargets = _default_rust_targets,
        cargo_target_dir: Optional[Path] = None,
    ) -> None:
        self.repo_root = repo_root.resolve()
        self.agent_root = self.repo_root / "packages/browseros-agent"
        self.browseros_root = self.repo_root / "packages/browseros"
        self.host_platform = host_platform or _host_platform()
        self.run = run
        self.which = which
        self.rust_targets = rust_targets
        self.cargo_target_dir = (
            cargo_target_dir or self.agent_root / "target"
        ).resolve()

    def _preflight(self, product: str, target: ServerTarget) -> ServerBundle:
        bundle = _server_bundle(product)
        tool = "bun" if bundle.source_builder == "bun" else "cargo"
        if bundle.source_builder == "cargo" and self.host_platform != target.os:
            raise RuntimeError(
                f"BrowserOS neo target {target.id} requires host {target.os}; "
                f"current host is {self.host_platform}"
            )
        if self.which(tool) is None:
            action = "install Bun" if tool == "bun" else "install Rust and Cargo"
            raise RuntimeError(f"Cannot build {target.id}: {action}")
        if bundle.source_builder == "cargo":
            if self.which("rustup") is None:
                raise RuntimeError(f"Cannot build {target.id}: install Rust with rustup")
            try:
                installed_targets = self.rust_targets()
            except (OSError, subprocess.CalledProcessError) as exc:
                raise RuntimeError("Could not query installed Rust targets") from exc
            if target.cargo_triple not in installed_targets:
                raise RuntimeError(
                    f"Cannot build {target.id}: run rustup target add "
                    f"{target.cargo_triple}"
                )
        return bundle

    def preflight(self, *, product: str, target: str) -> None:
        """Validate host and tool support for one server target."""
        try:
            target_spec = TARGETS[target]
        except KeyError as exc:
            raise ValueError(f"Unknown server target {target}") from exc
        self._preflight(product, target_spec)

    def prepare(
        self,
        *,
        product: str,
        target: str,
        version: str,
        source_sha: str,
    ) -> ServerResourceResult:
        """Prepare one concrete server target from the current checkout."""
        if not re.fullmatch(r"[0-9a-fA-F]{40}", source_sha):
            raise ValueError("Server resources require a full source SHA")
        try:
            target_spec = TARGETS[target]
        except KeyError as exc:
            raise ValueError(f"Unknown server target {target}") from exc
        bundle = self._preflight(product, target_spec)
        source = source_resources_for_product(product)
        normalized_version = normalize_component_version(
            source.server_component, version
        )
        source_version = read_component_version(
            self.agent_root.parent.parent,
            source.server_component,
        )
        if source_version != normalized_version:
            raise ValueError(
                f"Server source version {source_version} does not match "
                f"requested version {normalized_version}"
            )
        destination = self.browseros_root / bundle.local_resources_root / target
        if destination.exists():
            shutil.rmtree(destination)

        command, artifact = server_build_command(
            product, target, self.agent_root, self.cargo_target_dir
        )
        environment = {**os.environ, "CARGO_TARGET_DIR": str(self.cargo_target_dir)}
        self.run(command, self.agent_root, environment)
        if bundle.source_builder == "bun":
            self._stage_bun(
                artifact,
                destination,
                bundle,
                target_spec,
                normalized_version,
                source_sha,
            )
        else:
            self._stage_rust(
                artifact,
                destination,
                bundle,
                target_spec,
                normalized_version,
                source_sha,
            )
        return validate_server_resources(
            destination,
            product=product,
            target=target,
            version=normalized_version,
            source_sha=source_sha,
        )

    def _stage_bun(
        self,
        artifact: Path,
        destination: Path,
        bundle: ServerBundle,
        target: ServerTarget,
        version: str,
        source_sha: str,
    ) -> None:
        if not artifact.is_dir():
            raise FileNotFoundError(f"Bun server build did not produce {artifact}")
        document = _read_metadata(artifact)
        if document.get("version") != version or document.get("target") != target.id:
            raise ValueError("Bun server artifact identity does not match the request")
        shutil.copytree(artifact, destination)
        suffix = ".exe" if target.os == "windows" else ""
        runtime = destination / "resources/bin" / f"{bundle.runtime_binary_name}{suffix}"
        if not runtime.is_file():
            raise FileNotFoundError(f"Bun server artifact is missing {runtime}")
        if target.os != "windows":
            runtime.chmod(runtime.stat().st_mode | 0o755)
        document["sourceSha"] = source_sha
        (destination / "artifact-metadata.json").write_text(
            json.dumps(document, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    def _stage_rust(
        self,
        artifact: Path,
        destination: Path,
        bundle: ServerBundle,
        target: ServerTarget,
        version: str,
        source_sha: str,
    ) -> None:
        if not artifact.is_file():
            raise FileNotFoundError(f"Cargo build did not produce {artifact}")
        suffix = ".exe" if target.os == "windows" else ""
        runtime = destination / "resources/bin" / f"{bundle.runtime_binary_name}{suffix}"
        runtime.parent.mkdir(parents=True)
        shutil.copy2(artifact, runtime)
        if target.os != "windows":
            runtime.chmod(0o755)
        skill_source = self.agent_root / "resources/skills/browserclaw/SKILL.md"
        if not skill_source.is_file():
            raise FileNotFoundError(f"BrowserOS neo skill is missing: {skill_source}")
        skill = destination / "resources/skills/browserclaw/SKILL.md"
        skill.parent.mkdir(parents=True)
        shutil.copy2(skill_source, skill)
        files = [runtime, skill]
        document = {
            "version": version,
            "target": target.id,
            "sourceSha": source_sha,
            "files": [
                {
                    "path": path.relative_to(destination).as_posix(),
                    "size": path.stat().st_size,
                    "sha256": _sha256(path),
                }
                for path in files
            ],
        }
        (destination / "artifact-metadata.json").write_text(
            json.dumps(document, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
