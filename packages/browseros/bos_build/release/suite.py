#!/usr/bin/env python3
"""Product release transactions, with recovery for saved family reservations.

The suite is the persistence owner above the BrowserOS and BrowserOS neo
products. New nightlies own one product; legacy records own both. Async jobs
add only approved snapshots, and the suite derives the live state head
from GitHub so an interrupted push never requires an atomic PR-body update.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import tempfile
from dataclasses import asdict, dataclass, field, replace
from pathlib import Path
from typing import Literal, Mapping, Protocol, Sequence

from ..lib.versions import load_build_offset, load_semantic_version
from ..scripts.bump_version import bump_version
from .components import (
    AllocationRecord,
    component_by_id,
    component_version_from_package,
    normalize_component_version,
    read_component_version,
    resolve_candidate_versions,
    stamp_component,
)
from .feeds.render import (
    extract_appcast_version,
    extract_manifest_versions,
    parse_dotted_version,
)
from .github import (
    create_pull_request,
    list_pull_requests,
    mark_pull_request_ready,
)


SuiteMode = Literal["nightly", "full"]

_SCHEMA = "browseros-release-suite-v1"
_GATE_SCHEMA = "browseros-release-suite-gate-v1"
_MARKER_RE = re.compile(r"<!-- browseros-release-suite-v1\n(.*?)\n-->", re.DOTALL)
_SHA_RE = re.compile(r"[0-9a-fA-F]{40}")
_TRANSACTION_BRANCH_RE = re.compile(
    r"^bot/release-(nightly|full)-(?:(browseros|browserclaw)-)?([0-9a-f]{12})$"
)
_BROWSER_VERSION_RE = re.compile(
    r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:\.(0|[1-9]\d*))?"
)

SUITE_PRODUCTS = ("browseros", "browserclaw")
SUITE_RELEASE_COMPONENTS = (
    "server",
    "agent",
    "claw-server-rust",
    "browserclaw",
)
# Onboarding SPAs release independently from the family transaction. The suite
# still freezes both committed versions so each product build receives the
# matching immutable resource pin from the same source snapshot.
SUITE_ONBOARDING_COMPONENTS = ("app-onboard", "claw-onboard")
SUITE_COMPONENTS = (*SUITE_RELEASE_COMPONENTS, *SUITE_ONBOARDING_COMPONENTS)
SUITE_STATE_PATHS = (
    "updates/extensions/bundled-manifest.xml",
    "updates/extensions/extensions.alpha.json",
    "updates/extensions/update-manifest.alpha.xml",
    "updates/server/appcast-claw-server.alpha.xml",
    "updates/server/appcast-server.alpha.xml",
)


def suite_products(product: str = "") -> tuple[str, ...]:
    """An absent scope means a saved family transaction, never a guessed product."""
    if not product:
        return SUITE_PRODUCTS
    if product not in SUITE_PRODUCTS:
        raise ValueError(f"Unknown suite product: {product}")
    return (product,)


def suite_components(
    product: str = "", *, release_only: bool = False
) -> tuple[str, ...]:
    from ..products.resource_sources import source_resources_for_product

    components = []
    for selected in suite_products(product):
        resources = source_resources_for_product(selected)
        components.extend((resources.server_component, resources.extension_component))
        if not release_only:
            components.append(resources.onboarding_component)
    return tuple(components)


def suite_state_paths(product: str = "") -> tuple[str, ...]:
    if not product:
        return SUITE_STATE_PATHS
    suite_products(product)
    server = "appcast-server" if product == "browseros" else "appcast-claw-server"
    return (*SUITE_STATE_PATHS[:3], f"updates/server/{server}.alpha.xml")


@dataclass(frozen=True)
class BrowserAllocation:
    """One browser version and build-offset reservation held by a suite PR."""

    transaction_id: str
    browser_version: str
    build_offset: int


@dataclass(frozen=True)
class SuiteRequest:
    """Frozen source and explicit product scope; empty scope recovers legacy families."""

    mode: SuiteMode
    source_sha: str
    default_branch: str
    dispatch_ref: str
    product: str = ""


@dataclass(frozen=True)
class SuitePullRequest:
    """Live pull-request fields used at the exact-head persistence seam."""

    number: int
    url: str
    state: str
    head_sha: str
    head_branch: str
    base_branch: str
    mergeable: bool
    draft: bool
    merge_sha: str = ""


@dataclass(frozen=True)
class SuiteRecord:
    """Product ownership plus immutable build and live persistence identities."""

    transaction_id: str
    mode: str
    source_sha: str
    reservation_sha: str
    state_sha: str
    default_branch: str
    branch: str
    browser_version: str
    build_offset: int
    component_versions: Mapping[str, str]
    pull_request_number: int
    pull_request_url: str
    state: str = "open"
    draft: bool = True
    merge_sha: str = ""
    state_checksums: Mapping[str, str] = field(default_factory=dict)
    schema: str = _SCHEMA
    product: str = ""
    # Derived from the final state's second parent, not stored in the PR marker.
    # Builds always use reservation_sha; this base only owns tracked state.
    state_base_sha: str = ""

    @property
    def products(self) -> tuple[str, ...]:
        return suite_products(self.product)

    @property
    def release_components(self) -> tuple[str, ...]:
        return suite_components(self.product, release_only=True)

    @property
    def state_paths(self) -> tuple[str, ...]:
        return suite_state_paths(self.product)

    def to_dict(self) -> dict[str, object]:
        return dict(asdict(self))

    def to_json(self) -> str:
        return json.dumps(self.to_dict(), indent=2, sort_keys=True) + "\n"

    def _marker_dict(self) -> dict[str, object]:
        # Mutable state and live PR identity come from the same GitHub PR object.
        # Keeping both out removes crash windows between Git push, PR creation,
        # and a second body edit while preserving one immutable reservation marker.
        return {
            "schema": self.schema,
            **({"product": self.product} if self.product else {}),
            "transaction_id": self.transaction_id,
            "mode": self.mode,
            "source_sha": self.source_sha,
            "reservation_sha": self.reservation_sha,
            "default_branch": self.default_branch,
            "branch": self.branch,
            "browser_version": self.browser_version,
            "build_offset": self.build_offset,
            "component_versions": dict(self.component_versions),
        }

    def pull_request_body(self) -> str:
        versions = "\n".join(
            f"- `{name}`: `{version}`"
            for name, version in sorted(self.component_versions.items())
        )
        return (
            f"## BrowserOS {self.product or 'family'} release transaction\n\n"
            f"- Transaction: `{self.transaction_id}`\n"
            f"- Mode: `{self.mode}`\n"
            f"- Frozen artifact source: `{self.source_sha}`\n"
            f"- Reservation commit: `{self.reservation_sha}`\n"
            f"- Browser version: `{self.browser_version}`\n"
            f"- Build offset: `{self.build_offset}`\n\n"
            "### Exact component pins\n\n"
            f"{versions}\n\n"
            "The PR head is the tracked-state commit. It may advance as the suite "
            "reconciles snapshots, but the reservation identity above is immutable.\n\n"
            f"<!-- browseros-release-suite-v1\n"
            f"{json.dumps(self._marker_dict(), sort_keys=True)}\n"
            "-->"
        )

    def summary(self) -> str:
        versions = ", ".join(
            f"{name}={version}"
            for name, version in sorted(self.component_versions.items())
        )
        merge = f"\n- Merge commit: `{self.merge_sha}`" if self.merge_sha else ""
        return (
            f"## BrowserOS {self.product or 'family'} release transaction\n\n"
            f"- Transaction: `{self.transaction_id}`\n"
            f"- Source SHA: `{self.source_sha}`\n"
            f"- State SHA: `{self.state_sha}`\n"
            f"- Browser version: `{self.browser_version}`\n"
            f"- Components: {versions}\n"
            f"- State: `{self.state}`{merge}\n"
        )

    def state_ref(self) -> str:
        """Return the durable ref that makes the transaction history reachable."""
        if self.state == "merged":
            # GitHub retains the synthetic PR-head ref after deleting the source
            # branch. The squash merge is intentionally not used: unrelated
            # default-branch commits may have entered its first-parent tree.
            return f"refs/pull/{self.pull_request_number}/head"
        return self.branch

    @classmethod
    def from_dict(cls, document: Mapping[str, object]) -> "SuiteRecord":
        if document.get("schema") != _SCHEMA:
            raise ValueError("Unsupported suite record schema")
        versions = _string_map(document.get("component_versions"), "component_versions")
        checksums = _string_map(document.get("state_checksums", {}), "state_checksums")
        string_fields = {
            name: document.get(name, "")
            for name in (
                "transaction_id",
                "mode",
                "source_sha",
                "reservation_sha",
                "state_sha",
                "default_branch",
                "branch",
                "browser_version",
                "pull_request_url",
                "state",
                "merge_sha",
                "product",
                "state_base_sha",
            )
        }
        if not all(isinstance(value, str) for value in string_fields.values()):
            raise ValueError("Suite record contains invalid string fields")
        number = document.get("pull_request_number")
        offset = document.get("build_offset")
        draft = document.get("draft", True)
        if not isinstance(number, int) or not isinstance(offset, int):
            raise ValueError("Suite record contains invalid numeric fields")
        if not isinstance(draft, bool):
            raise ValueError("Suite record contains invalid draft state")
        record = cls(
            **string_fields,
            build_offset=offset,
            component_versions=versions,
            pull_request_number=number,
            draft=draft,
            state_checksums=checksums,
        )
        _validate_record(record)
        return record

    @classmethod
    def from_path(cls, path: Path) -> "SuiteRecord":
        document = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(document, dict):
            raise ValueError("Suite record must be a JSON object")
        return cls.from_dict(document)


class SuiteBackend(Protocol):
    def current_sha(self) -> str: ...

    def changed_paths(self) -> Sequence[str]: ...

    def find_transaction(self, request: SuiteRequest) -> SuiteRecord | None: ...

    def discover_allocations(self, product: str = "") -> Sequence[AllocationRecord]: ...

    def discover_browser_allocations(self) -> Sequence[BrowserAllocation]: ...

    def read_committed_versions(self, product: str = "") -> Mapping[str, str]: ...

    def read_browser_version(self) -> str: ...

    def read_build_offset(self) -> int: ...

    def create_transaction(
        self,
        request: SuiteRequest,
        branch: str,
        component_versions: Mapping[str, str],
        browser_version: str,
        build_offset: int,
    ) -> SuiteRecord: ...

    def reconcile_state(self, record: SuiteRecord, state_root: Path) -> SuiteRecord: ...

    def reconcile_product_state(
        self, record: SuiteRecord, state_root: Path, state_base_sha: str
    ) -> SuiteRecord: ...

    def inspect_pull_request(self, number: int) -> SuitePullRequest: ...

    def default_branch_contains_transaction(self, record: SuiteRecord) -> bool: ...

    def mark_pull_request_ready(self, number: int) -> None: ...

    def merge_pull_request(self, number: int, expected_head_sha: str) -> str: ...

    def merge_commit_matches_transaction(
        self, record: SuiteRecord, merge_sha: str
    ) -> bool: ...


def _string_map(value: object, name: str) -> dict[str, str]:
    if not isinstance(value, dict) or not all(
        isinstance(key, str) and isinstance(item, str) for key, item in value.items()
    ):
        raise ValueError(f"Suite {name} must be a string map")
    return dict(value)


def _validate_sha(value: str, name: str) -> None:
    if not _SHA_RE.fullmatch(value):
        raise ValueError(f"{name} must be a full commit SHA")


def transaction_id(mode: str, source_sha: str, product: str = "") -> str:
    """Return the stable retry identity; Actions attempts never participate."""
    if mode not in ("nightly", "full"):
        raise ValueError("mode must be nightly or full")
    _validate_sha(source_sha, "source SHA")
    suite_products(product)
    scope = f"-{product}" if product else ""
    return f"{mode}{scope}-{source_sha.lower()}"


def transaction_branch(mode: str, source_sha: str, product: str = "") -> str:
    """Return the deterministic, transaction-scoped short-lived branch."""
    identity, source = transaction_id(mode, source_sha, product).rsplit("-", 1)
    return f"bot/release-{identity}-{source[:12]}"


def _normalize_browser_version(version: str) -> str:
    match = _BROWSER_VERSION_RE.fullmatch(version)
    if match is None:
        raise ValueError(f"Invalid browser version: {version}")
    major, minor, build, patch = match.groups()
    return f"{int(major)}.{int(minor)}.{int(build)}" + (
        f".{int(patch)}" if patch and int(patch) else ""
    )


def _browser_version_key(version: str) -> tuple[int, int, int, int]:
    parts = [int(part) for part in _normalize_browser_version(version).split(".")]
    parts.extend([0] * (4 - len(parts)))
    return tuple(parts)  # type: ignore[return-value]


def _next_browser_version(version: str) -> str:
    major, minor, build, _ = _browser_version_key(version)
    return f"{major}.{minor}.{build + 1}"


def _validate_record(record: SuiteRecord, request: SuiteRequest | None = None) -> None:
    _validate_sha(record.source_sha, "source_sha")
    _validate_sha(record.reservation_sha, "reservation_sha")
    _validate_sha(record.state_sha, "state_sha")
    if record.state_base_sha:
        _validate_sha(record.state_base_sha, "state_base_sha")
    if record.merge_sha:
        _validate_sha(record.merge_sha, "merge_sha")
    expected_id = transaction_id(record.mode, record.source_sha, record.product)
    if record.transaction_id != expected_id:
        raise ValueError("Suite transaction_id does not match mode and source")
    if record.branch != transaction_branch(
        record.mode, record.source_sha, record.product
    ):
        raise ValueError("Suite branch does not match transaction identity")
    if set(record.component_versions) != set(suite_components(record.product)):
        raise ValueError("Suite component reservation is incomplete")
    for component, version in record.component_versions.items():
        normalize_component_version(component, version)
    _normalize_browser_version(record.browser_version)
    if record.build_offset < 0:
        raise ValueError("Suite build_offset must be non-negative")
    if set(record.state_checksums) - set(record.state_paths):
        raise ValueError("Suite record contains unexpected state checksums")
    if any(
        not re.fullmatch(r"[0-9a-f]{64}", value)
        for value in record.state_checksums.values()
    ):
        raise ValueError("Suite state checksum must be lowercase sha256")
    if request is not None:
        if (
            record.product != request.product
            or record.mode != request.mode
            or record.source_sha.lower() != request.source_sha.lower()
            or record.default_branch != request.default_branch
        ):
            raise ValueError("Recovered suite identity does not match the request")


def _validate_request(request: SuiteRequest) -> None:
    transaction_id(request.mode, request.source_sha, request.product)
    if not request.default_branch:
        raise ValueError("default_branch is required")
    if request.dispatch_ref != request.default_branch:
        raise ValueError(
            f"New release suites must be dispatched from the default branch {request.default_branch}"
        )


def merge_suite_component_allocations(
    allocations: Sequence[AllocationRecord],
    records: Sequence[SuiteRecord],
    components: Sequence[str],
) -> tuple[AllocationRecord, ...]:
    """Merge validated branch-ledger reservations into component allocation state.

    A suite reservation is visible first through its remote branch and later
    through both that branch and its PR marker. Keeping conversion and dedupe
    here makes every candidate/standalone allocator enforce one shared identity
    rule instead of independently translating the aggregate.
    """
    component_ids = tuple(sorted(set(components)))
    for component in component_ids:
        component_by_id(component)

    combined = list(allocations)
    pr_candidate_keys: set[tuple[str, str]] = set()
    for allocation in combined:
        if allocation.kind != "candidate":
            continue
        key = (allocation.component, allocation.candidate_id)
        if key in pr_candidate_keys:
            # Canonical PR history is the durable lifecycle ledger. Two PR
            # markers for one suite branch are corruption, not duplicate views:
            # accepting the newest could hide an older closed ownership veto.
            raise ValueError(
                "Multiple pull requests contain the same suite component allocation"
            )
        pr_candidate_keys.add(key)

    for record in records:
        if record.pull_request_number != 0 or record.pull_request_url:
            raise ValueError(
                "Suite branch ledger record unexpectedly contains a pull request"
            )
        reusable = False
        for component in component_ids:
            if component not in record.component_versions:
                continue
            version = record.component_versions[component]
            combined.append(
                AllocationRecord(
                    component=component,
                    version=version,
                    kind="candidate",
                    source_sha=record.source_sha,
                    candidate_id=record.branch,
                    reference=component_by_id(component).tag_prefix + version,
                    # The suite's open PR authorizes its own component
                    # finalizers to reuse the reservation. A pre-PR branch or
                    # closed/merged PR remains collision history only.
                    reusable=reusable,
                    reuse_forbidden=not reusable,
                )
            )

    deduplicated: list[AllocationRecord] = []
    candidate_indexes: dict[tuple[str, str], int] = {}
    for allocation in combined:
        if allocation.kind != "candidate":
            deduplicated.append(allocation)
            continue
        key = (allocation.component, allocation.candidate_id)
        previous_index = candidate_indexes.get(key)
        if previous_index is None:
            candidate_indexes[key] = len(deduplicated)
            deduplicated.append(allocation)
        else:
            previous = deduplicated[previous_index]
            # The only tolerated duplicate is the independently reconstructed
            # branch view. The pre-PR view is deliberately non-reusable; once
            # one open PR exists, its otherwise identical view carries
            # authorization for suite component finalization. No identity field
            # may differ.
            if (
                replace(
                    allocation,
                    reusable=previous.reusable,
                    reuse_forbidden=previous.reuse_forbidden,
                )
                != previous
            ):
                raise ValueError(
                    "Suite branch and pull request contain conflicting component allocations"
                )
    return tuple(deduplicated)


def _allocate_browser(
    transaction: str,
    committed_version: str,
    committed_offset: int,
    allocations: Sequence[BrowserAllocation],
) -> tuple[str, int]:
    reused = [item for item in allocations if item.transaction_id == transaction]
    if reused:
        if len(reused) != 1:
            raise ValueError("Suite browser reservation is duplicated")
        item = reused[0]
        return _normalize_browser_version(item.browser_version), item.build_offset
    highest_version = max(
        (
            _normalize_browser_version(committed_version),
            *(item.browser_version for item in allocations),
        ),
        key=_browser_version_key,
    )
    highest_offset = max(
        (committed_offset, *(item.build_offset for item in allocations))
    )
    return _next_browser_version(highest_version), highest_offset + 1


def inspect_transaction(request: SuiteRequest, backend: SuiteBackend) -> SuiteRecord:
    """Inspect the canonical PR without allocating or mutating release state."""
    _validate_request(request)
    record = backend.find_transaction(request)
    if record is None:
        raise ValueError(
            f"Suite transaction not found: {transaction_id(request.mode, request.source_sha, request.product)}"
        )
    _validate_record(record, request)
    # Recovery may skip the merge command entirely after an interrupted run.
    # Prove the committed tree here too before a caller can publish from it.
    if record.state == "merged" and (
        not record.merge_sha
        or not backend.merge_commit_matches_transaction(record, record.merge_sha)
    ):
        raise ValueError("Suite merge commit does not match transaction state")
    return record


def reconcile_transaction(
    request: SuiteRequest,
    backend: SuiteBackend,
    *,
    state_root: Path | None = None,
    state_base_sha: str = "",
) -> SuiteRecord:
    """Create/recover a reservation and optionally reconcile its final snapshots."""
    _validate_request(request)
    if backend.current_sha().lower() != request.source_sha.lower():
        raise ValueError("Checkout does not match the frozen source SHA")

    changed = set(backend.changed_paths())
    unexpected = changed - set(suite_state_paths(request.product))
    if unexpected:
        raise ValueError(
            "Suite reconcile found unexpected changes: " + ", ".join(sorted(unexpected))
        )
    if state_root is None and changed:
        raise ValueError("Initial suite reservation requires a clean checkout")
    if state_root is not None:
        missing = [
            path
            for path in suite_state_paths(request.product)
            if not (state_root / path).is_file()
        ]
        if missing:
            raise ValueError(
                "Final suite reconcile requires the complete snapshot set; missing: "
                + ", ".join(missing)
            )

    existing = backend.find_transaction(request)
    if existing is None:
        committed = dict(
            backend.read_committed_versions(request.product)
            if request.product
            else backend.read_committed_versions()
        )
        if set(committed) != set(suite_components(request.product)):
            raise ValueError("Committed suite component set is incomplete")
        allocations = tuple(
            backend.discover_allocations(request.product)
            if request.product
            else backend.discover_allocations()
        )
        branch = transaction_branch(request.mode, request.source_sha, request.product)
        versions: dict[str, str] = {}
        for product in suite_products(request.product):
            component_ids = set(suite_components(product, release_only=True))
            product_allocations = tuple(
                item for item in allocations if item.component in component_ids
            )
            versions.update(
                resolve_candidate_versions(
                    product_id=product,
                    committed_versions=committed,
                    allocations=product_allocations,
                    candidate_id=branch,
                )
            )
        for component in set(suite_components(request.product)) - set(
            suite_components(request.product, release_only=True)
        ):
            versions[component] = normalize_component_version(
                component, committed[component]
            )
        browser_version, build_offset = _allocate_browser(
            transaction_id(request.mode, request.source_sha, request.product),
            backend.read_browser_version(),
            backend.read_build_offset(),
            backend.discover_browser_allocations(),
        )
        existing = backend.create_transaction(
            request,
            branch,
            versions,
            browser_version,
            build_offset,
        )
    _validate_record(existing, request)
    if state_root is None and existing.state != "open":
        # Closed suite PRs remain durable allocation records, but they are not
        # executable release transactions. Merged records are inspectable by
        # publication recovery; neither may emit fresh pre-build outputs.
        raise ValueError(
            f"Initial suite reconciliation requires an open pull request, got {existing.state}"
        )
    if state_root is not None and existing.state not in ("open", "merged"):
        raise ValueError(
            f"Final suite reconciliation requires an open or merged pull request, got {existing.state}"
        )
    if existing.state == "open" and not existing.draft and state_root is None:
        raise ValueError(
            "Ready suite pull request can only be recovered with complete final state"
        )
    if existing.state == "open" and not existing.draft and not existing.product:
        desired = {
            path: hashlib.sha256((state_root / path).read_bytes()).hexdigest()
            for path in existing.state_paths
        }
        if dict(existing.state_checksums) != desired:
            raise ValueError(
                "Ready suite pull request does not match the complete final state"
            )
        # A ready PR is immutable from the suite's perspective. The only next
        # mutation allowed is the separately gated exact-head merge.
        return existing
    if state_root is None:
        return existing
    if request.product:
        reconciled = backend.reconcile_product_state(
            existing, state_root, state_base_sha
        )
    else:
        reconciled = backend.reconcile_state(existing, state_root)
    _validate_record(reconciled, request)
    return reconciled


def _validate_gate(record: SuiteRecord, gate: Mapping[str, object]) -> None:
    if gate.get("schema") != _GATE_SCHEMA or gate.get("passed") is not True:
        raise ValueError("Family release gate did not pass")
    expected: Mapping[str, object] = {
        "transaction_id": record.transaction_id,
        "source_sha": record.source_sha,
        "state_sha": record.state_sha,
        "browser_version": record.browser_version,
        "component_versions": dict(record.component_versions),
        "state_checksums": dict(record.state_checksums),
        "products": list(record.products),
    }
    for name, value in expected.items():
        if gate.get(name) != value:
            raise ValueError(f"Family release gate {name} does not match transaction")


def merge_transaction(
    record: SuiteRecord,
    gate: Mapping[str, object],
    backend: SuiteBackend,
) -> SuiteRecord:
    """Squash-merge one unchanged family state PR after its complete gate."""
    _validate_record(record)
    _validate_gate(record, gate)
    if set(record.state_checksums) != set(record.state_paths):
        raise ValueError("Suite merge requires the complete final state checksum set")
    pull_request = backend.inspect_pull_request(record.pull_request_number)
    if (
        pull_request.url != record.pull_request_url
        or pull_request.head_branch != record.branch
        or pull_request.base_branch != record.default_branch
        or pull_request.head_sha != record.state_sha
    ):
        raise ValueError("Suite pull request identity or head changed")
    if pull_request.state == "merged":
        if not pull_request.merge_sha or not backend.merge_commit_matches_transaction(
            record, pull_request.merge_sha
        ):
            raise ValueError("Suite merge commit does not match transaction state")
        return replace(
            record, state="merged", draft=False, merge_sha=pull_request.merge_sha
        )
    if pull_request.state != "open":
        raise ValueError(f"Suite pull request is not open: {pull_request.state}")
    if not pull_request.mergeable:
        raise ValueError("Suite pull request is not mergeable")
    if backend.default_branch_contains_transaction(record):
        raise ValueError("Suite transaction was superseded on the default branch")
    # The ready transition is deliberately adjacent to the exact-head merge.
    # If the process dies between them, a retry may proceed only through this
    # same complete-checksum, matching-gate path; initial reconciliation fails.
    if pull_request.draft:
        backend.mark_pull_request_ready(record.pull_request_number)
    merge_sha = backend.merge_pull_request(record.pull_request_number, record.state_sha)
    _validate_sha(merge_sha, "merge_sha")
    # The default branch can advance while the merge helper waits. Re-read the
    # resulting squash tree before publication so a mixed or lost-update merge
    # can never be treated as the committed transaction state.
    if not backend.merge_commit_matches_transaction(record, merge_sha):
        raise ValueError("Suite merge commit does not match transaction state")
    return replace(record, state="merged", draft=False, merge_sha=merge_sha)


def _marker_record(document: Mapping[str, object]) -> SuiteRecord:
    full = {
        **document,
        # Old markers included these fields; new markers intentionally derive
        # them from the live same-repository PR object at the parser boundary.
        "pull_request_number": document.get("pull_request_number", 0),
        "pull_request_url": document.get("pull_request_url", ""),
        "state_sha": document.get("reservation_sha", ""),
        "state": "open",
        "merge_sha": "",
        "state_checksums": {},
    }
    return SuiteRecord.from_dict(full)


def suite_record_from_body(body: str) -> SuiteRecord | None:
    """Read immutable suite reservation metadata embedded in a PR body."""
    match = _MARKER_RE.search(body)
    if match is None:
        return None
    document = json.loads(match.group(1))
    if not isinstance(document, dict):
        raise ValueError("Suite pull request marker must be an object")
    return _marker_record(document)


def suite_record_from_pull_request(
    pull_request: Mapping[str, object], repo: str
) -> SuiteRecord | None:
    """Read a suite record only from its canonical same-repository PR branch."""
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
        record = suite_record_from_body(body)
        if record is None:
            return None
        number = pull_request.get("number")
        url = pull_request.get("url")
        head_sha = pull_request.get("headRefOid")
        if (
            not isinstance(number, int)
            or not isinstance(url, str)
            or not isinstance(head_sha, str)
        ):
            return None
        if (
            record.pull_request_number not in (0, number)
            or record.pull_request_url not in ("", url)
            or pull_request.get("headRefName") != record.branch
            or pull_request.get("baseRefName") != record.default_branch
        ):
            return None
        _validate_sha(head_sha, "state_sha")
        merged = bool(pull_request.get("mergedAt"))
        state = "merged" if merged else str(pull_request.get("state", "")).lower()
        draft_value = pull_request.get("isDraft", True)
        if not isinstance(draft_value, bool):
            return None
        merge_document = pull_request.get("mergeCommit")
        merge_sha = ""
        if isinstance(merge_document, dict):
            value = merge_document.get("oid")
            merge_sha = value if isinstance(value, str) else ""
        return replace(
            record,
            pull_request_number=number,
            pull_request_url=url,
            state_sha=head_sha,
            state=state,
            draft=draft_value,
            merge_sha=merge_sha,
        )
    except (json.JSONDecodeError, TypeError, ValueError):
        return None


def suite_allocation_record_from_pull_request(
    pull_request: Mapping[str, object], repo: str
) -> SuiteRecord | None:
    """Return a canonical suite allocation or fail on corrupted ledger state."""
    repository = pull_request.get("headRepository")
    branch = pull_request.get("headRefName")
    canonical = (
        pull_request.get("isCrossRepository") is False
        and isinstance(repository, dict)
        and str(repository.get("nameWithOwner", "")).lower() == repo.lower()
        and isinstance(branch, str)
        and _TRANSACTION_BRANCH_RE.fullmatch(branch) is not None
    )
    record = suite_record_from_pull_request(pull_request, repo)
    if canonical and record is None:
        # Closed PRs are a durable ledger after their source branch is deleted.
        # Silently ignoring a malformed marker would release every reservation.
        raise ValueError(
            f"Canonical suite pull request {branch} has invalid allocation metadata"
        )
    return record


class GitHubSuiteBackend:
    """Git/GitHub persistence for the family transaction aggregate.

    Branch creation and final snapshot reconciliation are separate async
    handoffs. Every recovery validates parent/tree or exact file checksums so a
    pre-existing branch can never be mistaken for this transaction.
    """

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

    def _git(self, *args: str, cwd: Path | None = None, check: bool = True) -> str:
        result = subprocess.run(
            ["git", *args],
            cwd=cwd or self.repo_root,
            capture_output=True,
            text=True,
            check=check,
        )
        return result.stdout.strip()

    def _git_bytes(self, *args: str, cwd: Path | None = None) -> bytes:
        return subprocess.run(
            ["git", *args],
            cwd=cwd or self.repo_root,
            capture_output=True,
            check=True,
        ).stdout

    def current_sha(self) -> str:
        return self._git("rev-parse", "HEAD")

    def changed_paths(self) -> Sequence[str]:
        paths = set(self._git("diff", "--name-only").splitlines())
        paths.update(self._git("diff", "--cached", "--name-only").splitlines())
        paths.update(
            self._git("ls-files", "--others", "--exclude-standard").splitlines()
        )
        return tuple(sorted(path for path in paths if path))

    def _sync(self) -> None:
        self._git(
            "fetch",
            self.remote,
            f"refs/heads/{self.default_branch}:refs/remotes/{self.remote}/{self.default_branch}",
            "--no-tags",
        )
        self._git("fetch", "--force", self.remote, "--tags", "--prune")

    def _state_checksums(self, ref: str, product: str = "") -> dict[str, str]:
        return {
            path: hashlib.sha256(self._git_bytes("show", f"{ref}:{path}")).hexdigest()
            for path in suite_state_paths(product)
        }

    def find_transaction(self, request: SuiteRequest) -> SuiteRecord | None:
        branch = transaction_branch(request.mode, request.source_sha, request.product)
        matches = list_pull_requests(self.repo, state="all", head=branch)
        if not matches:
            return None
        records = [
            record
            for pull_request in matches
            if (record := suite_record_from_pull_request(pull_request, self.repo))
            is not None
        ]
        if len(matches) != 1 or len(records) != 1:
            raise ValueError(
                f"Canonical suite branch {branch} has conflicting pull requests"
            )
        record = records[0]
        _validate_record(record, request)
        if record.product:
            # Inspection may run from an old source checkout after a state
            # merge; refresh main before proving the state's second parent.
            self._git(
                "fetch",
                "--no-tags",
                self.remote,
                f"refs/heads/{record.default_branch}:refs/remotes/{self.remote}/{record.default_branch}",
            )
        if record.state == "merged":
            # GitHub retains this synthetic ref after deleting the suite branch.
            # It is the original frozen-source overlay; the squash tree may also
            # contain unrelated default-branch commits and is not a build input.
            self._git(
                "fetch",
                "--no-tags",
                self.remote,
                f"refs/pull/{record.pull_request_number}/head",
            )
            advertised = self._git("rev-parse", "FETCH_HEAD")
        else:
            # A fresh Actions runner has only the frozen source checkout. Fetch
            # the advertised suite head before deriving its mutable state SHA.
            self._git(
                "fetch",
                "--no-tags",
                self.remote,
                f"refs/heads/{record.branch}:refs/remotes/{self.remote}/{record.branch}",
            )
            advertised = self._git(
                "rev-parse", f"refs/remotes/{self.remote}/{record.branch}"
            )
        if advertised != record.state_sha:
            raise ValueError("Suite branch changed while it was being inspected")
        # A same-repository PR is writable by maintainers. Prove both overlay
        # stages before exposing build outputs so ancestry alone cannot bless an
        # arbitrary code commit as the frozen source.
        self._validate_reservation_history(record)
        self._validate_state_history(record, record.state_sha)
        return replace(
            record,
            state_checksums=self._state_checksums(record.state_sha, record.product),
            state_base_sha=self._product_state_base(record) if record.product else "",
        )

    def discover_allocations(self, product: str = "") -> Sequence[AllocationRecord]:
        # Legacy candidates, tags, releases, and suite reservations all share
        # component namespaces, so every allocator must see the union.
        from .candidate import GitHubCandidateBackend

        allocations: list[AllocationRecord] = []
        legacy = GitHubCandidateBackend(
            self.repo_root, self.repo, self.default_branch, self.remote
        )
        for selected in suite_products(product):
            allocations.extend(legacy.discover_allocations(selected))
        return tuple(allocations)

    def discover_branch_reservations(self) -> Sequence[SuiteRecord]:
        """Recover reservations persisted by the branch push before PR creation.

        Git push and draft-PR creation are separate external writes. The remote
        branch is therefore the allocation ledger for that crash window; its
        reservation commit is accepted only after reconstructing the exact
        source overlay and validating every later commit as suite-owned state.
        """
        output = self._git(
            "ls-remote",
            "--heads",
            self.remote,
            "refs/heads/bot/release-nightly-*",
            "refs/heads/bot/release-full-*",
        )
        advertised: dict[str, str] = {}
        for line in output.splitlines():
            fields = line.split()
            if len(fields) != 2 or not fields[1].startswith("refs/heads/"):
                raise ValueError("Remote suite branch returned invalid metadata")
            branch = fields[1].removeprefix("refs/heads/")
            if _TRANSACTION_BRANCH_RE.fullmatch(branch) is None:
                continue
            _validate_sha(fields[0], "remote suite SHA")
            if branch in advertised:
                raise ValueError(
                    f"Remote suite branch {branch} resolved more than once"
                )
            advertised[branch] = fields[0].lower()

        records = []
        if advertised:
            self._git(
                "fetch",
                "--no-tags",
                self.remote,
                f"refs/heads/{self.default_branch}:refs/remotes/{self.remote}/{self.default_branch}",
            )
        default_ref = f"refs/remotes/{self.remote}/{self.default_branch}"
        for branch, advertised_sha in sorted(advertised.items()):
            self._git(
                "fetch",
                "--no-tags",
                self.remote,
                f"refs/heads/{branch}",
            )
            head_sha = self._git("rev-parse", "FETCH_HEAD")
            if head_sha != advertised_sha:
                raise ValueError(
                    f"Remote suite branch {branch} changed while being inspected"
                )
            record = self._record_from_reservation_branch(branch, head_sha)
            # Canonical branches are the write-ahead allocation ledger before a
            # PR exists, so an off-main source cannot be dismissed as stale: its
            # versions may already name immutable external effects. Rewriting
            # main is unsupported; recovery must fail closed until an operator
            # audits and explicitly removes the invalid reservation.
            source_on_default = subprocess.run(
                ["git", "merge-base", "--is-ancestor", record.source_sha, default_ref],
                cwd=self.repo_root,
                capture_output=True,
            )
            if source_on_default.returncode != 0:
                raise ValueError(
                    f"Remote suite branch {branch} source is not on {self.default_branch}"
                )
            records.append(record)
        return tuple(records)

    def _record_from_reservation_branch(
        self, branch: str, head_sha: str
    ) -> SuiteRecord:
        match = _TRANSACTION_BRANCH_RE.fullmatch(branch)
        if match is None:
            raise ValueError(f"Non-canonical suite branch: {branch}")
        mode = match.group(1)
        product = match.group(2) or ""

        # Reconciled branches advance beyond the reservation. Walk their
        # first-parent history until the branch identity and the commit parent
        # agree; that commit remains the immutable source overlay.
        reservation_sha = ""
        source_sha = ""
        for candidate_sha in self._git(
            "rev-list", "--first-parent", head_sha
        ).splitlines():
            parents = self._git(
                "rev-list", "--parents", "-n", "1", candidate_sha
            ).split()
            if len(parents) != 2:
                continue
            candidate_source = parents[1]
            if transaction_branch(mode, candidate_source, product) == branch:
                reservation_sha = candidate_sha
                source_sha = candidate_source
                break
        if not reservation_sha:
            raise ValueError(
                f"Remote suite branch {branch} has no canonical reservation commit"
            )

        offset_text = self._git(
            "show",
            f"{reservation_sha}:packages/browseros/bos_build/config/BROWSEROS_BUILD_OFFSET",
        )
        if not offset_text.isdigit():
            raise ValueError("Suite reservation build offset is invalid")
        record = SuiteRecord(
            transaction_id=transaction_id(mode, source_sha, product),
            product=product,
            mode=mode,
            source_sha=source_sha,
            reservation_sha=reservation_sha,
            state_sha=head_sha,
            default_branch=self.default_branch,
            branch=branch,
            browser_version=self._browser_version_at_ref(reservation_sha),
            build_offset=int(offset_text),
            component_versions={
                component: self._component_version_at_ref(component, reservation_sha)
                for component in suite_components(product)
            },
            pull_request_number=0,
            pull_request_url="",
            state_checksums=self._state_checksums(head_sha, product),
        )
        _validate_record(record)
        self._validate_reservation_history(record)
        self._validate_state_history(record, head_sha)
        return record

    def discover_browser_allocations(self) -> Sequence[BrowserAllocation]:
        allocations: dict[str, BrowserAllocation] = {}
        for record in self.discover_branch_reservations():
            allocation = BrowserAllocation(
                record.transaction_id,
                record.browser_version,
                record.build_offset,
            )
            allocations[record.transaction_id] = allocation
        # A suite PR is the durable browser-version reservation even if a human
        # closes it after immutable artifacts have escaped. Scanning all PR
        # states prevents that closed transaction from silently releasing its
        # version back to a different source; merged versions are harmlessly
        # redundant with the version already visible on the default branch.
        for pull_request in list_pull_requests(self.repo, state="all"):
            record = suite_allocation_record_from_pull_request(pull_request, self.repo)
            if record is not None:
                allocation = BrowserAllocation(
                    record.transaction_id,
                    record.browser_version,
                    record.build_offset,
                )
                previous = allocations.get(record.transaction_id)
                if previous is not None and previous != allocation:
                    raise ValueError(
                        "Suite branch and pull request contain conflicting browser allocations"
                    )
                allocations[record.transaction_id] = allocation
        return tuple(allocations.values())

    def read_committed_versions(self, product: str = "") -> Mapping[str, str]:
        return {
            component: read_component_version(self.repo_root, component)
            for component in suite_components(product)
        }

    def read_browser_version(self) -> str:
        version = load_semantic_version(self.repo_root / "packages/browseros")
        return _normalize_browser_version(version)

    def read_build_offset(self) -> int:
        value = load_build_offset(self.repo_root / "packages/browseros")
        if not value.isdigit():
            raise ValueError("Browser build offset must be a non-negative integer")
        return int(value)

    def _remote_branch_sha(self, branch: str, *, cwd: Path) -> str | None:
        ref = f"refs/heads/{branch}"
        output = self._git("ls-remote", "--heads", self.remote, ref, cwd=cwd)
        if not output:
            return None
        fields = output.split()
        if len(fields) != 2 or fields[1] != ref:
            raise ValueError(f"Remote suite branch {branch} returned invalid metadata")
        _validate_sha(fields[0], "remote suite SHA")
        return fields[0]

    def _publish_reservation(
        self, branch: str, local_sha: str, source_sha: str, *, cwd: Path
    ) -> str:
        tree_sha = self._git("rev-parse", f"{local_sha}^{{tree}}", cwd=cwd)
        advertised = self._remote_branch_sha(branch, cwd=cwd)
        if advertised is None:
            try:
                self._git(
                    "push",
                    self.remote,
                    f"{local_sha}:refs/heads/{branch}",
                    cwd=cwd,
                )
                return local_sha
            except subprocess.CalledProcessError:
                advertised = self._remote_branch_sha(branch, cwd=cwd)
                if advertised is None:
                    raise
        self._git("fetch", "--no-tags", self.remote, f"refs/heads/{branch}", cwd=cwd)
        fetched = self._git("rev-parse", "FETCH_HEAD", cwd=cwd)
        parents = self._git(
            "rev-list", "--parents", "-n", "1", fetched, cwd=cwd
        ).split()
        if fetched != advertised or parents != [fetched, source_sha]:
            raise ValueError(f"Remote suite branch {branch} has an unexpected parent")
        if self._git("rev-parse", f"{fetched}^{{tree}}", cwd=cwd) != tree_sha:
            raise ValueError(f"Remote suite branch {branch} has conflicting content")
        return fetched

    def _stage_reservation(
        self,
        worktree: Path,
        component_versions: Mapping[str, str],
        browser_version: str,
        build_offset: int,
    ) -> tuple[str, ...]:
        """Apply the deterministic source overlay owned by the reservation."""
        package_root = worktree / "packages/browseros"
        current = load_semantic_version(package_root)
        current_offset = int(load_build_offset(package_root))
        while _browser_version_key(current) < _browser_version_key(browser_version):
            current = bump_version(package_root, "offset+build")
            current_offset += 1
        if current != browser_version or current_offset != build_offset:
            raise ValueError(
                "Resolved browser reservation cannot be staged from source"
            )

        changed = {
            Path("packages/browseros/resources/BROWSEROS_VERSION"),
            Path("packages/browseros/bos_build/config/BROWSEROS_BUILD_OFFSET"),
        }
        for component in SUITE_RELEASE_COMPONENTS:
            if component not in component_versions:
                continue
            changed.update(
                path.relative_to(worktree)
                for path in stamp_component(
                    worktree, component, component_versions[component]
                )
            )
        return tuple(sorted(path.as_posix() for path in changed))

    def _validate_reservation_history(self, record: SuiteRecord) -> None:
        """Prove the reservation is exactly one deterministic source overlay."""
        parents = self._git(
            "rev-list", "--parents", "-n", "1", record.reservation_sha
        ).split()
        if parents != [record.reservation_sha, record.source_sha]:
            raise ValueError(
                "Suite reservation does not have the frozen source as parent"
            )

        expected_paths = {
            "packages/browseros/resources/BROWSEROS_VERSION",
            "packages/browseros/bos_build/config/BROWSEROS_BUILD_OFFSET",
            *(
                path.as_posix()
                for component in record.release_components
                for path in (
                    component_by_id(component).manifest_path,
                    component_by_id(component).lockfile_path,
                )
            ),
        }
        changed = set(
            self._git(
                "diff",
                "--name-only",
                f"{record.source_sha}..{record.reservation_sha}",
            ).splitlines()
        )
        if changed != expected_paths:
            raise ValueError("Suite reservation contains unexpected files")

        # Recreate the overlay from the immutable marker and compare whole-tree
        # identity. This catches payload changes hidden inside an allowed
        # manifest or lockfile, not just extra path names.
        with tempfile.TemporaryDirectory(prefix="browseros-suite-proof-") as temp_dir:
            worktree = Path(temp_dir) / "repo"
            self._git("worktree", "add", "--detach", str(worktree), record.source_sha)
            try:
                relative = self._stage_reservation(
                    worktree,
                    record.component_versions,
                    record.browser_version,
                    record.build_offset,
                )
                if set(relative) != expected_paths:
                    raise ValueError("Suite reservation path contract is inconsistent")
                self._git("add", "--", *relative, cwd=worktree)
                expected_tree = self._git("write-tree", cwd=worktree)
            finally:
                self._git("worktree", "remove", "--force", str(worktree))
        actual_tree = self._git("rev-parse", f"{record.reservation_sha}^{{tree}}")
        if actual_tree != expected_tree:
            raise ValueError("Suite reservation content does not match its record")

    def create_transaction(
        self,
        request: SuiteRequest,
        branch: str,
        component_versions: Mapping[str, str],
        browser_version: str,
        build_offset: int,
    ) -> SuiteRecord:
        with tempfile.TemporaryDirectory(prefix="browseros-suite-") as temp_dir:
            worktree = Path(temp_dir) / "repo"
            self._git("worktree", "add", "--detach", str(worktree), request.source_sha)
            try:
                relative = self._stage_reservation(
                    worktree, component_versions, browser_version, build_offset
                )
                self._git("add", "--", *relative, cwd=worktree)
                staged = set(
                    self._git(
                        "diff", "--cached", "--name-only", cwd=worktree
                    ).splitlines()
                )
                if staged != set(relative):
                    raise ValueError("Suite reservation contains unexpected files")
                self._git(
                    "-c",
                    "user.name=BrowserOS CI",
                    "-c",
                    "user.email=ci@browseros.com",
                    "commit",
                    "-m",
                    f"chore(release): reserve {request.mode} {request.product or 'family'} transaction",
                    cwd=worktree,
                )
                local_sha = self._git("rev-parse", "HEAD", cwd=worktree)
                reservation_sha = self._publish_reservation(
                    branch, local_sha, request.source_sha, cwd=worktree
                )
            finally:
                self._git("worktree", "remove", "--force", str(worktree))

        provisional = SuiteRecord(
            transaction_id=transaction_id(
                request.mode, request.source_sha, request.product
            ),
            mode=request.mode,
            product=request.product,
            source_sha=request.source_sha,
            reservation_sha=reservation_sha,
            state_sha=reservation_sha,
            default_branch=request.default_branch,
            branch=branch,
            browser_version=browser_version,
            build_offset=build_offset,
            component_versions=dict(component_versions),
            pull_request_number=0,
            pull_request_url="",
            state_checksums=self._state_checksums(reservation_sha, request.product),
        )
        url = create_pull_request(
            repo=self.repo,
            head=branch,
            base=request.default_branch,
            title=f"chore(release): {request.mode} {request.product or 'BrowserOS family'} transaction",
            body=provisional.pull_request_body(),
            draft=True,
        )
        match = re.search(r"/(\d+)$", url)
        if match is None:
            raise RuntimeError(f"Could not parse suite pull request number from {url}")
        record = replace(
            provisional,
            pull_request_number=int(match.group(1)),
            pull_request_url=url,
        )
        return record

    def _validate_state_history(self, record: SuiteRecord, state_sha: str) -> None:
        if record.product:
            self._validate_product_state(record, state_sha)
            return
        ancestor = subprocess.run(
            ["git", "merge-base", "--is-ancestor", record.reservation_sha, state_sha],
            cwd=self.repo_root,
        )
        if ancestor.returncode != 0:
            raise ValueError("Suite state head is not descended from its reservation")
        changed = set(
            self._git(
                "diff", "--name-only", f"{record.reservation_sha}..{state_sha}"
            ).splitlines()
        )
        unexpected = changed - set(SUITE_STATE_PATHS)
        if unexpected:
            raise ValueError(
                "Suite state branch contains unexpected files: "
                + ", ".join(sorted(unexpected))
            )

    def _product_state_paths(self, record: SuiteRecord) -> tuple[str, ...]:
        """The only files a product overlay may change relative to current main."""
        return tuple(
            sorted(
                {
                    *record.state_paths,
                    "packages/browseros/resources/BROWSEROS_VERSION",
                    "packages/browseros/bos_build/config/BROWSEROS_BUILD_OFFSET",
                    *(
                        path.as_posix()
                        for component in record.release_components
                        for path in (
                            component_by_id(component).manifest_path,
                            component_by_id(component).lockfile_path,
                        )
                    ),
                }
            )
        )

    def _product_state_base(self, record: SuiteRecord) -> str:
        if record.state_sha == record.reservation_sha:
            return ""
        parents = self._git(
            "rev-list", "--parents", "-n", "1", record.state_sha
        ).split()
        if len(parents) != 3:
            raise ValueError(
                "Product state must retain its reservation and main ancestry"
            )
        return parents[2]

    def _stage_product_state(
        self, worktree: Path, record: SuiteRecord, state_root: Path
    ) -> None:
        """Compose selected state on main, retaining sibling code and reservations."""
        for component in record.release_components:
            current = read_component_version(worktree, component)
            reserved = record.component_versions[component]
            if tuple(map(int, current.split("."))) > tuple(
                map(int, reserved.split("."))
            ):
                raise ValueError(
                    f"Product component {component} was superseded on main"
                )
            stamp_component(worktree, component, reserved)

        # Both products share Chromium's version/offset files. A slower product
        # can finish with an older reservation; publishing it must not rewind the
        # allocation watermark left by the faster sibling.
        package_root = worktree / "packages/browseros"
        current = load_semantic_version(package_root)
        offset = int(load_build_offset(package_root))
        if _browser_version_key(current) < _browser_version_key(record.browser_version):
            while _browser_version_key(current) < _browser_version_key(
                record.browser_version
            ):
                current = bump_version(package_root, "offset+build")
                offset += 1
            if offset != record.build_offset:
                raise ValueError(
                    "Product browser version and offset allocation disagree"
                )
        elif offset < record.build_offset or (
            current == record.browser_version and offset != record.build_offset
        ):
            raise ValueError("Main browser version and offset allocation disagree")

        for relative in record.state_paths:
            destination = worktree / relative
            incoming = (state_root / relative).read_bytes()
            # A standalone publisher can commit a newer snapshot before its
            # component manifest reflects that version. Guard committed state
            # here as well as live R2, before the PR can overwrite the snapshot.
            if relative.endswith(".xml"):
                current_text = destination.read_text()
                incoming_text = incoming.decode("utf-8")
                if relative.startswith("updates/extensions/"):
                    before = extract_manifest_versions(current_text)
                    after = extract_manifest_versions(incoming_text)
                else:
                    old_version = extract_appcast_version(current_text)
                    new_version = extract_appcast_version(incoming_text)
                    before = {"server": old_version} if old_version else {}
                    after = {"server": new_version} if new_version else {}
                if any(
                    key not in after
                    or parse_dotted_version(after[key]) < parse_dotted_version(version)
                    for key, version in before.items()
                ):
                    raise ValueError(
                        f"Snapshot would regress committed state: {relative}"
                    )
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(incoming)

    def _validate_product_state(self, record: SuiteRecord, state_sha: str) -> None:
        """Prove each state handoff is an approved overlay of an actual main commit.

        First parents retain the immutable reservation (and earlier retries);
        second parents supply current main without rebasing or force-pushing.
        Reconstructing the entire tree catches hidden changes in shared lockfiles.
        """
        if state_sha == record.reservation_sha:
            return
        parents = self._git("rev-list", "--parents", "-n", "1", state_sha).split()
        if len(parents) != 3:
            raise ValueError("Product state has unexpected parentage")
        previous, base = parents[1:]
        self._validate_product_state(record, previous)
        default_ref = f"{self.remote}/{record.default_branch}"
        for older, newer in ((record.source_sha, base), (base, default_ref)):
            if subprocess.run(
                ["git", "merge-base", "--is-ancestor", older, newer],
                cwd=self.repo_root,
                capture_output=True,
            ).returncode:
                raise ValueError("Product state base must descend from source on main")
        with tempfile.TemporaryDirectory(prefix="browseros-product-proof-") as temp_dir:
            worktree = Path(temp_dir) / "repo"
            snapshots = Path(temp_dir) / "snapshots"
            for relative in record.state_paths:
                path = snapshots / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(self._git_bytes("show", f"{state_sha}:{relative}"))
            self._git("worktree", "add", "--detach", str(worktree), base)
            try:
                self._stage_product_state(worktree, record, snapshots)
                self._git("add", "--", *self._product_state_paths(record), cwd=worktree)
                expected = self._git("write-tree", cwd=worktree)
            finally:
                self._git("worktree", "remove", "--force", str(worktree))
        if self._git("rev-parse", f"{state_sha}^{{tree}}") != expected:
            raise ValueError(
                "Product state contains changes outside its approved overlay"
            )

    def _mark_pull_request_draft(self, number: int) -> None:
        subprocess.run(
            ["gh", "pr", "ready", str(number), "--undo", "--repo", self.repo],
            cwd=self.repo_root,
            check=True,
        )

    def reconcile_product_state(
        self, record: SuiteRecord, state_root: Path, state_base_sha: str
    ) -> SuiteRecord:
        """Persist product state after rendering under the shared publication lock."""
        _validate_sha(state_base_sha, "state_base_sha")
        self._validate_state_history(record, record.state_sha)
        if record.state == "merged":
            return record
        self._git(
            "fetch",
            "--no-tags",
            self.remote,
            f"refs/heads/{record.default_branch}:refs/remotes/{self.remote}/{record.default_branch}",
        )
        latest = self._git("rev-parse", f"{self.remote}/{record.default_branch}")
        # Code-only main advances are fine; changed owned files mean the caller's
        # baseline is stale and must be re-rendered before any durable write.
        paths = self._product_state_paths(record)
        if any(
            self._git_bytes("show", f"{latest}:{path}")
            != self._git_bytes("show", f"{state_base_sha}:{path}")
            for path in paths
        ):
            raise ValueError("Product state baseline changed; retry publication")
        with tempfile.TemporaryDirectory(prefix="browseros-product-state-") as temp_dir:
            worktree = Path(temp_dir) / "repo"
            self._git("worktree", "add", "--detach", str(worktree), latest)
            try:
                self._stage_product_state(worktree, record, state_root)
                self._git("add", "--", *paths, cwd=worktree)
                tree = self._git("write-tree", cwd=worktree)
                if (
                    record.state_sha != record.reservation_sha
                    and self._product_state_base(record) == latest
                    and self._git("rev-parse", f"{record.state_sha}^{{tree}}") == tree
                ):
                    return record
                if not record.draft:
                    # An interrupted helper may have enabled auto-merge. Return
                    # to draft before changing the head so only a new exact-head
                    # gate can make the recomposed state mergeable.
                    self._mark_pull_request_draft(record.pull_request_number)
                    record = replace(record, draft=True)
                # A merge-shaped state commit keeps both histories reachable.
                # It is a fresh additive commit even after interrupted publication.
                state_sha = self._git(
                    "-c",
                    "user.name=BrowserOS CI",
                    "-c",
                    "user.email=ci@browseros.com",
                    "commit-tree",
                    tree,
                    "-p",
                    record.state_sha,
                    "-p",
                    latest,
                    "-m",
                    f"chore(release): reconcile {record.product} nightly state",
                    cwd=worktree,
                )
                staged = replace(record, state_sha=state_sha, state_base_sha=latest)
                self._validate_product_state(staged, state_sha)
                try:
                    self._git(
                        "push", self.remote, f"{state_sha}:refs/heads/{record.branch}"
                    )
                except subprocess.CalledProcessError:
                    self._git(
                        "fetch", "--no-tags", self.remote, f"refs/heads/{record.branch}"
                    )
                    recovered = self._git("rev-parse", "FETCH_HEAD")
                    self._validate_product_state(record, recovered)
                    if self._git("rev-parse", f"{recovered}^{{tree}}") != tree:
                        raise ValueError(
                            "Remote product state conflicts after push race"
                        )
                    state_sha = recovered
            finally:
                self._git("worktree", "remove", "--force", str(worktree))
        return replace(
            record,
            state_sha=state_sha,
            state_base_sha=latest,
            state_checksums=self._state_checksums(state_sha, record.product),
        )

    @staticmethod
    def _filesystem_checksums(root: Path) -> dict[str, str]:
        return {
            path: hashlib.sha256((root / path).read_bytes()).hexdigest()
            for path in SUITE_STATE_PATHS
        }

    def reconcile_state(self, record: SuiteRecord, state_root: Path) -> SuiteRecord:
        desired = self._filesystem_checksums(state_root)
        ref = record.merge_sha if record.state == "merged" else record.state_sha
        self._validate_state_history(record, record.state_sha)
        current = self._state_checksums(ref)
        if current == desired:
            return replace(record, state_checksums=desired)
        if record.state == "merged" or record.state_sha != record.reservation_sha:
            raise ValueError("Existing suite state conflicts with reconciled snapshots")

        with tempfile.TemporaryDirectory(prefix="browseros-suite-state-") as temp_dir:
            worktree = Path(temp_dir) / "repo"
            self._git("worktree", "add", "--detach", str(worktree), record.state_sha)
            try:
                for relative in SUITE_STATE_PATHS:
                    destination = worktree / relative
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_bytes((state_root / relative).read_bytes())
                self._git("add", "--", *SUITE_STATE_PATHS, cwd=worktree)
                if not self._git("diff", "--cached", "--name-only", cwd=worktree):
                    return replace(record, state_checksums=desired)
                self._git(
                    "-c",
                    "user.name=BrowserOS CI",
                    "-c",
                    "user.email=ci@browseros.com",
                    "commit",
                    "-m",
                    f"chore(release): reconcile {record.mode} family state",
                    cwd=worktree,
                )
                state_sha = self._git("rev-parse", "HEAD", cwd=worktree)
                try:
                    self._git(
                        "push",
                        self.remote,
                        f"{state_sha}:refs/heads/{record.branch}",
                        cwd=worktree,
                    )
                except subprocess.CalledProcessError:
                    recovered = self._remote_branch_sha(record.branch, cwd=worktree)
                    if recovered is None:
                        raise
                    self._git(
                        "fetch",
                        "--no-tags",
                        self.remote,
                        f"refs/heads/{record.branch}",
                        cwd=worktree,
                    )
                    state_sha = self._git("rev-parse", "FETCH_HEAD", cwd=worktree)
                    self._validate_state_history(record, state_sha)
                    if self._state_checksums(state_sha) != desired:
                        raise ValueError("Remote suite state conflicts after push race")
            finally:
                self._git("worktree", "remove", "--force", str(worktree))
        return replace(record, state_sha=state_sha, state_checksums=desired)

    def inspect_pull_request(self, number: int) -> SuitePullRequest:
        result = subprocess.run(
            [
                "gh",
                "pr",
                "view",
                str(number),
                "--repo",
                self.repo,
                "--json",
                "number,url,state,isDraft,headRefOid,headRefName,baseRefName,mergeable,mergedAt,mergeCommit",
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        document = json.loads(result.stdout)
        merged = bool(document.get("mergedAt"))
        merge_document = document.get("mergeCommit")
        merge_sha = ""
        if isinstance(merge_document, dict):
            value = merge_document.get("oid")
            merge_sha = value if isinstance(value, str) else ""
        return SuitePullRequest(
            number=int(document["number"]),
            url=str(document["url"]),
            state="merged" if merged else str(document.get("state", "")).lower(),
            head_sha=str(document["headRefOid"]),
            head_branch=str(document["headRefName"]),
            base_branch=str(document["baseRefName"]),
            mergeable=document.get("mergeable") == "MERGEABLE",
            draft=document.get("isDraft") is True,
            merge_sha=merge_sha,
        )

    def mark_pull_request_ready(self, number: int) -> None:
        mark_pull_request_ready(self.repo, number)

    def _component_version_at_ref(self, component: str, ref: str) -> str:
        spec = component_by_id(component)
        content = self._git("show", f"{ref}:{spec.manifest_path.as_posix()}")
        if spec.manifest_path.suffix == ".json":
            value = json.loads(content).get("version")
        else:
            match = re.search(
                r'(?ms)^\[package\]\s*$.*?^version\s*=\s*"([^"]+)"', content
            )
            value = match.group(1) if match else None
        if not isinstance(value, str):
            raise ValueError(f"Missing {component} version at {ref}")
        return component_version_from_package(component, value)

    def _browser_version_at_ref(self, ref: str) -> str:
        text = self._git(
            "show", f"{ref}:packages/browseros/resources/BROWSEROS_VERSION"
        )
        values = dict(line.split("=", 1) for line in text.splitlines() if "=" in line)
        parts = [
            int(values[f"BROWSEROS_{name}"])
            for name in ("MAJOR", "MINOR", "BUILD", "PATCH")
        ]
        return f"{parts[0]}.{parts[1]}.{parts[2]}" + (
            f".{parts[3]}" if parts[3] else ""
        )

    def default_branch_contains_transaction(self, record: SuiteRecord) -> bool:
        self._git(
            "fetch",
            "--no-tags",
            self.remote,
            f"refs/heads/{record.default_branch}:refs/remotes/{self.remote}/{record.default_branch}",
        )
        default_ref = f"{self.remote}/{record.default_branch}"
        if record.product:
            # The state tree was composed from current main under the feed lock.
            # Sibling completion before that base is expected; later changes to
            # any owned file require recomposition, never a blind squash.
            base = self._product_state_base(record)
            if not base:
                return True
            return any(
                self._git_bytes("show", f"{default_ref}:{path}")
                != self._git_bytes("show", f"{base}:{path}")
                for path in self._product_state_paths(record)
            )
        if self._browser_version_at_ref(default_ref) != self._browser_version_at_ref(
            record.source_sha
        ):
            return True
        if any(
            self._component_version_at_ref(component, default_ref)
            != self._component_version_at_ref(component, record.source_sha)
            for component in SUITE_RELEASE_COMPONENTS
        ):
            return True
        # The suite snapshots are a compare-and-swap set. A standalone release
        # may update one while the Mac builds run; merging source-derived bytes
        # over that newer commit would lose state even if all versions match.
        return self._state_checksums(default_ref) != self._state_checksums(
            record.source_sha
        )

    def merge_pull_request(self, number: int, expected_head_sha: str) -> str:
        script = (
            self.repo_root
            / "packages/browseros-agent/scripts/release/merge-release-pr.sh"
        )
        environment = {**os.environ, "GITHUB_REPOSITORY": self.repo}
        subprocess.run(
            [
                str(script),
                str(number),
                expected_head_sha,
                "chore(release): reconcile nightly release state",
                "Persist the exact gated release transaction.",
            ],
            cwd=self.repo_root,
            env=environment,
            check=True,
        )
        state = self.inspect_pull_request(number)
        if state.state != "merged" or not state.merge_sha:
            raise RuntimeError(f"Suite pull request #{number} merged without a commit")
        return state.merge_sha

    def merge_commit_matches_transaction(
        self, record: SuiteRecord, merge_sha: str
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
        if record.product:
            self._validate_product_state(record, record.state_sha)
            return all(
                self._git_bytes("show", f"{merge_sha}:{path}")
                == self._git_bytes("show", f"{record.state_sha}:{path}")
                for path in self._product_state_paths(record)
            )
        return (
            self._browser_version_at_ref(merge_sha) == record.browser_version
            and all(
                self._component_version_at_ref(component, merge_sha)
                == record.component_versions[component]
                for component in SUITE_RELEASE_COMPONENTS
            )
            and self._state_checksums(merge_sha) == dict(record.state_checksums)
        )
