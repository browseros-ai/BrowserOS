#!/usr/bin/env python3
"""Resolve immutable resource versions for a browser release."""

import re
import time
from dataclasses import asdict, dataclass
from typing import Callable, Iterable


_VERSION_RE = re.compile(r"[0-9]+(?:\.[0-9]+){2}")
_SHA_RE = re.compile(r"[0-9a-f]{40}")
_SHA256_RE = re.compile(r"[0-9a-f]{64}")
_BINDING_FIELDS = frozenset({"component", "release-sha", "sha256", "target", "version"})


@dataclass(frozen=True)
class ResourceFamily:
    """R2 layout for one browser resource family."""

    name: str
    component: str
    archive_base: str
    targets: tuple[str, ...]

    def filename(self, target: str) -> str:
        suffix = "" if target == "universal" else f"-{target}"
        return f"{self.archive_base}{suffix}.zip"

    def latest_key(self, target: str) -> str:
        return f"{self.component}/latest/{self.filename(target)}"

    def version_key(self, version: str, target: str) -> str:
        return f"{self.component}/{version}/{self.filename(target)}"


@dataclass(frozen=True)
class ResourceObjectPin:
    """Identity of one immutable platform object."""

    target: str
    key: str
    etag: str
    size: int
    sha256: str
    release_sha: str


@dataclass(frozen=True)
class ResourcePin:
    """One coherent version spanning every target in a resource family."""

    name: str
    version: str
    objects: tuple[ResourceObjectPin, ...]

    def to_dict(self) -> dict:
        return {
            "version": self.version,
            "objects": [asdict(item) for item in self.objects],
        }


RESOURCE_FAMILIES = (
    ResourceFamily(
        name="browseros_server",
        component="artifacts/server",
        archive_base="browseros-server-resources",
        targets=(
            "darwin-arm64",
            "darwin-x64",
            "linux-arm64",
            "linux-x64",
            "windows-x64",
        ),
    ),
    ResourceFamily(
        name="browserclaw_server",
        component="claw-server-rust/prod-resources",
        archive_base="browseros-claw-server-rust-resources",
        targets=(
            "darwin-arm64",
            "darwin-x64",
            "linux-arm64",
            "linux-x64",
            "windows-x64",
        ),
    ),
    ResourceFamily(
        name="browserclaw_onboard",
        component="claw-onboard/prod-resources",
        archive_base="browseros-claw-onboard-resources",
        targets=("universal",),
    ),
    ResourceFamily(
        name="browseros_onboard",
        component="app-onboard/prod-resources",
        archive_base="browseros-app-onboard-resources",
        targets=("universal",),
    ),
)

COMPONENT_RESOURCE_FAMILY = {
    "browseros-server": "browseros_server",
    "browserclaw-server": "browserclaw_server",
    "browserclaw-onboard": "browserclaw_onboard",
    "browseros-onboard": "browseros_onboard",
}


@dataclass(frozen=True)
class _Snapshot:
    etag: str
    size: int
    metadata: tuple[tuple[str, str], ...]

    def metadata_dict(self) -> dict[str, str]:
        return dict(self.metadata)


class _IncoherentSnapshot(RuntimeError):
    pass


def _head(client, bucket: str, key: str) -> _Snapshot:
    response = client.head_object(Bucket=bucket, Key=key)
    metadata = tuple(
        sorted(
            (str(name).lower(), str(value))
            for name, value in response.get("Metadata", {}).items()
        )
    )
    return _Snapshot(
        etag=str(response["ETag"]),
        size=int(response["ContentLength"]),
        metadata=metadata,
    )


def _validate_version(version: str, family: ResourceFamily) -> None:
    if not _VERSION_RE.fullmatch(version):
        raise _IncoherentSnapshot(
            f"{family.name} has invalid resource version {version!r}"
        )


def _binding(
    snapshot: _Snapshot,
    family: ResourceFamily,
    target: str,
    *,
    version: str | None = None,
    release_sha: str | None = None,
) -> dict[str, str]:
    metadata = snapshot.metadata_dict()
    if not metadata:
        raise _IncoherentSnapshot(f"{family.name}/{target} has no object binding")
    missing = sorted(_BINDING_FIELDS.difference(metadata))
    if missing:
        raise _IncoherentSnapshot(
            f"{family.name}/{target} binding is missing: {', '.join(missing)}"
        )
    if metadata["component"] != family.component:
        raise _IncoherentSnapshot(f"{family.name}/{target} component binding mismatch")
    if metadata["target"] != target:
        raise _IncoherentSnapshot(f"{family.name}/{target} target binding mismatch")
    _validate_version(metadata["version"], family)
    if not _SHA_RE.fullmatch(metadata["release-sha"]):
        raise _IncoherentSnapshot(f"{family.name}/{target} has invalid release SHA")
    if not _SHA256_RE.fullmatch(metadata["sha256"]):
        raise _IncoherentSnapshot(f"{family.name}/{target} has invalid sha256 binding")
    if version is not None and metadata["version"] != version:
        raise _IncoherentSnapshot(f"{family.name}/{target} version binding mismatch")
    if release_sha is not None and metadata["release-sha"] != release_sha:
        raise _IncoherentSnapshot(f"{family.name}/{target} source binding mismatch")
    return metadata


def _object_pin(
    family: ResourceFamily,
    target: str,
    version: str,
    snapshot: _Snapshot,
) -> ResourceObjectPin:
    metadata = snapshot.metadata_dict()
    return ResourceObjectPin(
        target=target,
        key=family.version_key(version, target),
        etag=snapshot.etag,
        size=snapshot.size,
        sha256=metadata.get("sha256", ""),
        release_sha=metadata.get("release-sha", ""),
    )


def _verify_prepared(
    client,
    bucket: str,
    family: ResourceFamily,
    version: str,
    release_sha: str = "",
) -> ResourcePin:
    _validate_version(version, family)
    if release_sha and not _SHA_RE.fullmatch(release_sha):
        raise RuntimeError(f"{family.name} prepared source SHA is invalid")
    snapshots = {
        target: _head(client, bucket, family.version_key(version, target))
        for target in family.targets
    }
    if release_sha:
        for target, snapshot in snapshots.items():
            _binding(
                snapshot,
                family,
                target,
                version=version,
                release_sha=release_sha,
            )
    else:
        metadata_states = {bool(snapshot.metadata) for snapshot in snapshots.values()}
        if metadata_states == {True}:
            bindings = {
                target: _binding(snapshot, family, target, version=version)
                for target, snapshot in snapshots.items()
            }
            release_shas = {binding["release-sha"] for binding in bindings.values()}
            if len(release_shas) != 1:
                raise _IncoherentSnapshot(
                    f"{family.name} prepared objects are source-skewed"
                )
        elif metadata_states != {False}:
            raise _IncoherentSnapshot(
                f"{family.name} prepared objects have mixed bindings"
            )
    objects = [
        _object_pin(family, target, version, snapshots[target])
        for target in family.targets
    ]
    return ResourcePin(family.name, version, tuple(objects))


def verify_prepared_resource_pin(
    client,
    bucket: str,
    family_name: str,
    version: str,
    release_sha: str = "",
) -> ResourcePin:
    """Verify one exact prepared resource family."""
    family = next(
        (item for item in RESOURCE_FAMILIES if item.name == family_name),
        None,
    )
    if family is None:
        raise ValueError(f"unknown prepared resource family: {family_name}")
    try:
        return _verify_prepared(client, bucket, family, version, release_sha)
    except _IncoherentSnapshot as error:
        raise RuntimeError(
            f"Could not verify prepared {family_name} {version}: {error}"
        ) from error


def _list_objects(client, bucket: str, prefix: str) -> Iterable[dict]:
    token = None
    while True:
        request = {"Bucket": bucket, "Prefix": prefix}
        if token:
            request["ContinuationToken"] = token
        response = client.list_objects_v2(**request)
        yield from response.get("Contents", [])
        if not response.get("IsTruncated"):
            return
        token = response.get("NextContinuationToken")
        if not token:
            raise RuntimeError(
                f"R2 listing for {prefix} omitted its continuation token"
            )


def _legacy_version(
    client,
    bucket: str,
    family: ResourceFamily,
    latest: dict[str, _Snapshot],
) -> tuple[str, dict[str, _Snapshot]]:
    by_version: dict[str, dict[str, tuple[str, int]]] = {}
    prefix = f"{family.component}/"
    expected_names = {family.filename(target): target for target in family.targets}
    for item in _list_objects(client, bucket, prefix):
        key = str(item.get("Key", ""))
        relative = key.removeprefix(prefix)
        parts = relative.split("/")
        if len(parts) != 2 or parts[0] == "latest":
            continue
        version, filename = parts
        target = expected_names.get(filename)
        if target is None or not _VERSION_RE.fullmatch(version):
            continue
        by_version.setdefault(version, {})[target] = (
            str(item["ETag"]),
            int(item["Size"]),
        )

    candidates = [
        version
        for version, objects in by_version.items()
        if all(
            objects.get(target) == (latest[target].etag, latest[target].size)
            for target in family.targets
        )
    ]
    if len(candidates) != 1:
        raise _IncoherentSnapshot(
            f"{family.name} legacy latest aliases match {len(candidates)} versions"
        )

    version = candidates[0]
    exact = {
        target: _head(client, bucket, family.version_key(version, target))
        for target in family.targets
    }
    for target in family.targets:
        if exact[target] != latest[target]:
            raise _IncoherentSnapshot(
                f"{family.name}/{target} legacy alias does not match its exact object"
            )
    return version, exact


def _metadata_version(
    client,
    bucket: str,
    family: ResourceFamily,
    latest: dict[str, _Snapshot],
) -> tuple[str, dict[str, _Snapshot]]:
    bindings = {
        target: _binding(snapshot, family, target)
        for target, snapshot in latest.items()
    }
    versions = {binding["version"] for binding in bindings.values()}
    release_shas = {binding["release-sha"] for binding in bindings.values()}
    if len(versions) != 1 or len(release_shas) != 1:
        raise _IncoherentSnapshot(f"{family.name} latest aliases are version-skewed")
    version = versions.pop()
    release_sha = release_shas.pop()
    exact = {}
    for target in family.targets:
        snapshot = _head(client, bucket, family.version_key(version, target))
        _binding(
            snapshot,
            family,
            target,
            version=version,
            release_sha=release_sha,
        )
        if snapshot != latest[target]:
            raise _IncoherentSnapshot(
                f"{family.name}/{target} latest alias differs from its exact object"
            )
        exact[target] = snapshot
    return version, exact


def _resolve_latest_once(client, bucket: str, family: ResourceFamily) -> ResourcePin:
    latest = {
        target: _head(client, bucket, family.latest_key(target))
        for target in family.targets
    }
    metadata_states = {bool(snapshot.metadata) for snapshot in latest.values()}
    if metadata_states == {True}:
        version, exact = _metadata_version(client, bucket, family, latest)
    elif metadata_states == {False}:
        version, exact = _legacy_version(client, bucket, family, latest)
    else:
        raise _IncoherentSnapshot(f"{family.name} latest aliases have mixed bindings")

    second_read = {
        target: _head(client, bucket, family.latest_key(target))
        for target in family.targets
    }
    if second_read != latest:
        raise _IncoherentSnapshot(
            f"{family.name} latest aliases changed during snapshot"
        )
    return ResourcePin(
        family.name,
        version,
        tuple(
            _object_pin(family, target, version, exact[target])
            for target in family.targets
        ),
    )


def resolve_resource_pins(
    client,
    bucket: str,
    prepared: dict[str, tuple[str, str]],
    *,
    attempts: int = 5,
    sleep: Callable[[float], None] = time.sleep,
) -> dict[str, ResourcePin]:
    """Resolve prepared versions and coherent snapshots of every other family."""
    known = {family.name for family in RESOURCE_FAMILIES}
    unknown = sorted(set(prepared).difference(known))
    if unknown:
        raise ValueError(f"unknown prepared resource families: {', '.join(unknown)}")
    if attempts < 1:
        raise ValueError("attempts must be positive")

    pins = {}
    for family in RESOURCE_FAMILIES:
        override = prepared.get(family.name)
        if override is not None:
            pins[family.name] = _verify_prepared(
                client, bucket, family, override[0], override[1]
            )
            continue

        last_error = None
        for attempt in range(attempts):
            try:
                pins[family.name] = _resolve_latest_once(client, bucket, family)
                break
            except _IncoherentSnapshot as error:
                last_error = error
                if attempt + 1 < attempts:
                    sleep(2**attempt)
        else:
            raise RuntimeError(
                f"Could not resolve a coherent {family.name} latest snapshot: "
                f"{last_error}"
            ) from last_error
    return pins
