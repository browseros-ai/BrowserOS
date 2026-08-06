#!/usr/bin/env python3
"""Finalize an attested browser candidate without promoting it."""

import hashlib
import json
import re
import tempfile
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Mapping, Protocol

from ..core.products import get_product_descriptor
from .candidate import CandidateRecord
from .common import generate_release_notes, validate_release_metadata
from .feeds.render import render_browser_appcast
from .feeds.spec import browser_feeds_for_product
from .github import (
    create_github_release,
    delete_github_release_asset,
    download_file,
    edit_github_release,
    github_release_tag,
    inspect_github_release,
    upload_to_github_release,
    verify_github_release_target,
)
from .lane import ArtifactAttestation, LaneGate


FINALIZATION_SCHEMA = "browseros-release-finalization-v1"


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _expected_asset_metadata(
    metadata: Mapping[str, Mapping[str, object]],
) -> dict[str, dict[str, object]]:
    result = {}
    for release in metadata.values():
        artifacts = release.get("artifacts")
        if not isinstance(artifacts, dict):
            raise ValueError("Release metadata artifacts must be an object")
        for artifact in artifacts.values():
            if not isinstance(artifact, dict):
                raise ValueError("Release metadata artifact must be an object")
            result[str(artifact["filename"])] = {
                "sha256": str(artifact["sha256"]).lower(),
                "size": artifact["size"],
            }
    return result


def _asset_metadata_matches(
    release: Mapping[str, object],
    expected: Mapping[str, Mapping[str, object]],
) -> bool:
    current = release.get("asset_metadata")
    if not isinstance(current, dict) or set(current) != set(expected):
        return False
    return all(current[name] == identity for name, identity in expected.items())


@dataclass(frozen=True)
class DraftState:
    """Browser draft release state after reconciliation."""

    tag: str
    url: str
    target_sha: str
    action: str
    assets: tuple[str, ...]


class DraftBackend(Protocol):
    def ensure_draft(
        self, candidate: CandidateRecord, metadata: dict[str, dict]
    ) -> DraftState: ...


@dataclass(frozen=True)
class FinalizationRecord:
    """Auditable browser-only finalization result."""

    product: str
    parent_sha: str
    candidate_sha: str
    merge_sha: str
    browser_version: str
    component_versions: Mapping[str, str]
    common_manifest_digest: str
    lanes: tuple[str, ...]
    outcomes: tuple[str, ...]
    server_checksums: Mapping[str, str]
    artifact_checksums: Mapping[str, str]
    pull_request_number: int
    pull_request_url: str
    draft: DraftState
    appcast_previews: Mapping[str, str]
    schema: str = FINALIZATION_SCHEMA

    def to_dict(self) -> dict[str, object]:
        document = asdict(self)
        document["draft"] = asdict(self.draft)
        return document

    def to_json(self) -> str:
        return json.dumps(self.to_dict(), indent=2, sort_keys=True) + "\n"

    def summary(self) -> str:
        versions = ", ".join(
            f"{name}={version}"
            for name, version in sorted(self.component_versions.items())
        )
        servers = "\n".join(
            f"  - `{name}`: `{checksum}`"
            for name, checksum in sorted(self.server_checksums.items())
        )
        artifacts = "\n".join(
            f"  - `{name}`: `{checksum}`"
            for name, checksum in sorted(self.artifact_checksums.items())
        )
        previews = "\n".join(
            f"  - `{key}`: `{path}`"
            for key, path in sorted(self.appcast_previews.items())
        )
        return (
            "## Browser release finalized\n\n"
            f"- Product: `{self.product}`\n"
            f"- Parent: `{self.parent_sha}`\n"
            f"- Candidate: `{self.candidate_sha}`\n"
            f"- Candidate PR: [#{self.pull_request_number}]({self.pull_request_url})\n"
            f"- Merge commit: `{self.merge_sha}`\n"
            f"- Browser version: `{self.browser_version}`\n"
            f"- Component versions: {versions}\n"
            f"- Common resources: `{self.common_manifest_digest}`\n"
            f"- Lanes: {', '.join(f'`{lane}` passed' for lane in self.lanes)}\n"
            f"- Outcomes: {', '.join(f'`{outcome}`' for outcome in self.outcomes)}\n"
            f"- Draft: [{self.draft.tag}]({self.draft.url}) "
            f"(`{self.draft.action}`, target `{self.draft.target_sha}`)\n\n"
            "### Server checksums\n\n"
            f"{servers}\n\n"
            "### Browser artifact checksums\n\n"
            f"{artifacts}\n\n"
            "### Local appcast previews\n\n"
            f"{previews}\n\n"
            "Review the draft and appcast previews, then publish separately.\n"
        )


def _validate_candidate(candidate: CandidateRecord, gate: LaneGate) -> None:
    if candidate.state != "merged" or not re.fullmatch(
        r"[0-9a-fA-F]{40}", candidate.merge_sha
    ):
        raise ValueError("Browser finalization requires a merged candidate")
    expected = {
        "product": candidate.product,
        "parent_sha": candidate.parent_sha,
        "candidate_sha": candidate.candidate_sha,
        "browser_version": candidate.browser_version,
        "component_versions": dict(candidate.component_versions),
    }
    for field, value in expected.items():
        if getattr(gate, field) != value:
            raise ValueError(f"Finalization gate {field} does not match candidate")


def _validate_metadata(
    candidate: CandidateRecord,
    gate: LaneGate,
    metadata: Mapping[str, Mapping[str, object]],
) -> dict[str, dict]:
    selected = validate_release_metadata(
        {platform: dict(release) for platform, release in metadata.items()},
        version=candidate.browser_version,
        product_id=candidate.product,
        platforms="all",
        macos_arch="universal",
        source_sha=candidate.candidate_sha,
    )
    attestations = {}
    for platform, release in selected.items():
        identity = {
            "parent_sha": candidate.parent_sha,
            "component_versions": dict(candidate.component_versions),
            "common_manifest_digest": gate.common_manifest_digest,
        }
        for field, expected in identity.items():
            if release.get(field) != expected:
                raise RuntimeError(
                    f"{platform} release metadata {field} mismatch: "
                    f"expected {expected!r}, got {release.get(field)!r}"
                )
        for artifact in release["artifacts"].values():
            filename = artifact["filename"]
            if Path(filename).name != filename:
                raise RuntimeError(f"Release artifact filename is unsafe: {filename}")
            checksum = artifact.get("sha256")
            if not isinstance(checksum, str) or not re.fullmatch(
                r"[0-9a-fA-F]{64}", checksum
            ):
                raise RuntimeError(
                    f"Release artifact checksum is invalid: {filename}"
                )
            if not isinstance(artifact.get("size"), int) or artifact["size"] <= 0:
                raise RuntimeError(f"Release artifact size is invalid: {filename}")
            if filename in attestations:
                raise RuntimeError(f"Duplicate release artifact filename: {filename}")
            attestation = ArtifactAttestation.from_dict(artifact)
            if attestation.sparkle_signature and artifact.get(
                "sparkle_length", attestation.size
            ) != attestation.size:
                raise RuntimeError(
                    f"Release artifact signature length mismatch: {filename}"
                )
            attestations[filename] = attestation
    if attestations != dict(gate.artifacts):
        raise RuntimeError("Release artifact evidence does not match the gate")
    return selected


def _render_previews(
    product: str,
    version: str,
    metadata: dict[str, dict],
) -> dict[str, str]:
    previews = {}
    for spec in browser_feeds_for_product(product):
        release = metadata.get(spec.platform)
        if release is None:
            continue
        artifacts = release["artifacts"]
        artifact_key = next(
            (key for key in spec.artifact_keys if key in artifacts), None
        )
        if artifact_key is None:
            continue
        previews[spec.key] = render_browser_appcast(
            spec,
            artifacts[artifact_key],
            version,
            release.get("sparkle_version", ""),
            release.get("build_date", ""),
        )
    if not previews:
        raise ValueError("No browser appcast previews could be rendered")
    return previews


def finalize_browser_release(
    candidate: CandidateRecord,
    gate: LaneGate,
    metadata: Mapping[str, Mapping[str, object]],
    preview_dir: Path,
    backend: DraftBackend,
) -> FinalizationRecord:
    """Validate evidence, reconcile a draft, and write local feed previews."""
    _validate_candidate(candidate, gate)
    selected = _validate_metadata(candidate, gate, metadata)
    rendered = _render_previews(candidate.product, candidate.browser_version, selected)
    preview_paths = {}
    for key, content in rendered.items():
        path = preview_dir / key
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        preview_paths[key] = str(path)
    draft = backend.ensure_draft(candidate, selected)
    return FinalizationRecord(
        product=candidate.product,
        parent_sha=candidate.parent_sha,
        candidate_sha=candidate.candidate_sha,
        merge_sha=candidate.merge_sha,
        browser_version=candidate.browser_version,
        component_versions=dict(candidate.component_versions),
        common_manifest_digest=gate.common_manifest_digest,
        lanes=gate.lanes,
        outcomes=gate.outcomes,
        server_checksums=dict(gate.server_checksums),
        artifact_checksums=dict(gate.artifact_checksums),
        pull_request_number=candidate.pull_request_number,
        pull_request_url=candidate.pull_request_url,
        draft=draft,
        appcast_previews=preview_paths,
    )


class GitHubDraftBackend:
    """Idempotent GitHub draft reconciliation for browser artifacts only."""

    def __init__(self, repo: str) -> None:
        self.repo = repo

    def ensure_draft(
        self, candidate: CandidateRecord, metadata: dict[str, dict]
    ) -> DraftState:
        product = get_product_descriptor(candidate.product)
        tag = github_release_tag(candidate.browser_version, candidate.product)
        title = f"{product.display_name} v{candidate.browser_version}"
        notes = generate_release_notes(candidate.browser_version, metadata, product)
        expected_metadata = _expected_asset_metadata(metadata)
        created, result = create_github_release(
            tag,
            self.repo,
            title,
            notes,
            draft=True,
            target=candidate.candidate_sha,
        )
        if created:
            verify_github_release_target(tag, self.repo, candidate.candidate_sha)
            current_assets: set[str] = set()
            action = "created"
        else:
            if "already exists" not in result.lower():
                raise RuntimeError(f"Failed to create browser draft: {result}")
            release = inspect_github_release(tag, self.repo)
            if release.get("isDraft") is not True:
                raise RuntimeError(f"Release {tag} exists and is not a draft")
            verify_github_release_target(
                tag, self.repo, candidate.candidate_sha, release=release
            )
            current_assets = set(release.get("assets", []))
            edit_github_release(tag, self.repo, title, notes)
            action = (
                "reused"
                if _asset_metadata_matches(release, expected_metadata)
                else "refreshed"
            )

        if action != "reused":
            with tempfile.TemporaryDirectory(prefix="browser-release-") as temp:
                root = Path(temp)
                prepared = []
                for release in metadata.values():
                    for artifact in release["artifacts"].values():
                        path = root / artifact["filename"]
                        if not download_file(artifact["url"], path):
                            raise RuntimeError(f"Failed to download {artifact['filename']}")
                        if path.stat().st_size != artifact["size"]:
                            raise RuntimeError(
                                f"Downloaded artifact size mismatch: {artifact['filename']}"
                            )
                        if _sha256(path) != artifact["sha256"]:
                            raise RuntimeError(
                                f"Downloaded artifact checksum mismatch: {artifact['filename']}"
                            )
                        prepared.append(path)
                for asset in sorted(current_assets):
                    delete_github_release_asset(tag, self.repo, asset)
                for path in prepared:
                    if not upload_to_github_release(tag, self.repo, path):
                        raise RuntimeError(f"Failed to upload {path.name}")
            final = inspect_github_release(tag, self.repo)
            if final.get("isDraft") is not True:
                raise RuntimeError(f"Release {tag} stopped being a draft")
            verify_github_release_target(
                tag, self.repo, candidate.candidate_sha, release=final
            )
            current_assets = set(final.get("assets", []))
            if not _asset_metadata_matches(final, expected_metadata):
                raise RuntimeError(
                    f"Draft release {tag} asset identity does not match the gate"
                )

        url = f"https://github.com/{self.repo}/releases/tag/{tag}"
        return DraftState(
            tag=tag,
            url=result if created and result.startswith("http") else url,
            target_sha=candidate.candidate_sha,
            action=action,
            assets=tuple(sorted(current_assets)),
        )
