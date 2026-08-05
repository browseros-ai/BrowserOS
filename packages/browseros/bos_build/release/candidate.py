#!/usr/bin/env python3
"""Immutable browser release candidate lifecycle."""

import json
import re
import subprocess
import tempfile
from dataclasses import asdict, dataclass, replace
from pathlib import Path
from typing import Mapping, Protocol, Sequence

from .components import (
    AllocationRecord,
    component_by_id,
    components_for_candidate,
    read_component_version,
    resolve_candidate_versions,
    stamp_component,
)
from .github import (
    create_pull_request,
    edit_pull_request_body,
    list_pull_requests,
    merge_pull_request,
)
from .lane import LaneGate


_SCHEMA = "browseros-release-candidate-v1"
_MARKER_RE = re.compile(r"<!-- browseros-release-candidate-v1\n(.*?)\n-->", re.DOTALL)


@dataclass(frozen=True)
class CandidateRequest:
    """Inputs frozen by a full browser release dispatch."""

    product: str
    parent_sha: str
    default_branch: str
    dispatch_ref: str


@dataclass(frozen=True)
class PullRequestState:
    """Pull request fields required by candidate reconciliation."""

    number: int
    url: str
    state: str
    head_sha: str
    head_branch: str
    base_branch: str
    mergeable: bool
    merge_sha: str = ""


@dataclass(frozen=True)
class CandidateRecord:
    """Portable identity and lifecycle state for one candidate."""

    product: str
    parent_sha: str
    candidate_sha: str
    default_branch: str
    branch: str
    browser_version: str
    component_versions: Mapping[str, str]
    pull_request_number: int
    pull_request_url: str
    state: str = "open"
    merge_sha: str = ""
    schema: str = _SCHEMA

    def to_dict(self) -> dict[str, object]:
        return dict(asdict(self))

    def to_json(self) -> str:
        return json.dumps(self.to_dict(), indent=2, sort_keys=True) + "\n"

    @classmethod
    def from_dict(cls, document: Mapping[str, object]) -> "CandidateRecord":
        if document.get("schema") != _SCHEMA:
            raise ValueError("Unsupported candidate record schema")
        component_versions = document.get("component_versions")
        if not isinstance(component_versions, dict) or not all(
            isinstance(key, str) and isinstance(value, str)
            for key, value in component_versions.items()
        ):
            raise ValueError("Candidate component_versions must be a string map")
        values = {
            "product": document.get("product"),
            "parent_sha": document.get("parent_sha"),
            "candidate_sha": document.get("candidate_sha"),
            "default_branch": document.get("default_branch"),
            "branch": document.get("branch"),
            "browser_version": document.get("browser_version"),
            "pull_request_url": document.get("pull_request_url"),
            "state": document.get("state", "open"),
            "merge_sha": document.get("merge_sha", ""),
        }
        if not all(isinstance(value, str) for value in values.values()):
            raise ValueError("Candidate record contains invalid string fields")
        number = document.get("pull_request_number")
        if not isinstance(number, int):
            raise ValueError("Candidate pull_request_number must be an integer")
        return cls(
            **values,
            component_versions=component_versions,
            pull_request_number=number,
        )

    @classmethod
    def from_path(cls, path: Path) -> "CandidateRecord":
        document = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(document, dict):
            raise ValueError("Candidate record must be a JSON object")
        return cls.from_dict(document)


class CandidateBackend(Protocol):
    def current_sha(self) -> str: ...

    def is_clean(self) -> bool: ...

    def find_candidate(self, product: str, parent_sha: str) -> CandidateRecord | None: ...

    def discover_allocations(self, product: str) -> Sequence[AllocationRecord]: ...

    def read_committed_versions(self, product: str) -> Mapping[str, str]: ...

    def read_browser_version(self) -> str: ...

    def create_candidate(
        self,
        request: CandidateRequest,
        branch: str,
        versions: Mapping[str, str],
        browser_version: str,
    ) -> CandidateRecord: ...

    def inspect_pull_request(self, number: int) -> PullRequestState: ...

    def merge_pull_request(self, number: int) -> str: ...

    def default_branch_contains_versions(self, record: CandidateRecord) -> bool: ...


def candidate_branch(product: str, parent_sha: str) -> str:
    """Return the deterministic branch for a product and frozen parent."""
    return f"bot/release-{product}-{parent_sha[:12]}"


def _validate_sha(value: str, name: str) -> None:
    if not re.fullmatch(r"[0-9a-fA-F]{40}", value):
        raise ValueError(f"{name} must be a full commit SHA")


def _validate_recovered(
    record: CandidateRecord, request: CandidateRequest, branch: str
) -> None:
    if record.product != request.product or record.parent_sha != request.parent_sha:
        raise ValueError("Recovered candidate identity does not match the request")
    if record.default_branch != request.default_branch or record.branch != branch:
        raise ValueError("Recovered candidate branch metadata does not match")
    _validate_sha(record.candidate_sha, "candidate_sha")
    expected = {
        *(spec.id for spec in components_for_candidate(request.product)),
        "claw-onboard",
    }
    if set(record.component_versions) != expected:
        raise ValueError("Recovered candidate component set is incomplete")


def ensure_candidate(
    request: CandidateRequest, backend: CandidateBackend
) -> CandidateRecord:
    """Create or recover one immutable browser release candidate."""
    _validate_sha(request.parent_sha, "parent_sha")
    if request.dispatch_ref != request.default_branch:
        raise ValueError(
            f"New candidates must be dispatched from the default branch {request.default_branch}"
        )
    if backend.current_sha() != request.parent_sha:
        raise ValueError("Checkout does not match the frozen parent SHA")
    if not backend.is_clean():
        raise ValueError("Candidate creation requires a clean checkout")

    branch = candidate_branch(request.product, request.parent_sha)
    existing = backend.find_candidate(request.product, request.parent_sha)
    if existing is not None:
        _validate_recovered(existing, request, branch)
        return existing

    committed_versions = backend.read_committed_versions(request.product)
    versions = resolve_candidate_versions(
        product_id=request.product,
        committed_versions=committed_versions,
        allocations=backend.discover_allocations(request.product),
        candidate_id=branch,
    )
    versions["claw-onboard"] = committed_versions["claw-onboard"]
    created = backend.create_candidate(
        request,
        branch,
        versions,
        backend.read_browser_version(),
    )
    _validate_recovered(created, request, branch)
    return created


def merge_candidate(
    record: CandidateRecord,
    gate: Mapping[str, object],
    backend: CandidateBackend,
) -> CandidateRecord:
    """Merge an unchanged candidate after its complete lane gate passes."""
    evidence = LaneGate.from_dict(gate)
    expected = {
        "product": record.product,
        "parent_sha": record.parent_sha,
        "candidate_sha": record.candidate_sha,
        "browser_version": record.browser_version,
        "component_versions": dict(record.component_versions),
    }
    for field, value in expected.items():
        if getattr(evidence, field) != value:
            raise ValueError(f"Candidate merge gate {field} does not match")
    pull_request = backend.inspect_pull_request(record.pull_request_number)
    if pull_request.state == "merged":
        if not pull_request.merge_sha:
            raise ValueError("Merged candidate pull request has no merge commit")
        return replace(record, state="merged", merge_sha=pull_request.merge_sha)
    if pull_request.state != "open":
        raise ValueError("Candidate pull request is not open")
    if pull_request.head_sha != record.candidate_sha:
        raise ValueError("Candidate pull request head changed")
    if pull_request.head_branch != record.branch:
        raise ValueError("Candidate pull request branch changed")
    if pull_request.base_branch != record.default_branch:
        raise ValueError("Candidate pull request base changed")
    if not pull_request.mergeable:
        raise ValueError("Candidate pull request is not mergeable")
    if backend.default_branch_contains_versions(record):
        raise ValueError("Candidate component versions were superseded on the default branch")
    merge_sha = backend.merge_pull_request(record.pull_request_number)
    _validate_sha(merge_sha, "merge_sha")
    return replace(record, state="merged", merge_sha=merge_sha)


def _candidate_body(record: CandidateRecord) -> str:
    return (
        f"Browser release candidate for `{record.product}` from `{record.parent_sha}`.\n\n"
        f"<!-- browseros-release-candidate-v1\n"
        f"{json.dumps(record.to_dict(), sort_keys=True)}\n"
        f"-->"
    )


def _record_from_body(body: str) -> CandidateRecord | None:
    match = _MARKER_RE.search(body)
    if match is None:
        return None
    document = json.loads(match.group(1))
    if not isinstance(document, dict):
        raise ValueError("Candidate pull request metadata must be an object")
    return CandidateRecord.from_dict(document)


class GitHubCandidateBackend:
    """Git and GitHub implementation of the candidate backend."""

    def __init__(
        self,
        repo_root: Path,
        repo: str,
        default_branch: str,
        remote: str = "origin",
    ) -> None:
        self.repo_root = repo_root.resolve()
        self.repo = repo
        self.default_branch = default_branch
        self.remote = remote

    def _git(self, *args: str, cwd: Path | None = None) -> str:
        result = subprocess.run(
            ["git", *args],
            cwd=cwd or self.repo_root,
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout.strip()

    def current_sha(self) -> str:
        return self._git("rev-parse", "HEAD")

    def is_clean(self) -> bool:
        return not self._git("status", "--porcelain")

    def find_candidate(self, product: str, parent_sha: str) -> CandidateRecord | None:
        branch = candidate_branch(product, parent_sha)
        matches = []
        for pull_request in list_pull_requests(
            self.repo, state="all", head=branch
        ):
            body = pull_request.get("body")
            record = _record_from_body(body) if isinstance(body, str) else None
            if record is None:
                continue
            if record.product != product or record.parent_sha != parent_sha:
                continue
            head_sha = pull_request.get("headRefOid")
            if head_sha != record.candidate_sha:
                raise ValueError("Candidate branch head no longer matches its record")
            if pull_request.get("baseRefName") != record.default_branch:
                raise ValueError("Candidate pull request base no longer matches its record")
            state = "merged" if pull_request.get("mergedAt") else "open"
            if pull_request.get("state") == "CLOSED" and state != "merged":
                state = "closed"
            merge_commit = pull_request.get("mergeCommit")
            merge_sha = ""
            if isinstance(merge_commit, dict):
                value = merge_commit.get("oid")
                merge_sha = value if isinstance(value, str) else ""
            number = pull_request.get("number")
            url = pull_request.get("url")
            if not isinstance(number, int) or not isinstance(url, str):
                raise ValueError("Candidate pull request is missing its identity")
            matches.append(
                replace(
                    record,
                    pull_request_number=number,
                    pull_request_url=url,
                    state=state,
                    merge_sha=merge_sha,
                )
            )
        if len(matches) > 1:
            raise ValueError(f"Multiple candidate pull requests found for {branch}")
        return matches[0] if matches else None

    def discover_allocations(self, product: str) -> Sequence[AllocationRecord]:
        specs = components_for_candidate(product)
        allocations: list[AllocationRecord] = []
        for tag in self._git("tag", "--list").splitlines():
            for spec in specs:
                prefixes = (spec.tag_prefix, *spec.legacy_tag_prefixes)
                prefix = next((value for value in prefixes if tag.startswith(value)), "")
                if not prefix:
                    continue
                version = tag[len(prefix) :]
                try:
                    target = self._git("rev-list", "-n", "1", tag)
                    tag_type = self._git("cat-file", "-t", f"refs/tags/{tag}")
                    allocations.append(
                        AllocationRecord(
                            component=spec.id,
                            version=version,
                            kind="tag",
                            source_sha=target,
                            reusable=tag_type == "tag" and prefix == spec.tag_prefix,
                        )
                    )
                except (ValueError, subprocess.CalledProcessError):
                    continue

        result = subprocess.run(
            [
                "gh",
                "release",
                "list",
                "--repo",
                self.repo,
                "--limit",
                "1000",
                "--json",
                "tagName,isDraft,targetCommitish",
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        releases = json.loads(result.stdout or "[]")
        if not isinstance(releases, list):
            raise RuntimeError("GitHub release response must be an array")
        for release in releases:
            if not isinstance(release, dict):
                continue
            tag = release.get("tagName")
            if not isinstance(tag, str):
                continue
            for spec in specs:
                if not tag.startswith(spec.tag_prefix):
                    continue
                allocations.append(
                    AllocationRecord(
                        component=spec.id,
                        version=tag[len(spec.tag_prefix) :],
                        kind="release",
                        source_sha=str(release.get("targetCommitish", "")),
                        reusable=release.get("isDraft") is True,
                    )
                )

        for pull_request in list_pull_requests(self.repo, state="open"):
            body = pull_request.get("body")
            record = _record_from_body(body) if isinstance(body, str) else None
            if record is None:
                continue
            for component, version in record.component_versions.items():
                allocations.append(
                    AllocationRecord(
                        component=component,
                        version=version,
                        kind="candidate",
                        source_sha=record.candidate_sha,
                        candidate_id=record.branch,
                    )
                )
        return tuple(allocations)

    def read_committed_versions(self, product: str) -> Mapping[str, str]:
        versions = {
            spec.id: read_component_version(self.repo_root, spec.id)
            for spec in components_for_candidate(product)
        }
        versions["claw-onboard"] = read_component_version(
            self.repo_root, "claw-onboard"
        )
        return versions

    def read_browser_version(self) -> str:
        path = self.repo_root / "packages/browseros/resources/BROWSEROS_VERSION"
        version = path.read_text(encoding="utf-8").strip()
        if not version:
            raise ValueError(f"Browser version is empty: {path}")
        return version

    def create_candidate(
        self,
        request: CandidateRequest,
        branch: str,
        versions: Mapping[str, str],
        browser_version: str,
    ) -> CandidateRecord:
        with tempfile.TemporaryDirectory(prefix="browseros-candidate-") as temp_dir:
            worktree = Path(temp_dir) / "repo"
            self._git("worktree", "add", "--detach", str(worktree), request.parent_sha)
            try:
                self._git("switch", "-c", branch, cwd=worktree)
                changed: set[Path] = set()
                versioned_components = {
                    spec.id for spec in components_for_candidate(request.product)
                }
                for component, version in versions.items():
                    if component not in versioned_components:
                        continue
                    changed.update(stamp_component(worktree, component, version))
                relative = sorted(str(path.relative_to(worktree)) for path in changed)
                self._git("add", "--", *relative, cwd=worktree)
                staged = set(
                    self._git("diff", "--cached", "--name-only", cwd=worktree).splitlines()
                )
                if staged != set(relative):
                    raise ValueError("Candidate commit contains unexpected files")
                self._git(
                    "-c",
                    "user.name=BrowserOS CI",
                    "-c",
                    "user.email=ci@browseros.com",
                    "commit",
                    "-m",
                    f"chore(release): prepare {request.product} browser candidate",
                    cwd=worktree,
                )
                candidate_sha = self._git("rev-parse", "HEAD", cwd=worktree)
                self._git(
                    "push",
                    self.remote,
                    f"HEAD:refs/heads/{branch}",
                    cwd=worktree,
                )
            finally:
                self._git("worktree", "remove", "--force", str(worktree))

        provisional = CandidateRecord(
            product=request.product,
            parent_sha=request.parent_sha,
            candidate_sha=candidate_sha,
            default_branch=request.default_branch,
            branch=branch,
            browser_version=browser_version,
            component_versions=dict(versions),
            pull_request_number=0,
            pull_request_url="",
        )
        url = create_pull_request(
            repo=self.repo,
            head=branch,
            base=request.default_branch,
            title=f"chore(release): prepare {request.product} browser candidate",
            body=_candidate_body(provisional),
        )
        match = re.search(r"/(\d+)$", url)
        if match is None:
            raise RuntimeError(f"Could not parse pull request number from {url}")
        number = int(match.group(1))
        record = replace(
            provisional,
            pull_request_number=number,
            pull_request_url=url,
        )
        edit_pull_request_body(
            repo=self.repo,
            number=number,
            body=_candidate_body(record),
        )
        return record

    def inspect_pull_request(self, number: int) -> PullRequestState:
        result = subprocess.run(
            [
                "gh",
                "pr",
                "view",
                str(number),
                "--repo",
                self.repo,
                "--json",
                "number,url,state,headRefOid,headRefName,baseRefName,mergeable,mergedAt,mergeCommit",
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        document = json.loads(result.stdout)
        merged = bool(document.get("mergedAt"))
        state = "merged" if merged else str(document.get("state", "")).lower()
        merge_commit = document.get("mergeCommit")
        merge_sha = ""
        if isinstance(merge_commit, dict):
            value = merge_commit.get("oid")
            merge_sha = value if isinstance(value, str) else ""
        return PullRequestState(
            number=int(document["number"]),
            url=str(document["url"]),
            state=state,
            head_sha=str(document["headRefOid"]),
            head_branch=str(document["headRefName"]),
            base_branch=str(document["baseRefName"]),
            mergeable=document.get("mergeable") == "MERGEABLE",
            merge_sha=merge_sha,
        )

    def merge_pull_request(self, number: int) -> str:
        return merge_pull_request(self.repo, number)

    def _version_at_ref(self, component: str, ref: str) -> str:
        spec = component_by_id(component)
        content = self._git("show", f"{ref}:{spec.manifest_path}")
        if spec.manifest_path.suffix == ".json":
            document = json.loads(content)
            return str(document["version"])
        match = re.search(
            r'(?ms)^\[package\]\s*$.*?^version\s*=\s*"([^"]+)"', content
        )
        if match is None:
            raise ValueError(f"Missing version in {spec.manifest_path} at {ref}")
        return match.group(1)

    def default_branch_contains_versions(self, record: CandidateRecord) -> bool:
        default_ref = f"{self.remote}/{record.default_branch}"
        for component in record.component_versions:
            if self._version_at_ref(component, default_ref) != self._version_at_ref(
                component, record.parent_sha
            ):
                return True
        return False
