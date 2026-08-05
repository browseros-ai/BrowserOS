#!/usr/bin/env python3
"""Build, stamp, pack, and upload extension CRXs."""

import hashlib
import json
import os
import re
from pathlib import Path
from typing import List, Mapping, Optional, Sequence, Tuple

from ...core.step import Step, ValidationError
from ...lib.paths import get_package_root
from ...lib.r2 import BOTO3_AVAILABLE, get_r2_client
from ...lib.utils import log_info, log_success, log_warning
from ..feeds.spec import CDN_BASE_URL, EXTENSIONS as FEED_EXTENSIONS
from ..feeds.render import extract_manifest_versions
from .crx import find_chrome_binary, pack_crx
from .specs import (
    ExtensionSpec,
    ExternalRepoSource,
    InRepoSource,
    select_specs,
    spec_by_name,
)
from .workspace import (
    require_env,
    resolve_source,
    run_command,
    update_manifest_version,
    write_env_file,
)

_VERSION_RE = re.compile(r"^\d+(\.\d+){0,3}$")
_UPDATE_FEED_EXTENSION_NAMES = frozenset(
    extension.name for extension in FEED_EXTENSIONS if extension.in_update_feed
)
_UPDATE_MANIFEST_URL = f"{CDN_BASE_URL}/extensions/update-manifest.xml"
_AUTO_VERSION_EXTENSION_NAMES = frozenset({"agent", "browserclaw"})
_SOURCE_SHA_RE = re.compile(r"^[0-9a-fA-F]{40}$")
_CRX_BINDING_SCHEMA = "browseros-extension-crx-v1"


def _error_response(error: Exception) -> Tuple[str, Optional[int]]:
    response = getattr(error, "response", {})
    code = str(response.get("Error", {}).get("Code", ""))
    status = response.get("ResponseMetadata", {}).get("HTTPStatusCode")
    return code, status


def _is_missing_object(error: Exception) -> bool:
    code, status = _error_response(error)
    return code in {"404", "NoSuchKey", "NotFound"} or status == 404


def _is_precondition_failure(error: Exception) -> bool:
    code, status = _error_response(error)
    return code in {
        "409",
        "412",
        "ConditionalRequestConflict",
        "PreconditionFailed",
    } or status in {409, 412}


def _crx_binding(
    spec: ExtensionSpec,
    version: str,
    source_sha: str,
    data: bytes,
) -> Mapping[str, str]:
    return {
        "binding-schema": _CRX_BINDING_SCHEMA,
        "extension": spec.name,
        "version": version,
        "source-sha": source_sha,
        "sha256": hashlib.sha256(data).hexdigest(),
    }


def _read_bound_extension_crx(
    client,
    bucket: str,
    spec: ExtensionSpec,
    version: str,
    source_sha: str,
) -> Optional[bytes]:
    key = spec.crx_key(version)
    try:
        response = client.get_object(Bucket=bucket, Key=key)
    except Exception as e:
        if _is_missing_object(e):
            return None
        raise RuntimeError(f"Failed to read canonical extension CRX {key}: {e}") from e

    body = response.get("Body")
    if body is None or not hasattr(body, "read"):
        raise RuntimeError(f"Canonical extension CRX {key} returned no readable body")
    try:
        data = body.read()
    finally:
        close = getattr(body, "close", None)
        if close:
            close()

    metadata = response.get("Metadata")
    if not isinstance(metadata, Mapping):
        metadata = {}
    expected = {
        "binding-schema": _CRX_BINDING_SCHEMA,
        "extension": spec.name,
        "version": version,
        "source-sha": source_sha,
    }
    mismatches = {
        name: {"expected": value, "actual": metadata.get(name)}
        for name, value in expected.items()
        if metadata.get(name) != value
    }
    actual_sha256 = hashlib.sha256(data).hexdigest()
    if metadata.get("sha256") != actual_sha256:
        mismatches["sha256"] = {
            "expected": actual_sha256,
            "actual": metadata.get("sha256"),
        }
    if mismatches:
        raise RuntimeError(
            f"Canonical extension CRX binding mismatch for {key}: {mismatches}"
        )
    return data


def materialize_bound_extension_crx(
    client,
    bucket: str,
    spec: ExtensionSpec,
    version: str,
    source_sha: str,
    destination: Path,
) -> bool:
    """Materialize an existing source-bound CRX for retry attachment."""
    data = _read_bound_extension_crx(client, bucket, spec, version, source_sha)
    if data is None:
        return False
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(data)
    log_success(f"Reused canonical extension CRX: {spec.crx_key(version)}")
    return True


def upload_bound_extension_crx(
    client,
    bucket: str,
    spec: ExtensionSpec,
    version: str,
    source_sha: str,
    path: Path,
) -> str:
    """Upload a source-bound CRX or consume the concurrent canonical object."""
    key = spec.crx_key(version)
    data = path.read_bytes()
    try:
        client.put_object(
            Bucket=bucket,
            Key=key,
            Body=data,
            ContentType="application/x-chrome-extension",
            Metadata=dict(_crx_binding(spec, version, source_sha, data)),
            IfNoneMatch="*",
        )
    except Exception as e:
        if not _is_precondition_failure(e):
            raise RuntimeError(
                f"Failed to create source-bound extension CRX {key}: {e}"
            ) from e
        if not materialize_bound_extension_crx(
            client,
            bucket,
            spec,
            version,
            source_sha,
            path,
        ):
            raise RuntimeError(f"Concurrent canonical extension CRX disappeared: {key}")
        return "reused"
    log_success(f"Uploaded source-bound extension CRX: {key}")
    return "uploaded"


def normalize_extension_version(version: str) -> str:
    """Normalize a Chrome extension version to four integer components."""
    if not _VERSION_RE.fullmatch(version):
        raise ValueError(
            f"Invalid version '{version}' — expected 1-4 dot-separated integers"
        )
    parts = [int(part) for part in version.split(".")]
    parts.extend([0] * (4 - len(parts)))
    if any(part > 65535 for part in parts):
        raise ValueError(
            f"Invalid version '{version}' — every component must be at most 65535"
        )
    return ".".join(str(part) for part in parts)


def _version_parts(version: str) -> Tuple[int, int, int, int]:
    normalized = normalize_extension_version(version)
    major, minor, release, build = normalized.split(".")
    return int(major), int(minor), int(release), int(build)


def increment_extension_version(version: str) -> str:
    """Increment the third component and reset the fourth component."""
    major, minor, release, _ = _version_parts(version)
    if release == 65535:
        raise ValueError(f"Cannot increment extension version '{version}'")
    return f"{major}.{minor}.{release + 1}.0"


def extension_names(selection: str) -> Tuple[str, ...]:
    """Resolve one extension selection or the legacy all selection."""
    if selection == "all":
        return tuple(spec.name for spec in select_specs(None))
    return (spec_by_name(selection).name,)


def resolve_extension_version(
    *,
    extension: str,
    requested_version: str,
    release_sha: str,
    release_records: Sequence[Mapping[str, object]],
    manifest_contents: Sequence[str],
) -> str:
    """Resolve an explicit or automatically allocated extension version."""
    names = extension_names(extension)
    if requested_version:
        return normalize_extension_version(requested_version)
    if len(names) != 1 or names[0] not in _AUTO_VERSION_EXTENSION_NAMES:
        raise ValueError(
            f"Extension '{extension}' requires an explicit version; automatic "
            "allocation is limited to agent and browserclaw"
        )
    if not release_sha:
        raise ValueError("Automatic extension version allocation requires a source SHA")

    name = names[0]
    spec = spec_by_name(name)
    tag_pattern = re.compile(rf"^ext-{re.escape(name)}/v(.+)$")
    allocated: List[Tuple[int, int, int, int]] = []
    same_source: List[Tuple[int, int, int, int]] = []

    for record in release_records:
        tag_name = record.get("tag_name")
        if not isinstance(tag_name, str):
            continue
        match = tag_pattern.fullmatch(tag_name)
        if not match:
            continue
        try:
            version = _version_parts(match.group(1))
        except ValueError:
            continue
        allocated.append(version)
        record_sha = (
            record.get("tag_release_sha")
            or record.get("release_sha")
            or record.get("target_commitish")
            or record.get("targetCommitish")
        )
        reusable = record.get("tag_object") == "tag" or (
            record.get("draft") is True and record.get("tag_object") == "missing"
        )
        if record_sha == release_sha and reusable:
            same_source.append(version)

    if same_source:
        return ".".join(str(part) for part in max(same_source))

    for content in manifest_contents:
        versions = extract_manifest_versions(content)
        version = versions.get(spec.extension_id)
        if version:
            allocated.append(_version_parts(version))

    maximum = max(allocated, default=(0, 0, 0, 0))
    return increment_extension_version(".".join(str(part) for part in maximum))


def verify_versioned_crx_objects(
    client,
    bucket: str,
    version: str,
    names: Sequence[str],
    source_sha: str,
    output_dir: Optional[Path] = None,
) -> Tuple[str, ...]:
    """Require every selected CRX to have the expected source binding."""
    if not _SOURCE_SHA_RE.fullmatch(source_sha):
        raise ValueError("Extension verification requires a full source commit SHA")
    keys = []
    for name in names:
        spec = spec_by_name(name)
        key = spec.crx_key(version)
        data = _read_bound_extension_crx(
            client,
            bucket,
            spec,
            version,
            source_sha,
        )
        if data is None:
            raise RuntimeError(f"Prepared extension CRX is missing: {key}")
        if output_dir is not None:
            output_dir.mkdir(parents=True, exist_ok=True)
            (output_dir / spec.crx_filename(version)).write_bytes(data)
        keys.append(key)
    return tuple(keys)


def _validate_manifest_update_url(
    spec: ExtensionSpec, manifest: Mapping[str, object], dist_path: Path
) -> None:
    """Require update-feed extensions to declare the stable updater manifest."""
    if spec.name not in _UPDATE_FEED_EXTENSION_NAMES:
        return
    update_url = manifest.get("update_url")
    if update_url != _UPDATE_MANIFEST_URL:
        raise RuntimeError(
            f"Extension '{spec.name}' build at '{dist_path}' has update_url "
            f"{update_url!r}; expected '{_UPDATE_MANIFEST_URL}'"
        )


class ExtensionReleaseModule(Step):
    """Build, version-stamp, pack and upload the selected extension CRXs."""

    produces = []
    requires = []
    description = "Build and upload extension CRXs to the CDN"

    def __init__(
        self,
        version: str,
        names: Tuple[str, ...],
        source_sha: str,
        branch_override: Optional[str] = None,
        chrome_binary: Optional[str] = None,
        monorepo_root: Optional[Path] = None,
        work_root: Optional[Path] = None,
        r2_client=None,
    ):
        self.version = version
        self.names = names
        self.source_sha = source_sha
        self.branch_override = branch_override
        self.chrome_binary = chrome_binary
        self._monorepo_root = monorepo_root
        self._work_root = work_root
        self._r2_client = r2_client

    def _specs(self) -> List[ExtensionSpec]:
        return [spec_by_name(name) for name in self.names]

    def validate(self, ctx) -> None:
        if not _SOURCE_SHA_RE.fullmatch(self.source_sha):
            raise ValidationError("Extension releases require a full source commit SHA")
        try:
            specs = self._specs()
        except ValueError as e:
            raise ValidationError(str(e))

        for spec in specs:
            try:
                require_env(spec.signing_key_env)
            except EnvironmentError:
                raise ValidationError(
                    f"Signing key env var '{spec.signing_key_env}' for "
                    f"'{spec.name}' is missing or empty"
                )
            for name in spec.required_env:
                try:
                    require_env(name)
                except EnvironmentError:
                    raise ValidationError(
                        f"Required build env var '{name}' for "
                        f"'{spec.name}' is missing or empty"
                    )

        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )
        if not ctx.env.has_r2_config():
            raise ValidationError("R2 configuration not set")

        # Public repos clone fine without it — warn, don't fail.
        if not os.environ.get("GH_TOKEN") and any(
            isinstance(spec.source, ExternalRepoSource) for spec in specs
        ):
            log_warning(
                "GH_TOKEN is not set — cloning external extension repos "
                "will fail if any of them is private"
            )

        try:
            find_chrome_binary(self.chrome_binary)
        except RuntimeError as e:
            raise ValidationError(str(e))

    def execute(self, ctx) -> None:
        package_root = get_package_root()
        monorepo_root = self._monorepo_root or package_root.parent.parent
        work_root = self._work_root or package_root / "build" / "extensions"

        client = self._r2_client
        if client is None:
            client = get_r2_client(ctx.env)
            if client is None:
                raise RuntimeError("Failed to create R2 client")
        chrome = find_chrome_binary(self.chrome_binary)

        for spec in self._specs():
            log_info(f"\n=== Releasing extension: {spec.name} v{self.version} ===")
            r2_key = spec.crx_key(self.version)
            crx_path = work_root / "dist" / spec.crx_filename(self.version)
            if materialize_bound_extension_crx(
                client,
                ctx.env.r2_bucket,
                spec,
                self.version,
                self.source_sha,
                crx_path,
            ):
                log_success(f"CRX live at {CDN_BASE_URL}/{r2_key}")
                continue
            source_root = resolve_source(
                spec,
                monorepo_root=monorepo_root,
                work_root=work_root,
                branch_override=self.branch_override,
            )
            update_manifest_version(source_root / spec.manifest_path, self.version)
            if isinstance(spec.source, InRepoSource):
                touched = spec.manifest_path
                if spec.env:
                    touched += f" and {spec.env_dir or '.'}/.env"
                log_warning(
                    f"stamped {touched} in the working tree — "
                    "revert them if this was a local test run"
                )
            if spec.env:
                env_dir = source_root / spec.env_dir if spec.env_dir else source_root
                write_env_file(
                    env_dir,
                    spec.env,
                    required_names=spec.required_env,
                )
            if spec.pre_build:
                run_command(spec.pre_build, source_root)
            run_command(spec.build, source_root)

            dist_path = source_root / spec.dist_path
            manifest = json.loads(
                (dist_path / "manifest.json").read_text(encoding="utf-8")
            )
            _validate_manifest_update_url(spec, manifest, dist_path)
            crx_path = pack_crx(
                dist_path,
                require_env(spec.signing_key_env),
                chrome,
                crx_path,
            )

            upload_bound_extension_crx(
                client,
                ctx.env.r2_bucket,
                spec,
                self.version,
                self.source_sha,
                crx_path,
            )
            log_success(f"CRX live at {CDN_BASE_URL}/{r2_key}")


def build_pipeline(
    version: str,
    name: Optional[str],
    branch: Optional[str],
    chrome_binary: Optional[str],
    source_sha: str,
) -> List[Step]:
    """Assemble the CRX release pipeline for one --name (or every spec)."""
    if not _VERSION_RE.fullmatch(version):
        raise ValueError(
            f"Invalid version '{version}' — Chrome extension versions are "
            "1-4 dot-separated integers (e.g. 0.0.118)"
        )
    # A '-'-prefixed value would be parsed as a git option, not a branch.
    if branch and branch.startswith("-"):
        raise ValueError(f"Invalid branch name '{branch}'")
    if not _SOURCE_SHA_RE.fullmatch(source_sha):
        raise ValueError("Extension releases require a full source commit SHA")
    specs = select_specs(name)
    return [
        ExtensionReleaseModule(
            version=version,
            names=tuple(spec.name for spec in specs),
            source_sha=source_sha,
            branch_override=branch,
            chrome_binary=chrome_binary,
        )
    ]
