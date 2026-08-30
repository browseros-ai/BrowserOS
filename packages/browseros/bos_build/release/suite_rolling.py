#!/usr/bin/env python3
"""Resumable publication of one mutable family-nightly GitHub release.

The suite owns immutable browser objects first, then reconciles each rolling
release as a monotonic saga. Every GitHub write is followed by a fresh read so
a retry can safely continue after release, asset-upload, or tag cleanup crashes.
"""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, Protocol
from urllib.parse import quote


_SHA_RE = re.compile(r"^[0-9a-fA-F]{40}$")
_VERSION_RE = re.compile(r"^[0-9]+(?:\.[0-9]+){2,3}$")
_BODY_VERSION_RE = re.compile(r"^Browser version: `([^`]+)`$", re.MULTILINE)
_LEGACY_VERSION_RE = re.compile(
    r"^Automated signed .* macOS arm64 nightly for v([^ ]+)\.$", re.MULTILINE
)
_BODY_SOURCE_RE = re.compile(r"^Transaction source: `([0-9a-fA-F]{40})`$", re.MULTILINE)


@dataclass(frozen=True)
class RollingAsset:
    """One GitHub release asset identity returned by the Releases API."""

    name: str
    digest: str


@dataclass(frozen=True)
class RollingRelease:
    """Live mutable-release state, including drafts hidden by tag lookup APIs."""

    tag: str
    title: str
    target_sha: str
    body: str
    draft: bool
    prerelease: bool
    assets: tuple[RollingAsset, ...]


@dataclass(frozen=True)
class RollingReleaseRequest:
    """Desired identity for one product's family-nightly rolling release."""

    tag: str
    title: str
    source_sha: str
    browser_version: str
    asset: Path


RollingOutcome = Literal["created", "reused", "superseded"]


class RollingReleaseBackend(Protocol):
    """External GitHub effects required by the monotonic reconciliation."""

    def find_release(self, tag: str) -> RollingRelease | None: ...

    def tag_target(self, tag: str) -> str | None: ...

    def is_ancestor(self, older_sha: str, newer_sha: str) -> bool: ...

    def delete_release(self, tag: str) -> None: ...

    def delete_tag(self, tag: str) -> None: ...

    def create_draft(self, request: RollingReleaseRequest, notes: str) -> None: ...

    def upload_asset(self, tag: str, asset: Path) -> None: ...

    def publish_draft(self, tag: str) -> None: ...


def rolling_release_notes(request: RollingReleaseRequest) -> str:
    """Render the machine-readable identity embedded in a rolling release."""
    return (
        "Automated signed family macOS arm64 nightly for "
        f"v{request.browser_version}.\n\n"
        f"Browser version: `{request.browser_version}`\n"
        f"Transaction source: `{request.source_sha}`\n"
    )


def _version_key(version: str) -> tuple[int, ...]:
    if _VERSION_RE.fullmatch(version) is None:
        raise ValueError(f"Invalid rolling browser version: {version}")
    return tuple(int(part) for part in version.split("."))


def _release_version(release: RollingRelease) -> str:
    match = _BODY_VERSION_RE.search(release.body)
    if match is None:
        match = _LEGACY_VERSION_RE.search(release.body)
    if match is None or _VERSION_RE.fullmatch(match.group(1)) is None:
        raise ValueError(
            f"{release.tag} has no trustworthy browser version; refusing to move it"
        )
    return match.group(1)


def _asset_identity(path: Path) -> RollingAsset:
    if not path.is_file():
        raise ValueError(f"Rolling release asset does not exist: {path}")
    # Signed DMGs are large; hash the stream instead of duplicating the entire
    # artifact in runner memory during the mutable publication phase.
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    return RollingAsset(path.name, f"sha256:{digest}")


def _validate_request(request: RollingReleaseRequest) -> RollingAsset:
    if not request.tag or not request.title:
        raise ValueError("Rolling release tag and title are required")
    if _SHA_RE.fullmatch(request.source_sha) is None:
        raise ValueError("Rolling release source must be a full commit SHA")
    _version_key(request.browser_version)
    return _asset_identity(request.asset)


def _source_order(
    backend: RollingReleaseBackend, existing_sha: str, desired_sha: str
) -> Literal["same", "older", "newer"]:
    if _SHA_RE.fullmatch(existing_sha) is None:
        raise ValueError("Rolling release target is not a full commit SHA")
    if existing_sha == desired_sha:
        return "same"
    if backend.is_ancestor(existing_sha, desired_sha):
        return "older"
    if backend.is_ancestor(desired_sha, existing_sha):
        return "newer"
    raise ValueError("Rolling release target is not in the transaction source lineage")


def _verify_release_identity(
    release: RollingRelease,
    request: RollingReleaseRequest,
    desired_asset: RollingAsset,
    *,
    allow_draft: bool,
) -> None:
    if release.tag != request.tag:
        raise ValueError("GitHub returned the wrong rolling release tag")
    if release.target_sha != request.source_sha:
        raise ValueError(f"{request.tag} belongs to a different transaction source")
    if release.title != request.title or release.body != rolling_release_notes(request):
        raise ValueError(f"{request.tag} has conflicting release metadata")
    if release.draft and not allow_draft:
        raise ValueError(f"{request.tag} remained a draft after publication")
    if not release.draft and not release.prerelease:
        raise ValueError(f"{request.tag} is not a prerelease")
    if any(asset.name != desired_asset.name for asset in release.assets):
        raise ValueError(f"{request.tag} contains unexpected assets")
    matching = [asset for asset in release.assets if asset.name == desired_asset.name]
    if len(matching) > 1 or (matching and matching[0].digest != desired_asset.digest):
        raise ValueError(f"{request.tag} same source has conflicting assets")


def _finish_draft(
    request: RollingReleaseRequest,
    desired_asset: RollingAsset,
    backend: RollingReleaseBackend,
) -> RollingOutcome:
    draft = backend.find_release(request.tag)
    if draft is None or not draft.draft:
        raise ValueError(f"{request.tag} draft disappeared during reconciliation")
    _verify_release_identity(draft, request, desired_asset, allow_draft=True)
    draft_tag_target = backend.tag_target(request.tag)
    if draft_tag_target is not None and draft_tag_target != request.source_sha:
        raise ValueError(f"{request.tag} draft and tag target different sources")
    if not draft.assets:
        backend.upload_asset(request.tag, request.asset)
        draft = backend.find_release(request.tag)
        if draft is None or not draft.draft:
            raise ValueError(f"{request.tag} draft disappeared after asset upload")
        _verify_release_identity(draft, request, desired_asset, allow_draft=True)
        if tuple(draft.assets) != (desired_asset,):
            raise ValueError(f"{request.tag} asset upload was not durably recorded")
        draft_tag_target = backend.tag_target(request.tag)
        if draft_tag_target is not None and draft_tag_target != request.source_sha:
            raise ValueError(f"{request.tag} draft and tag target different sources")

    backend.publish_draft(request.tag)
    published = backend.find_release(request.tag)
    target = backend.tag_target(request.tag)
    if published is None or target != request.source_sha:
        raise ValueError(
            f"{request.tag} publication did not create the exact source tag"
        )
    _verify_release_identity(published, request, desired_asset, allow_draft=False)
    if tuple(published.assets) != (desired_asset,):
        raise ValueError(f"{request.tag} published asset digest does not match")
    return "created"


def _clear_or_classify_tag(
    request: RollingReleaseRequest, backend: RollingReleaseBackend
) -> bool:
    """Remove a same/older orphan tag; return false for a newer transaction."""
    target = backend.tag_target(request.tag)
    if target is None:
        return True
    order = _source_order(backend, target, request.source_sha)
    if order == "newer":
        return False
    backend.delete_tag(request.tag)
    if backend.tag_target(request.tag) is not None:
        raise ValueError(f"{request.tag} survived explicit tag deletion")
    return True


def _create_release(
    request: RollingReleaseRequest,
    desired_asset: RollingAsset,
    backend: RollingReleaseBackend,
) -> RollingOutcome:
    backend.create_draft(request, rolling_release_notes(request))
    # Draft creation, upload, and publication are independent GitHub writes.
    # Re-read after each so a successful API effect followed by runner death is
    # indistinguishable from an ordinary retry.
    return _finish_draft(request, desired_asset, backend)


def reconcile_rolling_release(
    request: RollingReleaseRequest, backend: RollingReleaseBackend
) -> RollingOutcome:
    """Reconcile a rolling release without ever moving its tag backward."""
    desired_asset = _validate_request(request)
    release = backend.find_release(request.tag)
    if release is None:
        # A failed release deletion may leave its mutable tag behind. ``--target``
        # does not retarget an existing tag, so classify and explicitly remove
        # only a same/older source before creating the replacement draft.
        if not _clear_or_classify_tag(request, backend):
            return "superseded"
        return _create_release(request, desired_asset, backend)

    current_version = _release_version(release)
    tag_target = backend.tag_target(request.tag)
    if tag_target is not None and release.target_sha != tag_target:
        # A release record and its live Git ref are one identity. Trusting only
        # one side could delete a newer tag or accept a release whose downloads
        # are attached to a different source than its visible ref.
        raise ValueError(f"{request.tag} release and tag target different sources")
    current_target = tag_target or release.target_sha
    source_order = _source_order(backend, current_target, request.source_sha)
    desired_version = _version_key(request.browser_version)
    existing_version = _version_key(current_version)

    if existing_version == desired_version:
        if source_order != "same":
            raise ValueError(
                f"{request.tag} same version belongs to a different source"
            )
        body_source = _BODY_SOURCE_RE.search(release.body)
        if (
            body_source is not None
            and body_source.group(1).lower() != request.source_sha
        ):
            raise ValueError(f"{request.tag} body belongs to a different source")
        if release.draft:
            return _finish_draft(request, desired_asset, backend)
        _verify_release_identity(release, request, desired_asset, allow_draft=False)
        if tuple(release.assets) != (desired_asset,):
            raise ValueError(f"{request.tag} same source has conflicting assets")
        if tag_target is None:
            # Published releases are incomplete without the mutable tag. Once
            # the exact source and bytes are proven, recreate the pair through
            # the same draft/upload/publish saga instead of reporting success.
            backend.delete_release(request.tag)
            if backend.find_release(request.tag) is not None:
                raise ValueError(f"{request.tag} survived release deletion")
            if not _clear_or_classify_tag(request, backend):
                return "superseded"
            return _create_release(request, desired_asset, backend)
        return "reused"

    if existing_version > desired_version:
        if source_order != "newer":
            raise ValueError(
                f"{request.tag} version/source order conflicts with transaction ancestry"
            )
        return "superseded"

    if source_order != "older":
        raise ValueError(
            f"{request.tag} version/source order conflicts with transaction ancestry"
        )
    backend.delete_release(request.tag)
    if backend.find_release(request.tag) is not None:
        raise ValueError(f"{request.tag} survived release deletion")
    if not _clear_or_classify_tag(request, backend):
        return "superseded"
    return _create_release(request, desired_asset, backend)


class GitHubRollingReleaseBackend:
    """GitHub CLI adapter that exposes drafts and dereferences remote tags."""

    def __init__(self, repo: str, repo_root: Path):
        self.repo = repo
        self.repo_root = repo_root.resolve()

    def _run(self, *args: str) -> str:
        result = subprocess.run(
            args,
            cwd=self.repo_root,
            check=True,
            capture_output=True,
            text=True,
        )
        return result.stdout

    def _api_json(self, endpoint: str) -> object:
        return json.loads(self._run("gh", "api", endpoint))

    def _optional_api_json(self, endpoint: str) -> object | None:
        result = subprocess.run(
            ("gh", "api", endpoint),
            cwd=self.repo_root,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            return json.loads(result.stdout)
        if "HTTP 404" in result.stderr:
            return None
        raise subprocess.CalledProcessError(
            result.returncode,
            result.args,
            output=result.stdout,
            stderr=result.stderr,
        )

    def find_release(self, tag: str) -> RollingRelease | None:
        raw_pages = json.loads(
            self._run(
                "gh",
                "api",
                "--paginate",
                "--slurp",
                f"repos/{self.repo}/releases?per_page=100",
            )
        )
        if not isinstance(raw_pages, list) or any(
            not isinstance(page, list) for page in raw_pages
        ):
            raise ValueError("GitHub returned invalid paginated release metadata")
        releases = [item for page in raw_pages for item in page]
        if any(not isinstance(item, dict) for item in releases):
            raise ValueError("GitHub returned invalid release metadata")
        matches = [item for item in releases if item.get("tag_name") == tag]
        if len(matches) > 1:
            raise ValueError(f"GitHub returned multiple releases for {tag}")
        if not matches:
            return None
        item = matches[0]
        assets = tuple(
            sorted(
                (
                    RollingAsset(
                        str(asset.get("name", "")), str(asset.get("digest", ""))
                    )
                    for asset in item.get("assets", [])
                ),
                key=lambda asset: asset.name,
            )
        )
        return RollingRelease(
            tag=tag,
            title=str(item.get("name", "")),
            target_sha=str(item.get("target_commitish", "")).lower(),
            body=str(item.get("body", "")),
            draft=item.get("draft") is True,
            prerelease=item.get("prerelease") is True,
            assets=assets,
        )

    def tag_target(self, tag: str) -> str | None:
        encoded = quote(tag, safe="")
        ref = self._optional_api_json(f"repos/{self.repo}/git/ref/tags/{encoded}")
        if ref is None:
            return None
        if not isinstance(ref, dict) or not isinstance(ref.get("object"), dict):
            raise ValueError(f"GitHub returned invalid tag metadata for {tag}")
        target = ref["object"]
        for _ in range(8):
            target_type = target.get("type")
            target_sha = str(target.get("sha", "")).lower()
            if _SHA_RE.fullmatch(target_sha) is None:
                raise ValueError(f"GitHub returned invalid tag target for {tag}")
            if target_type == "commit":
                return target_sha
            if target_type != "tag":
                raise ValueError(f"GitHub tag {tag} does not resolve to a commit")
            tag_object = self._api_json(f"repos/{self.repo}/git/tags/{target_sha}")
            if not isinstance(tag_object, dict) or not isinstance(
                tag_object.get("object"), dict
            ):
                raise ValueError(f"GitHub returned invalid annotated tag for {tag}")
            target = tag_object["object"]
        raise ValueError(f"GitHub tag {tag} has excessive annotation depth")

    def is_ancestor(self, older_sha: str, newer_sha: str) -> bool:
        comparison = self._api_json(
            f"repos/{self.repo}/compare/{older_sha}...{newer_sha}"
        )
        if not isinstance(comparison, dict):
            raise ValueError("GitHub returned invalid commit comparison metadata")
        return comparison.get("status") in ("ahead", "identical")

    def delete_release(self, tag: str) -> None:
        self._run(
            "gh",
            "release",
            "delete",
            tag,
            "--cleanup-tag",
            "--yes",
            "--repo",
            self.repo,
        )

    def delete_tag(self, tag: str) -> None:
        self._run(
            "gh",
            "api",
            "--method",
            "DELETE",
            f"repos/{self.repo}/git/refs/tags/{quote(tag, safe='')}",
        )

    def create_draft(self, request: RollingReleaseRequest, notes: str) -> None:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8") as stream:
            stream.write(notes)
            stream.flush()
            self._run(
                "gh",
                "release",
                "create",
                request.tag,
                "--draft",
                "--prerelease",
                "--latest=false",
                "--target",
                request.source_sha,
                "--title",
                request.title,
                "--notes-file",
                stream.name,
                "--repo",
                self.repo,
            )

    def upload_asset(self, tag: str, asset: Path) -> None:
        self._run("gh", "release", "upload", tag, str(asset), "--repo", self.repo)

    def publish_draft(self, tag: str) -> None:
        self._run(
            "gh",
            "release",
            "edit",
            tag,
            "--draft=false",
            "--prerelease",
            "--latest=false",
            "--repo",
            self.repo,
        )
