#!/usr/bin/env python3
"""Probe component versions for immutable R2 objects."""

import re
from dataclasses import dataclass
from typing import Iterable, Mapping

from .components import (
    AllocationRecord,
    component_by_id,
    normalize_component_version,
)
from .resource_pins import RESOURCE_FAMILIES, ResourceFamily


_SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
_RESOURCE_FAMILY_NAMES = {
    "server": "browseros_server",
    "claw-server-rust": "browserclaw_server",
    "claw-onboard": "browserclaw_onboard",
    "app-onboard": "browseros_onboard",
}
_EXTENSION_COMPONENTS = frozenset({"agent", "browserclaw"})


@dataclass(frozen=True)
class _R2Object:
    key: str
    expected_metadata: Mapping[str, str]


def _list_objects(client, bucket: str, prefix: str) -> Iterable[Mapping]:
    token = ""
    while True:
        request = {"Bucket": bucket, "Prefix": prefix}
        if token:
            request["ContinuationToken"] = token
        try:
            response = client.list_objects_v2(**request)
        except Exception as exc:
            raise RuntimeError(
                f"Failed to list immutable R2 objects at {prefix}: {exc}"
            ) from exc
        yield from response.get("Contents", [])
        if not response.get("IsTruncated"):
            return
        token = str(response.get("NextContinuationToken", ""))
        if not token:
            raise RuntimeError(
                f"R2 listing for {prefix} omitted its continuation token"
            )


def _family(component: str) -> ResourceFamily | None:
    family_name = _RESOURCE_FAMILY_NAMES.get(component)
    if family_name is None:
        return None
    return next(family for family in RESOURCE_FAMILIES if family.name == family_name)


def _resource_objects(
    client,
    bucket: str,
    family: ResourceFamily,
    version: str,
) -> list[_R2Object]:
    prefix = f"{family.component}/{version}/"
    targets = {family.version_key(version, target): target for target in family.targets}
    result = []
    for item in _list_objects(client, bucket, prefix):
        key = str(item.get("Key", ""))
        target = targets.get(key)
        if target is None:
            continue
        result.append(
            _R2Object(
                key,
                {
                    "component": family.component,
                    "release-sha": "",
                    "target": target,
                    "version": version,
                },
            )
        )
    return result


def _extension_objects(
    client,
    bucket: str,
    component: str,
    version: str,
) -> list[_R2Object]:
    prefix = f"extensions/{component}-{version}.crx"
    result = []
    for item in _list_objects(client, bucket, prefix):
        key = str(item.get("Key", ""))
        if key != prefix:
            continue
        result.append(
            _R2Object(
                key,
                {
                    "binding-schema": "browseros-extension-crx-v1",
                    "extension": component,
                    "source-sha": "",
                    "version": version,
                },
            )
        )
    return result


def _version_objects(
    client,
    bucket: str,
    component: str,
    version: str,
) -> list[_R2Object]:
    family = _family(component)
    if family is not None:
        return _resource_objects(client, bucket, family, version)
    if component in _EXTENSION_COMPONENTS:
        return _extension_objects(client, bucket, component, version)
    raise ValueError(f"Immutable R2 allocations are not defined for {component}")


def _matches_source_binding(
    client,
    bucket: str,
    obj: _R2Object,
    source_sha: str,
) -> bool:
    try:
        response = client.head_object(Bucket=bucket, Key=obj.key)
    except Exception as exc:
        raise RuntimeError(
            f"Failed to inspect immutable R2 object {obj.key}: {exc}"
        ) from exc
    raw_metadata = response.get("Metadata", {})
    if not isinstance(raw_metadata, Mapping):
        return False
    metadata = {str(name).lower(): str(value) for name, value in raw_metadata.items()}
    expected = dict(obj.expected_metadata)
    source_field = "source-sha" if "source-sha" in expected else "release-sha"
    expected[source_field] = source_sha
    return all(
        metadata.get(name) == value for name, value in expected.items()
    ) and bool(_SHA256_RE.fullmatch(metadata.get("sha256", "")))


def discover_r2_component_allocation(
    client,
    bucket: str,
    component: str,
    version: str,
    source_sha: str,
) -> AllocationRecord | None:
    """Probe one component version for an immutable R2 allocation."""
    if not bucket:
        raise ValueError("R2 bucket is required for immutable allocation discovery")
    normalized = normalize_component_version(component, version)
    objects = _version_objects(client, bucket, component, normalized)
    if not objects:
        return None
    canonical_prefix = component_by_id(component).tag_prefix
    reusable = bool(source_sha) and all(
        _matches_source_binding(client, bucket, obj, source_sha) for obj in objects
    )
    return AllocationRecord(
        component=component,
        version=normalized,
        kind="resource",
        source_sha=source_sha if reusable else "",
        reference=(
            f"{canonical_prefix}{normalized}"
            if reusable
            else f"r2://{bucket}/{objects[0].key}"
        ),
        reusable=reusable,
    )
