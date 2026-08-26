#!/usr/bin/env python3
"""Portable prepared common-resource contract."""

import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET
import zipfile
from dataclasses import asdict, dataclass
from pathlib import Path, PurePosixPath
from typing import Mapping, Protocol, cast

import requests

from ..core.products import BROWSEROS_BUG_REPORTER_EXTENSION_ID
from ..products.resource_sources import source_resources_for_product
from ..steps.storage.download import extract_artifact_zip
from .extensions.build import build_extension_crx
from .extensions.crx import find_chrome_binary, read_crx_extension_id
from .extensions.specs import spec_by_name


PREPARED_RESOURCES_NAME = "prepared-resources.json"
PREPARED_RESOURCES_SCHEMA = "browseros-prepared-resources-v1"
DEFAULT_BUNDLED_MANIFEST_URL = (
    "https://cdn.browseros.com/extensions/bundled-manifest.xml"
)


@dataclass(frozen=True)
class PreparedFile:
    """One checksummed file in a prepared resource directory."""

    path: str
    size: int
    sha256: str
    version: str
    extension_id: str = ""


@dataclass(frozen=True)
class PreparedResourcesManifest:
    """Identity and files shared by every browser lane."""

    product: str
    parent_sha: str
    source_sha: str
    browser_version: str
    component_versions: Mapping[str, str]
    files: Mapping[str, PreparedFile]
    schema: str = PREPARED_RESOURCES_SCHEMA

    def to_dict(self) -> dict[str, object]:
        return {
            "schema": self.schema,
            "product": self.product,
            "parent_sha": self.parent_sha,
            "source_sha": self.source_sha,
            "browser_version": self.browser_version,
            "component_versions": dict(self.component_versions),
            "files": {
                role: asdict(prepared_file)
                for role, prepared_file in self.files.items()
            },
        }

    def to_json(self) -> str:
        return json.dumps(self.to_dict(), indent=2, sort_keys=True) + "\n"

    def digest(self) -> str:
        payload = json.dumps(
            self.to_dict(), sort_keys=True, separators=(",", ":")
        ).encode()
        return hashlib.sha256(payload).hexdigest()

    @classmethod
    def from_dict(cls, document: Mapping[str, object]) -> "PreparedResourcesManifest":
        if document.get("schema") != PREPARED_RESOURCES_SCHEMA:
            raise ValueError("Unsupported prepared-resource schema")
        fields = {
            name: document.get(name)
            for name in (
                "product",
                "parent_sha",
                "source_sha",
                "browser_version",
            )
        }
        if not all(isinstance(value, str) for value in fields.values()):
            raise ValueError("Prepared-resource identity fields must be strings")
        versions = document.get("component_versions")
        if not isinstance(versions, dict) or not all(
            isinstance(key, str) and isinstance(value, str)
            for key, value in versions.items()
        ):
            raise ValueError("Prepared-resource component_versions must be a string map")
        raw_files = document.get("files")
        if not isinstance(raw_files, dict):
            raise ValueError("Prepared-resource files must be an object")
        files: dict[str, PreparedFile] = {}
        for role, raw_file in raw_files.items():
            if not isinstance(role, str) or not isinstance(raw_file, dict):
                raise ValueError("Prepared-resource file entries must be objects")
            try:
                files[role] = PreparedFile(
                    path=str(raw_file["path"]),
                    size=int(raw_file["size"]),
                    sha256=str(raw_file["sha256"]),
                    version=str(raw_file["version"]),
                    extension_id=str(raw_file.get("extension_id", "")),
                )
            except (KeyError, TypeError, ValueError) as exc:
                raise ValueError(f"Invalid prepared-resource file entry: {role}") from exc
        return cls(
            product=cast(str, fields["product"]),
            parent_sha=cast(str, fields["parent_sha"]),
            source_sha=cast(str, fields["source_sha"]),
            browser_version=cast(str, fields["browser_version"]),
            component_versions=versions,
            files=files,
        )


@dataclass(frozen=True)
class PreparationRequest:
    """Expected identity and destination for common resource production."""

    product: str
    parent_sha: str
    source_sha: str
    browser_version: str
    component_versions: Mapping[str, str]
    output_dir: Path
    manifest_url: str = DEFAULT_BUNDLED_MANIFEST_URL


class PreparationOperations(Protocol):
    def build_product_extension(
        self, request: PreparationRequest, destination: Path
    ) -> None: ...

    def fetch_manifest(self, url: str) -> str: ...

    def download(self, url: str) -> bytes: ...

    def build_onboarding(self, destination: Path, component: str) -> None: ...


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _prepared_file(
    root: Path,
    path: Path,
    version: str,
    extension_id: str = "",
) -> PreparedFile:
    return PreparedFile(
        path=path.relative_to(root).as_posix(),
        size=path.stat().st_size,
        sha256=_sha256(path),
        version=version,
        extension_id=extension_id,
    )


def _resolve_manifest_extension(xml: str, extension_id: str) -> tuple[str, str]:
    try:
        root = ET.fromstring(xml)
    except ET.ParseError as exc:
        raise ValueError("Bundled extension manifest is invalid XML") from exc
    matches = []
    for app in root.iter():
        if app.tag.rsplit("}", 1)[-1] != "app" or app.get("appid") != extension_id:
            continue
        for child in app:
            if child.tag.rsplit("}", 1)[-1] != "updatecheck":
                continue
            version = child.get("version")
            codebase = child.get("codebase")
            if version and codebase:
                matches.append((version, codebase))
    if len(matches) != 1:
        raise ValueError(
            f"Bundled extension manifest must contain one entry for {extension_id}"
        )
    return matches[0]


def _onboarding_archive_name(component: str) -> str:
    """Archive filename published by one onboarding component's build."""
    return f"browseros-{component}-resources.zip"


def _validate_onboarding_archive(path: Path, version: str) -> None:
    try:
        with zipfile.ZipFile(path, "r") as archive:
            metadata = json.loads(archive.read("artifact-metadata.json"))
    except (KeyError, OSError, zipfile.BadZipFile, json.JSONDecodeError) as exc:
        raise ValueError("Onboarding bundle is not a valid resource archive") from exc
    if not isinstance(metadata, dict):
        raise ValueError("Onboarding artifact metadata must be an object")
    if metadata.get("version") != version or metadata.get("target") != "universal":
        raise ValueError("Onboarding artifact identity does not match the checkout")
    with tempfile.TemporaryDirectory(prefix="browseros-onboarding-validate-") as temp_dir:
        try:
            extract_artifact_zip(path, Path(temp_dir))
        except RuntimeError as exc:
            raise ValueError(str(exc)) from exc


def load_prepared_resources(root: Path) -> PreparedResourcesManifest:
    """Load one prepared-resource manifest."""
    path = root / PREPARED_RESOURCES_NAME
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"Prepared-resource manifest is unreadable: {path}") from exc
    if not isinstance(document, dict):
        raise ValueError("Prepared-resource manifest must be a JSON object")
    return PreparedResourcesManifest.from_dict(document)


def _safe_file(root: Path, relative: str) -> Path:
    if "\\" in relative:
        raise ValueError(f"Prepared-resource path is unsafe: {relative}")
    path = PurePosixPath(relative)
    if (
        not relative
        or path.is_absolute()
        or path == PurePosixPath(".")
        or ".." in path.parts
    ):
        raise ValueError(f"Prepared-resource path is unsafe: {relative}")
    target = root.joinpath(*path.parts)
    try:
        target.resolve().relative_to(root.resolve())
    except ValueError as exc:
        raise ValueError(f"Prepared-resource path is unsafe: {relative}") from exc
    if target.is_symlink():
        raise ValueError(f"Prepared-resource path cannot be a symlink: {relative}")
    return target


def validate_prepared_resources(
    root: Path, expected: PreparationRequest
) -> PreparedResourcesManifest:
    """Validate identity, layout, sizes, and checksums before staging."""
    manifest = load_prepared_resources(root)
    expected_identity = {
        "product": expected.product,
        "parent_sha": expected.parent_sha,
        "source_sha": expected.source_sha,
        "browser_version": expected.browser_version,
    }
    for field, value in expected_identity.items():
        if getattr(manifest, field) != value:
            raise ValueError(f"Prepared-resource {field} does not match")
    if dict(manifest.component_versions) != dict(expected.component_versions):
        raise ValueError("Prepared-resource component versions do not match")
    required_roles = {"product_crx", "bug_reporter_crx", "onboarding"}
    if set(manifest.files) != required_roles:
        raise ValueError("Prepared-resource managed file set is incomplete")

    declared_paths: set[str] = set()
    for role, prepared_file in manifest.files.items():
        path = _safe_file(root, prepared_file.path)
        declared_paths.add(prepared_file.path)
        if not path.is_file():
            raise ValueError(f"Prepared-resource file is missing: {prepared_file.path}")
        if path.stat().st_size != prepared_file.size:
            raise ValueError(f"Prepared-resource size mismatch: {prepared_file.path}")
        if _sha256(path) != prepared_file.sha256:
            raise ValueError(f"Prepared-resource checksum mismatch: {prepared_file.path}")
        if role.endswith("crx"):
            try:
                actual_id = read_crx_extension_id(path.read_bytes())
            except ValueError as exc:
                raise ValueError(f"Prepared-resource CRX is invalid: {role}") from exc
            if actual_id != prepared_file.extension_id:
                raise ValueError(f"Prepared-resource extension identity mismatch: {role}")

    source = source_resources_for_product(expected.product)
    product_spec = spec_by_name(source.extension_name)
    product_file = manifest.files["product_crx"]
    if (
        product_file.extension_id != product_spec.extension_id
        or product_file.version
        != expected.component_versions[source.extension_component]
    ):
        raise ValueError("Prepared product extension identity does not match")
    bug_reporter = manifest.files["bug_reporter_crx"]
    if bug_reporter.extension_id != BROWSEROS_BUG_REPORTER_EXTENSION_ID:
        raise ValueError("Prepared bug reporter identity does not match")
    onboarding = manifest.files["onboarding"]
    _validate_onboarding_archive(
        root / onboarding.path,
        expected.component_versions[source.onboarding_component],
    )

    actual_paths = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file() and path.name != PREPARED_RESOURCES_NAME
    }
    if actual_paths != declared_paths:
        extras = ", ".join(sorted(actual_paths - declared_paths))
        missing = ", ".join(sorted(declared_paths - actual_paths))
        raise ValueError(
            f"Prepared-resource directory has unexpected files: "
            f"extra={extras or '-'}, missing={missing or '-'}"
        )
    return manifest


def prepare_common_resources(
    request: PreparationRequest,
    operations: PreparationOperations,
    *,
    rebuild: bool = False,
) -> PreparedResourcesManifest:
    """Build or reuse one strict prepared common-resource directory."""
    if not re.fullmatch(r"[0-9a-fA-F]{40}", request.source_sha):
        raise ValueError("Prepared resources require a full source SHA")
    if request.parent_sha and not re.fullmatch(r"[0-9a-fA-F]{40}", request.parent_sha):
        raise ValueError("Prepared resources require a full parent SHA")
    output = request.output_dir.resolve()
    manifest_path = output / PREPARED_RESOURCES_NAME
    if manifest_path.exists() and not rebuild:
        return validate_prepared_resources(output, request)

    source = source_resources_for_product(request.product)
    product_spec = spec_by_name(source.extension_name)
    product_version = request.component_versions[source.extension_component]
    onboarding_version = request.component_versions[source.onboarding_component]
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(
        tempfile.mkdtemp(prefix=f".{output.name}-", dir=output.parent)
    )
    try:
        extension_dir = staging / "extensions"
        extension_dir.mkdir(parents=True)
        product_crx = extension_dir / f"{product_spec.extension_id}.crx"
        operations.build_product_extension(request, product_crx)
        if read_crx_extension_id(product_crx.read_bytes()) != product_spec.extension_id:
            raise ValueError("Built product CRX identity does not match its product")

        manifest_xml = operations.fetch_manifest(request.manifest_url)
        bug_version, bug_url = _resolve_manifest_extension(
            manifest_xml, BROWSEROS_BUG_REPORTER_EXTENSION_ID
        )
        bug_crx = extension_dir / f"{BROWSEROS_BUG_REPORTER_EXTENSION_ID}.crx"
        bug_crx.write_bytes(operations.download(bug_url))
        if read_crx_extension_id(bug_crx.read_bytes()) != BROWSEROS_BUG_REPORTER_EXTENSION_ID:
            raise ValueError("Downloaded bug reporter CRX identity does not match")

        onboarding_dir = staging / "onboarding"
        onboarding_dir.mkdir()
        onboarding = onboarding_dir / _onboarding_archive_name(
            source.onboarding_component
        )
        operations.build_onboarding(onboarding, source.onboarding_component)
        _validate_onboarding_archive(onboarding, onboarding_version)

        prepared = PreparedResourcesManifest(
            product=request.product,
            parent_sha=request.parent_sha,
            source_sha=request.source_sha,
            browser_version=request.browser_version,
            component_versions=dict(request.component_versions),
            files={
                "product_crx": _prepared_file(
                    staging,
                    product_crx,
                    product_version,
                    product_spec.extension_id,
                ),
                "bug_reporter_crx": _prepared_file(
                    staging,
                    bug_crx,
                    bug_version,
                    BROWSEROS_BUG_REPORTER_EXTENSION_ID,
                ),
                "onboarding": _prepared_file(
                    staging,
                    onboarding,
                    onboarding_version,
                ),
            },
        )
        (staging / PREPARED_RESOURCES_NAME).write_text(
            prepared.to_json(), encoding="utf-8"
        )
        validate_prepared_resources(
            staging,
            PreparationRequest(
                **{**request.__dict__, "output_dir": staging}
            ),
        )
        if output.exists():
            if output.is_dir() and not output.is_symlink():
                shutil.rmtree(output)
            else:
                output.unlink()
        staging.replace(output)
        return prepared
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


class LocalPreparationOperations:
    """Local Bun, Chrome, and HTTP operations for common preparation."""

    def __init__(self, repo_root: Path, chrome_binary: str = "") -> None:
        self.repo_root = repo_root.resolve()
        self.chrome_binary = chrome_binary

    def build_product_extension(
        self, request: PreparationRequest, destination: Path
    ) -> None:
        source = source_resources_for_product(request.product)
        spec = spec_by_name(source.extension_name)
        chrome = find_chrome_binary(self.chrome_binary or None)
        build_extension_crx(
            spec=spec,
            version=request.component_versions[source.extension_component],
            output_path=destination,
            monorepo_root=self.repo_root,
            work_root=self.repo_root / "packages/browseros/build/prepared_extensions",
            chrome_binary=chrome,
            stamp_version=False,
        )

    def fetch_manifest(self, url: str) -> str:
        response = requests.get(url, timeout=30)
        response.raise_for_status()
        return response.text

    def download(self, url: str) -> bytes:
        response = requests.get(url, timeout=60)
        response.raise_for_status()
        return response.content

    def build_onboarding(self, destination: Path, component: str) -> None:
        agent_root = self.repo_root / "packages/browseros-agent"
        manifest = agent_root / f"apps/{component}/package.json"
        lockfile = agent_root / "bun.lock"
        before = (manifest.read_bytes(), lockfile.read_bytes())
        environment = {**os.environ, "NODE_ENV": "production"}
        subprocess.run(
            ["bun", f"scripts/build/{component}.ts", "--no-upload"],
            cwd=agent_root,
            env=environment,
            check=True,
        )
        if before != (manifest.read_bytes(), lockfile.read_bytes()):
            raise RuntimeError("Onboarding build changed its committed version files")
        built = (
            agent_root
            / f"dist/prod/{component}/{_onboarding_archive_name(component)}"
        )
        if not built.is_file():
            raise FileNotFoundError(f"Onboarding build did not produce {built}")
        shutil.copy2(built, destination)
