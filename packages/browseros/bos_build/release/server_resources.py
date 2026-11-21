#!/usr/bin/env python3
"""Build a product's server from the checkout into its local resource folder.

Source-mode browser builds call this in place of downloading the published
server zip: Cargo compiles the bundle's binary for one target, then the
product's resource manifest (the file the release packager in
``packages/browseros-agent/packages/build-server-tools`` stages) adds the files
that ship beside it, and ``artifact-metadata.json`` records each file's digest.
"""

import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import AbstractSet, Callable, Literal, Mapping, Optional, Union, cast, get_args

from ..lib.env import EnvConfig
from ..lib.r2 import download_from_r2
from ..products.resource_sources import source_resources_for_product
from ..products.server_binaries import ServerBundle, server_bundles_for_product
from .components import (
    component_for_source,
    normalize_component_version,
    read_component_version,
)

TargetOs = Literal["linux", "macos", "windows"]
TargetArch = Literal["arm64", "x64"]
_TARGET_OS_VALUES = frozenset(get_args(TargetOs))
_TARGET_ARCH_VALUES = frozenset(get_args(TargetArch))

# Manifest R2 keys are relative to the vendor prefix. This mirrors
# defaultR2DownloadPrefix in scripts/build/server/descriptor.ts; change both
# together or source builds and release zips stage different files.
VENDOR_R2_PREFIX = "artifacts/vendor"


@dataclass(frozen=True)
class ServerTarget:
    """One concrete server build target."""

    id: str
    os: TargetOs
    arch: TargetArch
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


@dataclass(frozen=True)
class R2Source:
    """A manifest file fetched from R2, keyed under ``VENDOR_R2_PREFIX``."""

    key: str


@dataclass(frozen=True)
class LocalSource:
    """A manifest file copied from the checkout, relative to the agent root."""

    path: str


@dataclass(frozen=True)
class ResourceRule:
    """One entry of a server resource manifest.

    The schema is build-server-tools' ``src/manifest.ts``. A rule applies to a
    target when its ``os`` and ``arch`` lists are absent or name the target.
    """

    name: str
    source: Union[R2Source, LocalSource]
    destination: str
    os: Optional[tuple[TargetOs, ...]] = None
    arch: Optional[tuple[TargetArch, ...]] = None
    executable: bool = False

    def applies_to(self, target: ServerTarget) -> bool:
        return (self.os is None or target.os in self.os) and (
            self.arch is None or target.arch in self.arch
        )


RunCommand = Callable[[tuple[str, ...], Path, Mapping[str, str]], None]
Which = Callable[[str], Optional[str]]
RustTargets = Callable[[], AbstractSet[str]]
DownloadR2 = Callable[[str, Path], bool]
R2Configured = Callable[[], bool]


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


def _default_r2_configured() -> bool:
    return EnvConfig().has_r2_config()


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


def _string_list(
    item: Mapping[str, object], field: str, allowed: AbstractSet[str], name: str
) -> Optional[tuple[str, ...]]:
    values = item.get(field)
    if values is None:
        return None
    if not isinstance(values, list) or not all(
        isinstance(value, str) for value in values
    ):
        raise ValueError(f"Resource rule {name} {field} must be a list of strings")
    invalid = [value for value in values if value not in allowed]
    if invalid:
        raise ValueError(
            f"Resource rule {name} has invalid {field} value(s): {', '.join(invalid)}"
        )
    return tuple(values)


def _parse_rule(raw: object) -> ResourceRule:
    if not isinstance(raw, dict):
        raise ValueError("Resource manifest contains an invalid rule entry")
    name = raw.get("name")
    if not isinstance(name, str) or not name.strip():
        raise ValueError("Resource rule is missing name")
    source = raw.get("source")
    if not isinstance(source, dict):
        raise ValueError(f"Resource rule {name} source must be an object")
    kind, key, local_path = source.get("type"), source.get("key"), source.get("path")
    parsed_source: Union[R2Source, LocalSource]
    if kind == "r2" and isinstance(key, str) and key:
        parsed_source = R2Source(key)
    elif kind == "local" and isinstance(local_path, str) and local_path:
        parsed_source = LocalSource(local_path)
    else:
        raise ValueError(f"Resource rule {name} has an invalid {kind} source")
    # No manifest copies directories; only build-server-tools implements it.
    if raw.get("recursive") is True:
        raise ValueError(f"Resource rule {name} uses unsupported recursive copy")
    destination = raw.get("destination")
    if not isinstance(destination, str) or not destination:
        raise ValueError(f"Resource rule {name} is missing destination")
    path = PurePosixPath(destination)
    if (
        "\\" in destination
        or path.is_absolute()
        or ".." in path.parts
        or path == PurePosixPath(".")
    ):
        raise ValueError(f"Resource rule {name} destination is unsafe: {destination}")
    return ResourceRule(
        name=name,
        source=parsed_source,
        destination=path.as_posix(),
        os=cast(
            Optional[tuple[TargetOs, ...]],
            _string_list(raw, "os", _TARGET_OS_VALUES, name),
        ),
        arch=cast(
            Optional[tuple[TargetArch, ...]],
            _string_list(raw, "arch", _TARGET_ARCH_VALUES, name),
        ),
        executable=raw.get("executable") is True,
    )


def load_resource_manifest(path: Path) -> tuple[ResourceRule, ...]:
    """Parse a server resource manifest, rejecting what the packager rejects."""
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"Server resource manifest is unreadable: {path}") from exc
    if not isinstance(document, dict) or not isinstance(
        document.get("resources"), list
    ):
        raise ValueError(f"Server resource manifest is missing resources: {path}")
    return tuple(_parse_rule(entry) for entry in document["resources"])


def target_resource_rules(
    product: str, target: ServerTarget, agent_root: Path
) -> tuple[ResourceRule, ...]:
    """Return the product's manifest rules that apply to one target, in order."""
    bundle = _server_bundle(product)
    rules = load_resource_manifest(agent_root / bundle.resource_manifest)
    return tuple(rule for rule in rules if rule.applies_to(target))


def server_build_command(
    product: str,
    target_id: str,
    agent_root: Path,
    cargo_target_dir: Path,
) -> tuple[tuple[str, ...], Path]:
    """Return the Cargo command for one target and the binary it produces."""
    try:
        target = TARGETS[target_id]
    except KeyError as exc:
        raise ValueError(f"Unknown server target {target_id}") from exc
    bundle = _server_bundle(product)
    # The package name comes from the component's manifest in this checkout,
    # so a crate rename only touches the component registry.
    package = component_for_source(
        agent_root.parent.parent, bundle.source_component
    ).package_name
    suffix = ".exe" if target.os == "windows" else ""
    binary = cargo_target_dir / target.cargo_triple / "release" / (
        f"{bundle.cargo_binary_name}{suffix}"
    )
    return (
        (
            "cargo",
            "build",
            "--release",
            "--locked",
            "-p",
            package,
            "--bin",
            bundle.cargo_binary_name,
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
    agent_root: Path,
) -> ServerResourceResult:
    """Validate a staged server resource directory and its provenance.

    Every file the product's resource manifest places for this target must be
    declared, so a folder staged without (say) the vendored Bun fails here.
    """
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
    for rule in target_resource_rules(product, target_spec, agent_root):
        if rule.destination not in declared:
            raise ValueError(
                f"Server artifact is missing {rule.name}: {rule.destination}"
            )
    # Chromium launches only runtime_binary_name; a file still carrying the
    # Cargo binary name means staging skipped the rename.
    if bundle.cargo_binary_name != bundle.runtime_binary_name and any(
        bundle.cargo_binary_name in path for path in declared
    ):
        raise ValueError(
            f"Server artifact staged the Cargo binary name {bundle.cargo_binary_name}"
        )
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
        download_r2: DownloadR2 = download_from_r2,
        r2_configured: R2Configured = _default_r2_configured,
        cargo_target_dir: Optional[Path] = None,
    ) -> None:
        self.repo_root = repo_root.resolve()
        self.agent_root = self.repo_root / "packages/browseros-agent"
        self.browseros_root = self.repo_root / "packages/browseros"
        self.host_platform = host_platform or _host_platform()
        self.run = run
        self.which = which
        self.rust_targets = rust_targets
        self.download_r2 = download_r2
        self.r2_configured = r2_configured
        self.cargo_target_dir = (
            cargo_target_dir or self.agent_root / "target"
        ).resolve()

    def _preflight(
        self, product: str, target: ServerTarget
    ) -> tuple[ServerBundle, tuple[ResourceRule, ...]]:
        bundle = _server_bundle(product)
        if self.host_platform != target.os:
            raise RuntimeError(
                f"{bundle.name} target {target.id} requires host {target.os}; "
                f"current host is {self.host_platform}"
            )
        if self.which("cargo") is None:
            raise RuntimeError(f"Cannot build {target.id}: install Rust and Cargo")
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
        rules = target_resource_rules(product, target, self.agent_root)
        remote = [rule.name for rule in rules if isinstance(rule.source, R2Source)]
        if remote and not self.r2_configured():
            raise RuntimeError(
                f"Cannot build {target.id}: R2 access is needed for {', '.join(remote)}; "
                "set R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, and R2_SECRET_ACCESS_KEY"
            )
        return bundle, rules

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
        bundle, rules = self._preflight(product, target_spec)
        source = source_resources_for_product(product)
        normalized_version = normalize_component_version(
            source.server_component, version
        )
        source_version = read_component_version(
            self.repo_root, source.server_component
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
        # The inherited environment carries compile-time values such as the
        # optional BROWSEROS_SENTRY_DSN, which the server embeds at build time.
        environment = {
            **os.environ,
            **bundle.cargo_env,
            "CARGO_TARGET_DIR": str(self.cargo_target_dir),
        }
        self.run(command, self.agent_root, environment)
        self._stage(
            artifact,
            destination,
            bundle,
            rules,
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
            agent_root=self.agent_root,
        )

    def _stage(
        self,
        artifact: Path,
        destination: Path,
        bundle: ServerBundle,
        rules: tuple[ResourceRule, ...],
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
        files = [runtime]
        for rule in rules:
            path = destination.joinpath(*PurePosixPath(rule.destination).parts)
            path.parent.mkdir(parents=True, exist_ok=True)
            if isinstance(rule.source, R2Source):
                key = f"{VENDOR_R2_PREFIX}/{rule.source.key}"
                if not self.download_r2(key, path):
                    raise RuntimeError(f"Could not download {rule.name} from R2: {key}")
            else:
                source = self.agent_root / rule.source.path
                if not source.is_file():
                    raise FileNotFoundError(f"{rule.name} is missing: {source}")
                shutil.copy2(source, path)
            if rule.executable and target.os != "windows":
                path.chmod(0o755)
            files.append(path)
        # File order feeds manifest_sha256, which the lane manifest records as
        # server evidence: the binary comes first, then rules in manifest order.
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
