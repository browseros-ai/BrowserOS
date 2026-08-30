#!/usr/bin/env python3
"""Standalone component release resolution against shared allocation state."""

import json
import re
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Protocol, Sequence

from .candidate import GitHubCandidateBackend, candidate_record_from_pull_request
from .components import (
    AllocationRecord,
    component_by_id,
    component_version_from_package,
    normalize_component_version,
    resolve_standalone_version,
)
from .github import list_github_releases, list_pull_requests
from .r2_allocations import discover_r2_component_allocation
from .suite import suite_allocation_record_from_pull_request


@dataclass(frozen=True)
class TagState:
    """Resolved local tag identity."""

    target_sha: str
    annotated: bool


@dataclass(frozen=True)
class StandaloneReleaseRequest:
    """Inputs controlling one standalone component allocation."""

    component: str
    event_name: str
    default_branch: str
    ref_name: str = ""
    requested_version: str = ""
    release_ref: str = ""


@dataclass(frozen=True)
class StandaloneReleaseRecord:
    """Resolved immutable component release identity."""

    component: str
    version: str
    tag: str
    release_sha: str
    previous_tag: str
    reservation: str

    def to_dict(self) -> dict[str, str]:
        return asdict(self)

    def to_json(self) -> str:
        return json.dumps(self.to_dict(), indent=2, sort_keys=True) + "\n"

    def summary(self) -> str:
        spec = component_by_id(self.component)
        return (
            f"{spec.display_name} release:\n"
            f"- Version: `{self.version}`\n"
            f"- Tag: `{self.tag}`\n"
            f"- Release commit: `{self.release_sha}`\n"
            f"- Reservation: `{self.reservation}`\n"
        )


class ComponentReleaseOperations(Protocol):
    remote: str

    def sync(self, default_branch: str) -> None: ...

    def resolve_commit(self, ref: str) -> str: ...

    def read_version(self, component: str, ref: str) -> str: ...

    def tag_state(self, tag: str) -> TagState | None: ...

    def is_default_branch_ancestor(self, sha: str, default_branch: str) -> bool: ...

    def allocations(self, component: str) -> Sequence[AllocationRecord]: ...

    def resource_allocation(
        self, component: str, version: str, source_sha: str
    ) -> AllocationRecord | None: ...


def _version_key(component: str, version: str) -> tuple[int, ...]:
    return tuple(
        int(part) for part in normalize_component_version(component, version).split(".")
    )


def resolve_standalone_release(
    request: StandaloneReleaseRequest,
    operations: ComponentReleaseOperations,
    additional_allocations: Sequence[AllocationRecord] = (),
) -> StandaloneReleaseRecord:
    """Resolve one source-bound standalone component release."""
    if request.event_name not in {
        "push",
        "schedule",
        "workflow_dispatch",
        "workflow_call",
    }:
        raise ValueError(f"Unsupported release event: {request.event_name}")
    if not request.default_branch:
        raise ValueError("default_branch is required")
    spec = component_by_id(request.component)
    operations.sync(request.default_branch)

    if request.event_name == "push":
        if not request.ref_name.startswith(spec.tag_prefix):
            raise ValueError(
                f"Expected {spec.display_name} tag like {spec.tag_prefix}VERSION"
            )
        version = normalize_component_version(
            request.component, request.ref_name[len(spec.tag_prefix) :]
        )
        state = operations.tag_state(request.ref_name)
        if state is None:
            raise ValueError(f"Tag does not exist: {request.ref_name}")
        if not state.annotated:
            raise ValueError(f"Tag {request.ref_name} must be annotated")
        release_sha = state.target_sha
        requested = version
    else:
        ref = request.release_ref or f"{operations.remote}/{request.default_branch}"
        release_sha = operations.resolve_commit(ref)
        requested = request.requested_version
        version = requested or operations.read_version(request.component, release_sha)

    if not operations.is_default_branch_ancestor(release_sha, request.default_branch):
        raise ValueError(
            f"Release commit {release_sha} is not reachable from "
            f"{operations.remote}/{request.default_branch}"
        )

    allocations = [
        *operations.allocations(request.component),
        *additional_allocations,
    ]
    probed_versions = set()
    while True:
        resolved = resolve_standalone_version(
            component_id=request.component,
            committed_version=version,
            allocations=allocations,
            requested_version=requested,
            source_sha=release_sha,
        )
        if resolved in probed_versions:
            break
        probed_versions.add(resolved)
        resource = operations.resource_allocation(
            request.component, resolved, release_sha
        )
        if resource is None:
            break
        allocations.append(resource)
    tag = f"{spec.tag_prefix}{resolved}"
    tag_state = operations.tag_state(tag)
    if tag_state is not None:
        if not tag_state.annotated:
            raise ValueError(f"Tag {tag} must be annotated")
        if tag_state.target_sha != release_sha:
            raise ValueError(f"{tag} is already allocated to a different source")
        reservation = "tag"
    else:
        reusable = any(
            record.kind == "release"
            and record.reusable
            and record.reference == tag
            and record.source_sha == release_sha
            for record in allocations
        )
        reservation = "reuse" if reusable else "create"
    if request.event_name == "push" and reservation != "tag":
        raise ValueError(f"Tag does not exist: {tag}")

    earlier_tags = [
        record
        for record in allocations
        if record.kind == "tag"
        and record.reference
        and _version_key(request.component, record.version)
        < _version_key(request.component, resolved)
    ]
    previous_tag = ""
    if earlier_tags:
        previous_tag = max(
            earlier_tags,
            key=lambda record: _version_key(request.component, record.version),
        ).reference
    return StandaloneReleaseRecord(
        component=request.component,
        version=resolved,
        tag=tag,
        release_sha=release_sha,
        previous_tag=previous_tag,
        reservation=reservation,
    )


class GitComponentReleaseOperations:
    """Git and GitHub allocation discovery for a single component."""

    def __init__(
        self,
        repo_root: Path,
        repo: str,
        remote: str = "origin",
        *,
        r2_client=None,
        r2_bucket: str = "",
    ) -> None:
        self.repo_root = repo_root.resolve()
        self.repo = repo
        self.remote = remote
        self.default_branch = ""
        self.r2_client = r2_client
        self.r2_bucket = r2_bucket

    def _git(self, *args: str, check: bool = True) -> str:
        result = subprocess.run(
            ["git", *args],
            cwd=self.repo_root,
            capture_output=True,
            text=True,
            check=check,
        )
        return result.stdout.strip()

    def sync(self, default_branch: str) -> None:
        self.default_branch = default_branch
        self._git(
            "fetch",
            self.remote,
            f"{default_branch}:refs/remotes/{self.remote}/{default_branch}",
            "--no-tags",
        )
        self._git("fetch", "--force", self.remote, "--tags", "--prune")

    def resolve_commit(self, ref: str) -> str:
        candidates = (ref, f"{self.remote}/{ref}")
        for candidate in candidates:
            result = subprocess.run(
                ["git", "rev-parse", f"{candidate}^{{commit}}"],
                cwd=self.repo_root,
                capture_output=True,
                text=True,
            )
            if result.returncode == 0:
                return result.stdout.strip()
        raise ValueError(f"Could not resolve release ref: {ref}")

    def read_version(self, component: str, ref: str) -> str:
        spec = component_by_id(component)
        content = self._git("show", f"{ref}:{spec.manifest_path.as_posix()}")
        if spec.manifest_path.suffix == ".json":
            document = json.loads(content)
            value = document.get("version")
        else:
            match = re.search(
                r'(?ms)^\[package\]\s*$.*?^version\s*=\s*"([^"]+)"', content
            )
            value = match.group(1) if match else None
        if not isinstance(value, str):
            raise ValueError(f"Missing version in {spec.manifest_path} at {ref}")
        return component_version_from_package(component, value)

    def tag_state(self, tag: str) -> TagState | None:
        result = subprocess.run(
            ["git", "cat-file", "-t", f"refs/tags/{tag}"],
            cwd=self.repo_root,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            return None
        target = self._git("rev-list", "-n", "1", tag)
        return TagState(target_sha=target, annotated=result.stdout.strip() == "tag")

    def is_default_branch_ancestor(self, sha: str, default_branch: str) -> bool:
        result = subprocess.run(
            [
                "git",
                "merge-base",
                "--is-ancestor",
                sha,
                f"{self.remote}/{default_branch}",
            ],
            cwd=self.repo_root,
        )
        return result.returncode == 0

    def allocations(self, component: str) -> Sequence[AllocationRecord]:
        from .suite import GitHubSuiteBackend, merge_suite_component_allocations

        spec = component_by_id(component)
        allocations = []
        prefixes = (spec.tag_prefix, *spec.legacy_tag_prefixes)
        for tag in self._git("tag", "--list").splitlines():
            prefix = next((value for value in prefixes if tag.startswith(value)), "")
            if not prefix:
                continue
            try:
                version = normalize_component_version(component, tag[len(prefix) :])
            except ValueError:
                continue
            state = self.tag_state(tag)
            if state is None:
                continue
            allocations.append(
                AllocationRecord(
                    component=component,
                    version=version,
                    kind="tag",
                    source_sha=state.target_sha,
                    reference=tag,
                    reusable=state.annotated and prefix == spec.tag_prefix,
                    public=state.annotated,
                )
            )

        for release in list_github_releases(self.repo):
            tag = release.get("tagName")
            if not isinstance(tag, str) or not tag.startswith(spec.tag_prefix):
                continue
            try:
                version = normalize_component_version(
                    component, tag[len(spec.tag_prefix) :]
                )
            except ValueError:
                continue
            tag_identity = self.tag_state(tag)
            release_source_sha = (
                tag_identity.target_sha
                if tag_identity is not None
                else str(release.get("targetCommitish", ""))
            )
            allocations.append(
                AllocationRecord(
                    component=component,
                    version=version,
                    kind="release",
                    source_sha=release_source_sha,
                    reference=tag,
                    reusable=release.get("isDraft") is True,
                    public=release.get("isDraft") is False,
                )
            )

        for pull_request in list_pull_requests(self.repo, state="all"):
            record = candidate_record_from_pull_request(pull_request, self.repo)
            suite_reservation = False
            candidate_is_open = pull_request.get(
                "state"
            ) == "OPEN" and not pull_request.get("mergedAt")
            if (
                record is not None
                and component in record.component_versions
                and candidate_is_open
            ):
                GitHubCandidateBackend(
                    self.repo_root,
                    self.repo,
                    record.default_branch,
                    self.remote,
                ).validate_candidate(record)
                version = record.component_versions[component]
                source_sha = record.candidate_sha
                candidate_id = record.branch
            else:
                suite = suite_allocation_record_from_pull_request(
                    pull_request, self.repo
                )
                if suite is None or component not in suite.component_versions:
                    continue
                # Suite reservations are validated by their immutable same-repo
                # PR marker. Their live state head may advance with snapshots, so
                # the allocation remains bound to the frozen artifact source.
                version = suite.component_versions[component]
                source_sha = suite.source_sha
                candidate_id = suite.branch
                reference = component_by_id(component).tag_prefix + version
                reusable = suite.state == "open"
                suite_reservation = True
            if (
                record is not None
                and component in record.component_versions
                and candidate_is_open
            ):
                reference = candidate_id
                reusable = False
            allocations.append(
                AllocationRecord(
                    component=component,
                    version=version,
                    kind="candidate",
                    source_sha=source_sha,
                    candidate_id=candidate_id,
                    reference=reference,
                    reusable=reusable,
                    reuse_forbidden=suite_reservation and not reusable,
                )
            )

        # sync() records the resolved default branch before allocation. The
        # remote transaction branch is the durable ledger during the interval
        # after its reservation push and before GitHub creates the draft PR.
        if self.default_branch:
            suite_backend = GitHubSuiteBackend(
                self.repo_root,
                self.repo,
                self.default_branch,
                self.remote,
            )
            return merge_suite_component_allocations(
                allocations,
                suite_backend.discover_branch_reservations(),
                (component,),
            )
        return tuple(allocations)

    def resource_allocation(
        self, component: str, version: str, source_sha: str
    ) -> AllocationRecord | None:
        if self.r2_client is None:
            return None
        return discover_r2_component_allocation(
            self.r2_client,
            self.r2_bucket,
            component,
            version,
            source_sha,
        )
