#!/usr/bin/env python3
"""Browser lane attestations and complete release gating."""

import hashlib
import json
import platform
import re
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Mapping, Sequence

from ..core.context import Context
from ..lib.utils import get_platform
from ..steps.storage.upload import detect_artifacts
from .prepared_resources import load_prepared_resources


LANE_SCHEMA = "browseros-release-lane-v1"
GATE_SCHEMA = "browseros-release-gate-v1"
REQUIRED_OUTCOMES = frozenset(
    {
        "linux-x64",
        "windows-x64",
        "macos-arm64",
        "macos-x64",
        "macos-universal",
    }
)
SIGNED_OUTCOMES = frozenset(
    {"windows-x64", "macos-arm64", "macos-x64", "macos-universal"}
)
LANE_REQUIREMENTS = {
    "linux-x64": (frozenset({"linux-x64"}), frozenset({"linux-x64"})),
    "windows-x64": (frozenset({"windows-x64"}), frozenset({"windows-x64"})),
    "macos-universal": (
        frozenset({"macos-arm64", "macos-x64", "macos-universal"}),
        frozenset({"darwin-arm64", "darwin-x64"}),
    ),
}
REQUIRED_LANES = frozenset(LANE_REQUIREMENTS)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _require_digest(value: str, name: str, length: int = 64) -> None:
    if not re.fullmatch(rf"[0-9a-fA-F]{{{length}}}", value):
        raise ValueError(f"{name} must be a {length}-character hex digest")


@dataclass(frozen=True)
class ArtifactAttestation:
    """Checksum and update signature for one browser package."""

    filename: str
    size: int
    sha256: str
    url: str = ""
    sparkle_signature: str = ""

    @classmethod
    def from_dict(cls, document: Mapping[str, object]) -> "ArtifactAttestation":
        filename = document.get("filename")
        size = document.get("size")
        sha256 = document.get("sha256")
        url = document.get("url", "")
        signature = document.get("sparkle_signature", "")
        if not isinstance(filename, str) or not filename:
            raise ValueError("Lane artifact filename is invalid")
        if not isinstance(size, int) or size <= 0:
            raise ValueError(f"Lane artifact size is invalid: {filename}")
        if not isinstance(sha256, str):
            raise ValueError(f"Lane artifact checksum is invalid: {filename}")
        _require_digest(sha256, f"Lane artifact checksum for {filename}")
        if not isinstance(url, str) or not isinstance(signature, str):
            raise ValueError(f"Lane artifact metadata is invalid: {filename}")
        return cls(filename, size, sha256.lower(), url, signature)


@dataclass(frozen=True)
class LaneOutcome:
    """One required platform and architecture outcome."""

    id: str
    artifacts: tuple[str, ...]
    signed: bool

    @classmethod
    def from_dict(cls, document: Mapping[str, object]) -> "LaneOutcome":
        outcome_id = document.get("id")
        artifacts = document.get("artifacts")
        signed = document.get("signed")
        if not isinstance(outcome_id, str) or not outcome_id:
            raise ValueError("Lane outcome id is invalid")
        if not isinstance(artifacts, list) or not all(
            isinstance(value, str) and value for value in artifacts
        ):
            raise ValueError(f"Lane outcome artifacts are invalid: {outcome_id}")
        if not isinstance(signed, bool):
            raise ValueError(f"Lane outcome signed state is invalid: {outcome_id}")
        return cls(outcome_id, tuple(artifacts), signed)


@dataclass(frozen=True)
class LaneManifest:
    """Portable evidence emitted by one native browser lane."""

    lane_id: str
    product: str
    parent_sha: str
    candidate_sha: str
    browser_version: str
    component_versions: Mapping[str, str]
    common_manifest_digest: str
    server_checksums: Mapping[str, str]
    artifacts: Mapping[str, ArtifactAttestation]
    outcomes: Mapping[str, LaneOutcome]
    toolchain: Mapping[str, str]
    result: str
    schema: str = LANE_SCHEMA

    def to_dict(self) -> dict[str, object]:
        return {
            "schema": self.schema,
            "lane_id": self.lane_id,
            "product": self.product,
            "parent_sha": self.parent_sha,
            "candidate_sha": self.candidate_sha,
            "browser_version": self.browser_version,
            "component_versions": dict(self.component_versions),
            "common_manifest_digest": self.common_manifest_digest,
            "server_checksums": dict(self.server_checksums),
            "artifacts": {
                name: asdict(artifact) for name, artifact in self.artifacts.items()
            },
            "outcomes": {
                name: asdict(outcome) for name, outcome in self.outcomes.items()
            },
            "toolchain": dict(self.toolchain),
            "result": self.result,
        }

    def to_json(self) -> str:
        return json.dumps(self.to_dict(), indent=2, sort_keys=True) + "\n"

    @classmethod
    def from_dict(cls, document: Mapping[str, object]) -> "LaneManifest":
        if document.get("schema") != LANE_SCHEMA:
            raise ValueError("Unsupported lane manifest schema")
        strings = {}
        for field in (
            "lane_id",
            "product",
            "parent_sha",
            "candidate_sha",
            "browser_version",
            "common_manifest_digest",
            "result",
        ):
            value = document.get(field)
            if not isinstance(value, str):
                raise ValueError(f"Lane manifest {field} is invalid")
            strings[field] = value
        maps = {}
        for field in ("component_versions", "server_checksums", "toolchain"):
            value = document.get(field)
            if not isinstance(value, dict) or not all(
                isinstance(key, str) and isinstance(item, str)
                for key, item in value.items()
            ):
                raise ValueError(f"Lane manifest {field} is invalid")
            maps[field] = value
        raw_artifacts = document.get("artifacts")
        raw_outcomes = document.get("outcomes")
        if not isinstance(raw_artifacts, dict) or not isinstance(raw_outcomes, dict):
            raise ValueError("Lane manifest evidence maps are invalid")
        artifacts = {
            str(name): ArtifactAttestation.from_dict(value)
            for name, value in raw_artifacts.items()
            if isinstance(value, dict)
        }
        outcomes = {
            str(name): LaneOutcome.from_dict(value)
            for name, value in raw_outcomes.items()
            if isinstance(value, dict)
        }
        if len(artifacts) != len(raw_artifacts) or len(outcomes) != len(raw_outcomes):
            raise ValueError("Lane manifest evidence entries are invalid")
        return cls(
            **strings,
            component_versions=maps["component_versions"],
            server_checksums=maps["server_checksums"],
            artifacts=artifacts,
            outcomes=outcomes,
            toolchain=maps["toolchain"],
        )

    @classmethod
    def from_path(cls, path: Path) -> "LaneManifest":
        document = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(document, dict):
            raise ValueError("Lane manifest must be a JSON object")
        return cls.from_dict(document)


@dataclass(frozen=True)
class LaneGate:
    """Complete cross-platform evidence accepted before candidate merge."""

    product: str
    parent_sha: str
    candidate_sha: str
    browser_version: str
    component_versions: Mapping[str, str]
    common_manifest_digest: str
    lanes: tuple[str, ...]
    outcomes: tuple[str, ...]
    server_checksums: Mapping[str, str]
    artifacts: Mapping[str, ArtifactAttestation]
    passed: bool = True
    schema: str = GATE_SCHEMA

    def to_dict(self) -> dict[str, object]:
        return dict(asdict(self))

    def to_json(self) -> str:
        return json.dumps(self.to_dict(), indent=2, sort_keys=True) + "\n"

    @property
    def artifact_checksums(self) -> dict[str, str]:
        return {
            name: artifact.sha256 for name, artifact in self.artifacts.items()
        }

    def summary(self) -> str:
        versions = ", ".join(
            f"{name}={version}"
            for name, version in sorted(self.component_versions.items())
        )
        return (
            "## Browser release gate\n\n"
            f"- Product: `{self.product}`\n"
            f"- Parent: `{self.parent_sha}`\n"
            f"- Candidate: `{self.candidate_sha}`\n"
            f"- Browser: `{self.browser_version}`\n"
            f"- Components: {versions}\n"
            f"- Common resources: `{self.common_manifest_digest}`\n"
            f"- Outcomes: {', '.join(self.outcomes)}\n"
            "- Result: passed\n"
        )

    @classmethod
    def from_dict(cls, document: Mapping[str, object]) -> "LaneGate":
        if document.get("schema") != GATE_SCHEMA or document.get("passed") is not True:
            raise ValueError("Unsupported or failed release gate")
        strings = {}
        for field in (
            "product",
            "parent_sha",
            "candidate_sha",
            "browser_version",
            "common_manifest_digest",
        ):
            value = document.get(field)
            if not isinstance(value, str) or not value:
                raise ValueError(f"Release gate {field} is invalid")
            strings[field] = value
        maps = {}
        for field in ("component_versions", "server_checksums"):
            value = document.get(field)
            if not isinstance(value, dict) or not all(
                isinstance(key, str) and key and isinstance(item, str) and item
                for key, item in value.items()
            ):
                raise ValueError(f"Release gate {field} is invalid")
            maps[field] = value
        raw_artifacts = document.get("artifacts")
        if not isinstance(raw_artifacts, dict):
            raise ValueError("Release gate artifacts are invalid")
        artifacts = {
            str(name): ArtifactAttestation.from_dict(value)
            for name, value in raw_artifacts.items()
            if isinstance(value, dict)
        }
        if len(artifacts) != len(raw_artifacts) or any(
            name != artifact.filename for name, artifact in artifacts.items()
        ):
            raise ValueError("Release gate artifact entries are invalid")
        sequences = {}
        for field in ("lanes", "outcomes"):
            value = document.get(field)
            if not isinstance(value, (list, tuple)) or not all(
                isinstance(item, str) and item for item in value
            ):
                raise ValueError(f"Release gate {field} is invalid")
            sequences[field] = tuple(value)
        _require_digest(strings["parent_sha"], "release gate parent SHA", 40)
        _require_digest(strings["candidate_sha"], "release gate candidate SHA", 40)
        _require_digest(
            strings["common_manifest_digest"], "release gate common manifest"
        )
        for name, digest in maps["server_checksums"].items():
            _require_digest(digest, f"release gate checksum for {name}")
        if set(sequences["lanes"]) != REQUIRED_LANES:
            raise ValueError("Release gate lane set is incomplete")
        if set(sequences["outcomes"]) != REQUIRED_OUTCOMES:
            raise ValueError("Release gate outcome set is incomplete")
        return cls(
            **strings,
            component_versions=maps["component_versions"],
            lanes=sequences["lanes"],
            outcomes=sequences["outcomes"],
            server_checksums=maps["server_checksums"],
            artifacts=artifacts,
        )

    @classmethod
    def from_path(cls, path: Path) -> "LaneGate":
        document = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(document, dict):
            raise ValueError("Release gate must be a JSON object")
        return cls.from_dict(document)


def _identity_mismatch(field: str) -> str:
    labels = {
        "product": "product",
        "parent_sha": "parent",
        "candidate_sha": "candidate",
        "browser_version": "browser version",
        "component_versions": "component versions",
        "common_manifest_digest": "common manifest digest",
    }
    return labels[field]


def gate_lane_manifests(manifests: Sequence[LaneManifest]) -> LaneGate:
    """Require one consistent, successful full release matrix."""
    if not manifests:
        raise ValueError("Release gate received no lane manifests")
    lane_ids = [manifest.lane_id for manifest in manifests]
    if len(set(lane_ids)) != len(lane_ids):
        raise ValueError("Release gate contains a duplicate lane")
    if set(lane_ids) != REQUIRED_LANES:
        missing = ", ".join(sorted(REQUIRED_LANES - set(lane_ids)))
        extra = ", ".join(sorted(set(lane_ids) - REQUIRED_LANES))
        raise ValueError(
            f"Release gate lane set mismatch: missing {missing or '-'}; "
            f"extra {extra or '-'}"
        )
    for manifest in manifests:
        if manifest.result != "success":
            raise ValueError(f"Release lane {manifest.lane_id} failed")
        required_outcomes, required_servers = LANE_REQUIREMENTS[manifest.lane_id]
        if set(manifest.outcomes) != required_outcomes:
            raise ValueError(
                f"Release lane {manifest.lane_id} has invalid outcomes"
            )
        if set(manifest.server_checksums) != required_servers:
            actual_servers = set(manifest.server_checksums)
            missing = ", ".join(sorted(required_servers - actual_servers))
            extra = ", ".join(sorted(actual_servers - required_servers))
            raise ValueError(
                f"Release lane {manifest.lane_id} has invalid server targets: "
                f"missing {missing or '-'}; extra {extra or '-'}"
            )
        referenced_artifacts = {
            filename
            for outcome in manifest.outcomes.values()
            for filename in outcome.artifacts
        }
        if set(manifest.artifacts) != referenced_artifacts:
            raise ValueError(
                f"Release lane {manifest.lane_id} has unbound artifacts"
            )

    first = manifests[0]
    identity_fields = (
        "product",
        "parent_sha",
        "candidate_sha",
        "browser_version",
        "component_versions",
        "common_manifest_digest",
    )
    for manifest in manifests[1:]:
        for field in identity_fields:
            if getattr(manifest, field) != getattr(first, field):
                raise ValueError(
                    f"Release lane {_identity_mismatch(field)} mismatch"
                )

    _require_digest(first.parent_sha, "parent SHA", 40)
    _require_digest(first.candidate_sha, "candidate SHA", 40)
    _require_digest(first.common_manifest_digest, "common manifest digest")
    outcomes: dict[str, LaneOutcome] = {}
    artifacts: dict[str, ArtifactAttestation] = {}
    servers: dict[str, str] = {}
    for manifest in manifests:
        for name, checksum in manifest.server_checksums.items():
            _require_digest(checksum, f"server checksum for {name}")
            if name in servers and servers[name] != checksum:
                raise ValueError(f"Duplicate server target disagrees: {name}")
            servers[name] = checksum
        for name, artifact in manifest.artifacts.items():
            ArtifactAttestation.from_dict(asdict(artifact))
            if name != artifact.filename:
                raise ValueError(f"Lane artifact key mismatch: {name}")
            if name in artifacts and artifacts[name] != artifact:
                raise ValueError(f"Duplicate browser artifact disagrees: {name}")
            artifacts[name] = artifact
        for name, outcome in manifest.outcomes.items():
            if name != outcome.id:
                raise ValueError(f"Lane outcome key mismatch: {name}")
            if name in outcomes:
                raise ValueError(f"Release gate contains duplicate outcome {name}")
            missing_artifacts = set(outcome.artifacts) - set(manifest.artifacts)
            if missing_artifacts:
                raise ValueError(
                    f"Lane outcome {name} omits artifact evidence: "
                    f"{', '.join(sorted(missing_artifacts))}"
                )
            if name in SIGNED_OUTCOMES:
                if not outcome.signed:
                    raise ValueError(f"Lane outcome {name} must be signed")
                signable = [
                    manifest.artifacts[filename]
                    for filename in outcome.artifacts
                    if filename.endswith((".dmg", ".exe"))
                ]
                if not signable or any(
                    not artifact.sparkle_signature for artifact in signable
                ):
                    raise ValueError(
                        f"Lane outcome {name} lacks update signatures"
                    )
            outcomes[name] = outcome

    actual_outcomes = set(outcomes)
    if actual_outcomes != REQUIRED_OUTCOMES:
        missing = ", ".join(sorted(REQUIRED_OUTCOMES - actual_outcomes))
        extra = ", ".join(sorted(actual_outcomes - REQUIRED_OUTCOMES))
        raise ValueError(
            f"Release gate missing outcomes: {missing or '-'}; extra: {extra or '-'}"
        )
    mac_lane = next(
        (manifest for manifest in manifests if manifest.lane_id == "macos-universal"),
        None,
    )
    if mac_lane is None:
        raise ValueError("Release gate is missing macos-universal lane")
    for target in ("darwin-arm64", "darwin-x64"):
        if target not in mac_lane.server_checksums:
            raise ValueError(f"macos-universal lane is missing {target}")
    for target in ("linux-x64", "windows-x64"):
        if target not in servers:
            raise ValueError(f"Release gate is missing {target} server evidence")

    return LaneGate(
        product=first.product,
        parent_sha=first.parent_sha,
        candidate_sha=first.candidate_sha,
        browser_version=first.browser_version,
        component_versions=dict(first.component_versions),
        common_manifest_digest=first.common_manifest_digest,
        lanes=tuple(sorted(lane_ids)),
        outcomes=tuple(sorted(outcomes)),
        server_checksums=dict(sorted(servers.items())),
        artifacts=dict(sorted(artifacts.items())),
    )


def _release_artifacts(contexts: Sequence[Context]) -> dict[str, dict[str, object]]:
    for context in reversed(contexts):
        metadata = context.artifact_registry.get("release_metadata")
        if isinstance(metadata, dict):
            artifacts = metadata.get("artifacts")
            if isinstance(artifacts, dict):
                return {
                    str(item.get("filename")): item
                    for item in artifacts.values()
                    if isinstance(item, dict) and item.get("filename")
                }
    return {}


def _outcomes_for_platform(
    platform_name: str,
    artifact_names: Sequence[str],
    signed_names: set[str],
) -> dict[str, LaneOutcome]:
    def matching(token: str, suffixes: tuple[str, ...]) -> tuple[str, ...]:
        return tuple(
            name
            for name in artifact_names
            if token in name and name.endswith(suffixes)
        )

    if platform_name == "macos":
        outcomes = {}
        for architecture in ("arm64", "x64", "universal"):
            names = matching(f"_{architecture}", (".dmg",))
            outcomes[f"macos-{architecture}"] = LaneOutcome(
                f"macos-{architecture}",
                names,
                bool(names) and all(name in signed_names for name in names),
            )
        return outcomes
    if platform_name == "windows":
        names = matching("_x64_", (".exe", ".zip"))
        signable = [name for name in names if name.endswith(".exe")]
        return {
            "windows-x64": LaneOutcome(
                "windows-x64",
                names,
                bool(signable) and all(name in signed_names for name in signable),
            )
        }
    raise ValueError(f"Filename-derived outcomes are unsupported for {platform_name}")


def build_lane_manifest(
    contexts: Sequence[Context],
    toolchain: Mapping[str, str] | None = None,
) -> LaneManifest:
    """Build a successful lane attestation from completed run contexts."""
    if not contexts:
        raise ValueError("Cannot attest an empty build run")
    first = contexts[0]
    if first.resource_mode != "source" or first.prepared_resources is None:
        raise ValueError("Lane attestations require source-mode prepared resources")
    common = load_prepared_resources(first.prepared_resources)
    release_artifacts = _release_artifacts(contexts)
    paths = detect_artifacts(contexts[-1])
    signed_names = set()
    signatures_by_name = {}
    for context in contexts:
        signatures = context.artifact_registry.get("sparkle_signatures", {})
        if isinstance(signatures, dict):
            signed_names.update(str(name) for name in signatures)
            for name, value in signatures.items():
                if isinstance(value, tuple) and value and isinstance(value[0], str):
                    signatures_by_name[str(name)] = value[0]
    evidence = {}
    for path in paths:
        metadata = release_artifacts.get(path.name, {})
        signature = metadata.get("sparkle_signature", "") or signatures_by_name.get(
            path.name, ""
        )
        if isinstance(signature, str) and signature:
            signed_names.add(path.name)
        evidence[path.name] = ArtifactAttestation(
            filename=path.name,
            size=path.stat().st_size,
            sha256=_sha256(path),
            url=str(metadata.get("url", "")),
            sparkle_signature=str(signature),
        )
    platform_name = get_platform()
    if platform_name == "linux":
        if contexts[-1].architecture != "x64":
            raise ValueError(
                "The Linux release lane supports only the x64 artifact pair"
            )
        # `detect_artifacts` obtains these paths from LinuxArtifactPair. Do not
        # throw away that correlation and reconstruct it from filename tokens.
        outcomes = {
            "linux-x64": LaneOutcome(
                "linux-x64",
                tuple(path.name for path in paths),
                False,
            )
        }
    else:
        outcomes = _outcomes_for_platform(
            platform_name,
            tuple(sorted(evidence)),
            signed_names,
        )
    if any(not outcome.artifacts for outcome in outcomes.values()):
        missing = [name for name, outcome in outcomes.items() if not outcome.artifacts]
        raise ValueError(f"Lane is missing browser artifacts: {', '.join(missing)}")
    servers = {}
    for context in contexts:
        results = context.artifact_registry.get("server_resources", {})
        if isinstance(results, dict):
            for target, result in results.items():
                checksum = getattr(result, "manifest_sha256", "")
                if checksum:
                    servers[str(target)] = str(checksum)
    lane_id = "macos-universal" if platform_name == "macos" else next(iter(outcomes))
    identity = {
        "python": platform.python_version(),
        "system": platform.platform(),
        "machine": platform.machine(),
        **dict(toolchain or {}),
    }
    return LaneManifest(
        lane_id=lane_id,
        product=common.product,
        parent_sha=common.parent_sha,
        candidate_sha=common.source_sha,
        browser_version=common.browser_version,
        component_versions=dict(common.component_versions),
        common_manifest_digest=common.digest(),
        server_checksums=servers,
        artifacts=evidence,
        outcomes=outcomes,
        toolchain=identity,
        result="success",
    )


def write_lane_manifest(
    contexts: Sequence[Context],
    output: Path,
    toolchain: Mapping[str, str] | None = None,
) -> LaneManifest:
    """Write one completed lane attestation."""
    manifest = build_lane_manifest(contexts, toolchain)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(manifest.to_json(), encoding="utf-8")
    return manifest
