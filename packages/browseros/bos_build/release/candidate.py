#!/usr/bin/env python3
"""Immutable browser release candidate lifecycle."""

import json
import re
import subprocess
import tempfile
from dataclasses import asdict, dataclass, replace
from pathlib import Path
from typing import Mapping, Protocol, Sequence

from ..lib.versions import load_semantic_version
from ..products.resource_sources import source_resources_for_product
from .components import (
    AllocationRecord,
    component_by_id,
    component_version_from_package,
    components_for_candidate,
    read_component_version,
    resolve_candidate_versions,
    stamp_component,
)
from .github import (
    create_pull_request,
    edit_pull_request_body,
    list_github_releases,
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

    def summary(self) -> str:
        versions = ", ".join(
            f"{name}={version}"
            for name, version in sorted(self.component_versions.items())
        )
        merge = f"\n- Merge commit: `{self.merge_sha}`" if self.merge_sha else ""
        return (
            "## Browser release candidate\n\n"
            f"- Product: `{self.product}`\n"
            f"- Parent: `{self.parent_sha}`\n"
            f"- Candidate: `{self.candidate_sha}`\n"
            f"- Candidate PR: [#{self.pull_request_number}]({self.pull_request_url})\n"
            f"- Browser version: `{self.browser_version}`\n"
            f"- Component versions: {versions}\n"
            f"- State: `{self.state}`{merge}\n"
        )

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

    def find_candidate(
        self, product: str, parent_sha: str
    ) -> CandidateRecord | None: ...

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

    def merge_pull_request(self, number: int, expected_head_sha: str) -> str: ...

    def default_branch_contains_versions(self, record: CandidateRecord) -> bool: ...

    def merge_commit_matches_candidate(
        self, record: CandidateRecord, merge_sha: str
    ) -> bool: ...


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
    source = source_resources_for_product(request.product)
    expected = {
        *(spec.id for spec in components_for_candidate(request.product)),
        source.onboarding_component,
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
    # Onboarding releases independently, so a browser candidate freezes the
    # committed product-owned version without allocating a new app version.
    onboarding_component = source_resources_for_product(
        request.product
    ).onboarding_component
    versions[onboarding_component] = committed_versions[onboarding_component]
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
    if pull_request.head_sha != record.candidate_sha:
        raise ValueError("Candidate pull request head changed")
    if pull_request.head_branch != record.branch:
        raise ValueError("Candidate pull request branch changed")
    if pull_request.base_branch != record.default_branch:
        raise ValueError("Candidate pull request base changed")
    if pull_request.state == "merged":
        if not pull_request.merge_sha:
            raise ValueError("Merged candidate pull request has no merge commit")
        if not backend.merge_commit_matches_candidate(record, pull_request.merge_sha):
            raise ValueError("Merged candidate commit does not match the candidate")
        return replace(record, state="merged", merge_sha=pull_request.merge_sha)
    if pull_request.state != "open":
        raise ValueError("Candidate pull request is not open")
    if not pull_request.mergeable:
        raise ValueError("Candidate pull request is not mergeable")
    if backend.default_branch_contains_versions(record):
        raise ValueError(
            "Candidate component versions were superseded on the default branch"
        )
    merge_sha = backend.merge_pull_request(
        record.pull_request_number, record.candidate_sha
    )
    _validate_sha(merge_sha, "merge_sha")
    return replace(record, state="merged", merge_sha=merge_sha)


def _candidate_body(record: CandidateRecord) -> str:
    return (
        f"Browser release candidate for `{record.product}` from `{record.parent_sha}`.\n\n"
        f"<!-- browseros-release-candidate-v1\n"
        f"{json.dumps(record.to_dict(), sort_keys=True)}\n"
        f"-->"
    )


def candidate_record_from_body(body: str) -> CandidateRecord | None:
    """Read candidate metadata embedded in a pull request body."""
    match = _MARKER_RE.search(body)
    if match is None:
        return None
    document = json.loads(match.group(1))
    if not isinstance(document, dict):
        raise ValueError("Candidate pull request metadata must be an object")
    return CandidateRecord.from_dict(document)


def candidate_record_from_pull_request(
    pull_request: Mapping[str, object], repo: str
) -> CandidateRecord | None:
    """Read candidate metadata only from a canonical repository branch."""
    repository = pull_request.get("headRepository")
    if (
        pull_request.get("isCrossRepository") is not False
        or not isinstance(repository, dict)
        or str(repository.get("nameWithOwner", "")).lower() != repo.lower()
    ):
        return None
    body = pull_request.get("body")
    if not isinstance(body, str):
        return None
    try:
        record = candidate_record_from_body(body)
        if record is None:
            return None
        _validate_sha(record.parent_sha, "candidate parent SHA")
        _validate_sha(record.candidate_sha, "candidate SHA")
        components_for_candidate(record.product)
    except (json.JSONDecodeError, TypeError, ValueError):
        return None
    branch = candidate_branch(record.product, record.parent_sha)
    if (
        record.branch != branch
        or pull_request.get("headRefName") != branch
        or pull_request.get("headRefOid") != record.candidate_sha
        or pull_request.get("baseRefName") != record.default_branch
    ):
        return None
    return record


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

    def _stage_candidate_versions(
        self,
        worktree: Path,
        product: str,
        versions: Mapping[str, str],
    ) -> None:
        changed: set[Path] = set()
        versioned_components = {spec.id for spec in components_for_candidate(product)}
        for component, version in versions.items():
            if component in versioned_components:
                changed.update(stamp_component(worktree, component, version))
        relative = sorted(str(path.relative_to(worktree)) for path in changed)
        self._git("add", "--", *relative, cwd=worktree)
        staged = set(
            self._git("diff", "--cached", "--name-only", cwd=worktree).splitlines()
        )
        if staged != set(relative):
            raise ValueError("Candidate commit contains unexpected files")

    def _expected_candidate_tree(self, record: CandidateRecord) -> str:
        with tempfile.TemporaryDirectory(
            prefix="browseros-candidate-check-"
        ) as temp_dir:
            worktree = Path(temp_dir) / "repo"
            self._git("worktree", "add", "--detach", str(worktree), record.parent_sha)
            try:
                self._stage_candidate_versions(
                    worktree,
                    record.product,
                    record.component_versions,
                )
                for component, version in record.component_versions.items():
                    if read_component_version(worktree, component) != version:
                        raise ValueError(
                            f"Candidate {component} version does not match its metadata"
                        )
                browser_version = load_semantic_version(worktree / "packages/browseros")
                if browser_version != record.browser_version:
                    raise ValueError(
                        "Candidate browser version does not match its metadata"
                    )
                return self._git("write-tree", cwd=worktree)
            finally:
                self._git("worktree", "remove", "--force", str(worktree))

    def _remote_branch_sha(self, branch: str, *, cwd: Path) -> str | None:
        ref = f"refs/heads/{branch}"
        output = self._git("ls-remote", "--heads", self.remote, ref, cwd=cwd)
        if not output:
            return None
        lines = output.splitlines()
        if len(lines) != 1:
            raise ValueError(f"Remote branch {branch} resolved more than once")
        fields = lines[0].split()
        if len(fields) != 2 or fields[1] != ref:
            raise ValueError(f"Remote branch {branch} returned invalid metadata")
        _validate_sha(fields[0], "remote candidate SHA")
        return fields[0]

    def _validate_remote_candidate(
        self,
        branch: str,
        advertised_sha: str,
        parent_sha: str,
        tree_sha: str,
        *,
        cwd: Path,
    ) -> str:
        ref = f"refs/heads/{branch}"
        self._git("fetch", "--no-tags", self.remote, ref, cwd=cwd)
        candidate_sha = self._git("rev-parse", "FETCH_HEAD", cwd=cwd)
        if candidate_sha != advertised_sha:
            raise ValueError(f"Remote branch {branch} changed while being recovered")
        commit_line = self._git(
            "rev-list", "--parents", "-n", "1", candidate_sha, cwd=cwd
        ).split()
        if commit_line != [candidate_sha, parent_sha]:
            raise ValueError(f"Remote branch {branch} has an unexpected parent")
        if self._git("rev-parse", f"{candidate_sha}^{{tree}}", cwd=cwd) != tree_sha:
            raise ValueError(f"Remote branch {branch} has unexpected candidate content")
        return candidate_sha

    def _publish_candidate_commit(
        self,
        branch: str,
        candidate_sha: str,
        parent_sha: str,
        *,
        cwd: Path,
    ) -> str:
        tree_sha = self._git("rev-parse", f"{candidate_sha}^{{tree}}", cwd=cwd)
        advertised_sha = self._remote_branch_sha(branch, cwd=cwd)
        if advertised_sha is None:
            try:
                self._git(
                    "push",
                    self.remote,
                    f"{candidate_sha}:refs/heads/{branch}",
                    cwd=cwd,
                )
                return candidate_sha
            except subprocess.CalledProcessError:
                advertised_sha = self._remote_branch_sha(branch, cwd=cwd)
                if advertised_sha is None:
                    raise
        return self._validate_remote_candidate(
            branch,
            advertised_sha,
            parent_sha,
            tree_sha,
            cwd=cwd,
        )

    def validate_candidate(self, record: CandidateRecord) -> None:
        """Verify candidate metadata against its immutable Git commit."""
        branch = candidate_branch(record.product, record.parent_sha)
        _validate_recovered(
            record,
            CandidateRequest(
                product=record.product,
                parent_sha=record.parent_sha,
                default_branch=record.default_branch,
                dispatch_ref=record.default_branch,
            ),
            branch,
        )
        advertised_sha = self._remote_branch_sha(branch, cwd=self.repo_root)
        if advertised_sha is None or advertised_sha != record.candidate_sha:
            raise ValueError("Candidate remote branch no longer matches its record")
        self._validate_remote_candidate(
            branch,
            advertised_sha,
            record.parent_sha,
            self._expected_candidate_tree(record),
            cwd=self.repo_root,
        )

    def find_candidate(self, product: str, parent_sha: str) -> CandidateRecord | None:
        branch = candidate_branch(product, parent_sha)
        matches = []
        for pull_request in list_pull_requests(self.repo, state="all", head=branch):
            record = candidate_record_from_pull_request(pull_request, self.repo)
            if record is None:
                continue
            if record.product != product or record.parent_sha != parent_sha:
                continue
            head_sha = pull_request.get("headRefOid")
            if head_sha != record.candidate_sha:
                raise ValueError("Candidate branch head no longer matches its record")
            if pull_request.get("baseRefName") != record.default_branch:
                raise ValueError(
                    "Candidate pull request base no longer matches its record"
                )
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
            self.validate_candidate(record)
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
        from .suite import (
            GitHubSuiteBackend,
            merge_suite_component_allocations,
            suite_allocation_record_from_pull_request,
        )

        specs = components_for_candidate(product)
        allocations: list[AllocationRecord] = []
        for tag in self._git("tag", "--list").splitlines():
            for spec in specs:
                prefixes = (spec.tag_prefix, *spec.legacy_tag_prefixes)
                prefix = next(
                    (value for value in prefixes if tag.startswith(value)), ""
                )
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
                            reference=tag,
                            reusable=tag_type == "tag" and prefix == spec.tag_prefix,
                            public=tag_type == "tag",
                        )
                    )
                except (ValueError, subprocess.CalledProcessError):
                    continue

        for release in list_github_releases(self.repo):
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
                        reference=tag,
                        reusable=release.get("isDraft") is True,
                        public=release.get("isDraft") is False,
                    )
                )

        for pull_request in list_pull_requests(self.repo, state="all"):
            record = candidate_record_from_pull_request(pull_request, self.repo)
            candidate_is_open = pull_request.get(
                "state"
            ) == "OPEN" and not pull_request.get("mergedAt")
            if record is not None and record.product == product and candidate_is_open:
                self.validate_candidate(record)
                versions = record.component_versions
                source_sha = record.candidate_sha
                candidate_id = record.branch
                suite_reservation = False
                reusable = False
            else:
                suite = suite_allocation_record_from_pull_request(
                    pull_request, self.repo
                )
                if suite is None:
                    continue
                product_components = {spec.id for spec in specs}
                versions = {
                    component: version
                    for component, version in suite.component_versions.items()
                    if component in product_components
                }
                source_sha = suite.source_sha
                candidate_id = suite.branch
                suite_reservation = True
                reusable = suite.state == "open"
            for component, version in versions.items():
                if component not in {spec.id for spec in specs}:
                    continue
                allocations.append(
                    AllocationRecord(
                        component=component,
                        version=version,
                        kind="candidate",
                        source_sha=source_sha,
                        candidate_id=candidate_id,
                        reference=(
                            component_by_id(component).tag_prefix + version
                            if suite_reservation
                            else candidate_id
                        ),
                        reusable=reusable,
                        reuse_forbidden=suite_reservation and not reusable,
                    )
                )

        # The reservation branch is pushed before its draft PR is created. If
        # the runner dies between those external writes, candidate allocation
        # still has to burn the suite's exact component versions immediately.
        suite_backend = GitHubSuiteBackend(
            self.repo_root,
            self.repo,
            self.default_branch,
            self.remote,
        )
        return merge_suite_component_allocations(
            allocations,
            suite_backend.discover_branch_reservations(),
            [spec.id for spec in specs],
        )

    def read_committed_versions(self, product: str) -> Mapping[str, str]:
        versions = {
            spec.id: read_component_version(self.repo_root, spec.id)
            for spec in components_for_candidate(product)
        }
        onboarding_component = source_resources_for_product(product).onboarding_component
        versions[onboarding_component] = read_component_version(
            self.repo_root, onboarding_component
        )
        return versions

    def read_browser_version(self) -> str:
        package_root = self.repo_root / "packages/browseros"
        version = load_semantic_version(package_root)
        if not version:
            raise ValueError(f"Browser version is empty: {package_root}")
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
                self._stage_candidate_versions(worktree, request.product, versions)
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
                local_candidate_sha = self._git("rev-parse", "HEAD", cwd=worktree)
                candidate_sha = self._publish_candidate_commit(
                    branch,
                    local_candidate_sha,
                    request.parent_sha,
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

    def merge_pull_request(self, number: int, expected_head_sha: str) -> str:
        return merge_pull_request(
            self.repo,
            number,
            expected_head_sha=expected_head_sha,
        )

    def _version_at_ref(self, component: str, ref: str) -> str:
        spec = component_by_id(component)
        content = self._git("show", f"{ref}:{spec.manifest_path}")
        if spec.manifest_path.suffix == ".json":
            document = json.loads(content)
            return str(document["version"])
        match = re.search(r'(?ms)^\[package\]\s*$.*?^version\s*=\s*"([^"]+)"', content)
        if match is None:
            raise ValueError(f"Missing version in {spec.manifest_path} at {ref}")
        return match.group(1)

    def default_branch_contains_versions(self, record: CandidateRecord) -> bool:
        self._git(
            "fetch",
            "--no-tags",
            self.remote,
            f"refs/heads/{record.default_branch}:refs/remotes/{self.remote}/{record.default_branch}",
        )
        default_ref = f"{self.remote}/{record.default_branch}"
        for spec in components_for_candidate(record.product):
            if self._version_at_ref(spec.id, default_ref) != self._version_at_ref(
                spec.id, record.parent_sha
            ):
                return True
        return False

    def merge_commit_matches_candidate(
        self, record: CandidateRecord, merge_sha: str
    ) -> bool:
        _validate_sha(merge_sha, "merge_sha")
        self._git(
            "fetch",
            "--no-tags",
            self.remote,
            f"refs/heads/{record.default_branch}:refs/remotes/{self.remote}/{record.default_branch}",
        )
        ancestor = subprocess.run(
            [
                "git",
                "merge-base",
                "--is-ancestor",
                merge_sha,
                f"{self.remote}/{record.default_branch}",
            ],
            cwd=self.repo_root,
        )
        if ancestor.returncode != 0:
            return False
        return all(
            component_version_from_package(
                spec.id,
                self._version_at_ref(spec.id, merge_sha),
            )
            == record.component_versions.get(spec.id)
            for spec in components_for_candidate(record.product)
        )
