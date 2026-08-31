#!/usr/bin/env python3
"""Publish one signed suite browser artifact through immutable R2 effects.

Signing jobs persist a DMG and its complete release receipt in Actions. This
module validates that receipt against the family transaction, then creates each
versioned R2 object conditionally so retries accept identical bytes and reject
every conflicting identity or checksum without overwriting public state.
"""

from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Protocol

from ..products.resource_sources import source_resources_for_product
from .suite import SUITE_PRODUCTS, SuiteRecord


_SHA256_RE = re.compile(r"[0-9a-f]{64}")


def _sha256(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


@dataclass(frozen=True)
class SuiteArtifactPublication:
    """Auditable immutable effects for one product's signed suite artifact."""

    product: str
    version: str
    source_sha: str
    reservation_sha: str
    artifact_key: str
    artifact_sha256: str
    receipt_key: str
    receipt_sha256: str

    def to_json(self) -> str:
        return json.dumps(self.__dict__, indent=2, sort_keys=True) + "\n"


class ImmutableObjectBackend(Protocol):
    def ensure(
        self,
        key: str,
        content: bytes,
        *,
        content_type: str,
        binding: Mapping[str, str],
    ) -> None: ...


def _read_body(response: Mapping[str, object], key: str) -> bytes:
    body = response.get("Body")
    if body is None or not hasattr(body, "read"):
        raise RuntimeError(f"R2 object {key} returned no readable body")
    try:
        content = body.read()
    finally:
        close = getattr(body, "close", None)
        if close:
            close()
    if not isinstance(content, bytes):
        raise RuntimeError(f"R2 object {key} returned non-byte content")
    return content


def _is_precondition_failure(error: Exception) -> bool:
    response = getattr(error, "response", {})
    code = str(response.get("Error", {}).get("Code", ""))
    status = response.get("ResponseMetadata", {}).get("HTTPStatusCode")
    return code in {
        "409",
        "412",
        "Conflict",
        "ConditionalRequestConflict",
        "PreconditionFailed",
    } or status in {409, 412}


class R2ImmutableObjectBackend:
    """Conditional-create adapter for the suite's versioned R2 namespace."""

    def __init__(self, client, bucket: str) -> None:
        self.client = client
        self.bucket = bucket

    def _verify(self, key: str, content: bytes, binding: Mapping[str, str]) -> None:
        try:
            response = self.client.get_object(Bucket=self.bucket, Key=key)
            actual = _read_body(response, key)
        except Exception as error:
            raise RuntimeError(
                f"Failed to verify immutable R2 object {key}: {error}"
            ) from error
        if actual != content:
            raise RuntimeError(
                f"Immutable R2 object {key} conflicts: "
                f"expected sha256 {_sha256(content)}, got {_sha256(actual)}"
            )
        metadata = response.get("Metadata", {})
        if not isinstance(metadata, dict):
            raise RuntimeError(f"Immutable R2 object {key} has invalid metadata")
        mismatches = {
            name: {"expected": value, "actual": metadata.get(name)}
            for name, value in binding.items()
            if metadata.get(name) != value
        }
        if mismatches:
            raise RuntimeError(
                f"Immutable R2 object {key} binding conflicts: {mismatches}"
            )

    def ensure(
        self,
        key: str,
        content: bytes,
        *,
        content_type: str,
        binding: Mapping[str, str],
    ) -> None:
        expected_binding = {**binding, "sha256": _sha256(content)}
        try:
            self.client.put_object(
                Bucket=self.bucket,
                Key=key,
                Body=content,
                ContentType=content_type,
                Metadata=expected_binding,
                IfNoneMatch="*",
            )
        except Exception as error:
            if not _is_precondition_failure(error):
                raise RuntimeError(
                    f"Failed to create immutable R2 object {key}: {error}"
                ) from error
        # Verification is mandatory after both create and precondition failure:
        # conditional requests provide atomicity, while the read proves bytes and
        # transaction binding rather than trusting object existence alone.
        self._verify(key, content, expected_binding)


def _string_map(value: object, name: str) -> dict[str, str]:
    if not isinstance(value, dict) or not all(
        isinstance(key, str) and isinstance(item, str) for key, item in value.items()
    ):
        raise ValueError(f"Suite artifact receipt {name} must be a string map")
    return dict(value)


def publish_suite_browser_artifact(
    record: SuiteRecord,
    product: str,
    artifact_root: Path,
    backend: ImmutableObjectBackend,
) -> SuiteArtifactPublication:
    """Validate and conditionally publish one product's signed DMG and receipt."""
    if record.state != "merged":
        raise ValueError("Suite browser artifacts publish only after the state merge")
    if product not in SUITE_PRODUCTS:
        raise ValueError(f"Unknown suite product: {product}")
    receipt_path = artifact_root / "release.json"
    document = json.loads(receipt_path.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise ValueError("Suite browser release receipt must be an object")

    expected_identity: Mapping[str, object] = {
        "product": product,
        "platform": "macos",
        "version": record.browser_version,
        "source_sha": record.source_sha,
        "reservation_sha": record.reservation_sha,
    }
    for name, expected in expected_identity.items():
        if document.get(name) != expected:
            raise ValueError(
                f"Suite browser release receipt {name} does not match transaction"
            )
    resources = source_resources_for_product(product)
    expected_components = {
        resources.server_component: record.component_versions[
            resources.server_component
        ],
        resources.extension_component: record.component_versions[
            resources.extension_component
        ],
        resources.onboarding_component: record.component_versions[
            resources.onboarding_component
        ],
    }
    if (
        _string_map(document.get("component_versions"), "component_versions")
        != expected_components
    ):
        raise ValueError(
            "Suite browser release receipt component_versions do not match transaction"
        )

    artifacts = document.get("artifacts")
    if not isinstance(artifacts, dict) or set(artifacts) != {"arm64"}:
        raise ValueError("Suite browser release receipt must contain only arm64")
    artifact = artifacts["arm64"]
    if not isinstance(artifact, dict):
        raise ValueError("Suite browser release artifact metadata must be an object")
    filename = artifact.get("filename")
    checksum = artifact.get("sha256")
    size = artifact.get("size")
    if (
        not isinstance(filename, str)
        or Path(filename).name != filename
        or not isinstance(checksum, str)
        or not _SHA256_RE.fullmatch(checksum)
        or not isinstance(size, int)
        or size <= 0
        or not isinstance(artifact.get("sparkle_signature"), str)
        or not artifact.get("sparkle_signature")
        or artifact.get("sparkle_length") != size
    ):
        raise ValueError("Suite browser release artifact metadata is incomplete")
    artifact_path = artifact_root / filename
    content = artifact_path.read_bytes()
    if len(content) != size or _sha256(content) != checksum:
        raise ValueError("Suite browser release artifact bytes do not match receipt")

    prefix = f"releases/{product}/{record.browser_version}/macos"
    artifact_key = f"{prefix}/{filename}"
    receipt_key = f"{prefix}/release.json"
    binding = {
        "transaction": record.transaction_id,
        "product": product,
        "source-sha": record.source_sha,
        "reservation-sha": record.reservation_sha,
    }
    backend.ensure(
        artifact_key,
        content,
        content_type="application/x-apple-diskimage",
        binding=binding,
    )
    receipt_content = receipt_path.read_bytes()
    backend.ensure(
        receipt_key,
        receipt_content,
        content_type="application/json",
        binding=binding,
    )
    return SuiteArtifactPublication(
        product=product,
        version=record.browser_version,
        source_sha=record.source_sha,
        reservation_sha=record.reservation_sha,
        artifact_key=artifact_key,
        artifact_sha256=checksum,
        receipt_key=receipt_key,
        receipt_sha256=_sha256(receipt_content),
    )
