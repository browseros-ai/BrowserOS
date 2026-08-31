#!/usr/bin/env python3
"""Component identity, version allocation, and stamping."""

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, Mapping, Sequence


VersionScheme = Literal["semver", "chrome"]
AllocationKind = Literal["tag", "release", "candidate", "resource"]

_SEMVER_RE = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")
_CHROME_RE = re.compile(r"^\d+(?:\.\d+){0,3}$")
_CHROME_PACKAGE_RE = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:\+(0|[1-9]\d*))?$"
)


@dataclass(frozen=True)
class ComponentSpec:
    """One versioned in-repository release component."""

    id: str
    display_name: str
    version_scheme: VersionScheme
    manifest_path: Path
    lockfile_path: Path
    package_name: str
    tag_prefix: str
    workspace_path: str = ""
    legacy_tag_prefixes: tuple[str, ...] = ()


@dataclass(frozen=True)
class AllocationRecord:
    """One version allocation discovered from release state."""

    component: str
    version: str
    kind: AllocationKind
    source_sha: str = ""
    candidate_id: str = ""
    reference: str = ""
    reusable: bool = False
    # A closed/merged suite owns this identity permanently even if another
    # effect (such as its still-draft GitHub release) would normally authorize
    # same-source reuse. This explicit veto avoids collapsing lifecycle
    # ownership into the weaker generic ``reusable`` signal.
    reuse_forbidden: bool = False
    blocks: bool = True
    public: bool = False


COMPONENTS: Mapping[str, ComponentSpec] = {
    "server": ComponentSpec(
        id="server",
        display_name="BrowserOS server",
        version_scheme="semver",
        manifest_path=Path("packages/browseros-agent/apps/server/package.json"),
        lockfile_path=Path("packages/browseros-agent/bun.lock"),
        package_name="@browseros/server",
        workspace_path="apps/server",
        tag_prefix="agent-server/v",
        legacy_tag_prefixes=("browseros-server-v",),
    ),
    "agent": ComponentSpec(
        id="agent",
        display_name="BrowserOS agent extension",
        version_scheme="chrome",
        manifest_path=Path("packages/browseros-agent/apps/app/package.json"),
        lockfile_path=Path("packages/browseros-agent/bun.lock"),
        package_name="@browseros/app",
        workspace_path="apps/app",
        tag_prefix="ext-agent/v",
    ),
    "claw-server-rust": ComponentSpec(
        id="claw-server-rust",
        display_name="BrowserOS neo server",
        version_scheme="semver",
        manifest_path=Path("packages/browseros-agent/apps/claw-server-rust/Cargo.toml"),
        lockfile_path=Path("packages/browseros-agent/Cargo.lock"),
        package_name="claw-server-rust",
        tag_prefix="claw-server/v",
    ),
    "browserclaw": ComponentSpec(
        id="browserclaw",
        display_name="BrowserOS neo extension",
        version_scheme="chrome",
        manifest_path=Path("packages/browseros-agent/apps/claw-app/package.json"),
        lockfile_path=Path("packages/browseros-agent/bun.lock"),
        package_name="@browseros/claw-app",
        workspace_path="apps/claw-app",
        tag_prefix="ext-browserclaw/v",
    ),
    "claw-onboard": ComponentSpec(
        id="claw-onboard",
        display_name="BrowserOS neo onboarding",
        version_scheme="semver",
        manifest_path=Path("packages/browseros-agent/apps/claw-onboard/package.json"),
        lockfile_path=Path("packages/browseros-agent/bun.lock"),
        package_name="@browseros/claw-onboard",
        workspace_path="apps/claw-onboard",
        tag_prefix="claw-onboard/v",
    ),
    "app-onboard": ComponentSpec(
        id="app-onboard",
        display_name="BrowserOS onboarding",
        version_scheme="semver",
        manifest_path=Path("packages/browseros-agent/apps/app-onboard/package.json"),
        lockfile_path=Path("packages/browseros-agent/bun.lock"),
        package_name="@browseros/app-onboard",
        workspace_path="apps/app-onboard",
        tag_prefix="app-onboard/v",
    ),
}

_CANDIDATE_COMPONENTS = {
    "browseros": ("server", "agent"),
    "browserclaw": ("claw-server-rust", "browserclaw"),
}


def component_by_id(component_id: str) -> ComponentSpec:
    """Return one component specification."""
    try:
        return COMPONENTS[component_id]
    except KeyError as exc:
        valid = ", ".join(sorted(COMPONENTS))
        raise ValueError(f"Unknown component '{component_id}'. Valid: {valid}") from exc


def components_for_candidate(product_id: str) -> tuple[ComponentSpec, ...]:
    """Return components whose versions change for a browser candidate."""
    try:
        component_ids = _CANDIDATE_COMPONENTS[product_id]
    except KeyError as exc:
        valid = ", ".join(sorted(_CANDIDATE_COMPONENTS))
        raise ValueError(f"Unknown product '{product_id}'. Valid: {valid}") from exc
    return tuple(component_by_id(component_id) for component_id in component_ids)


def normalize_component_version(component_id: str, version: str) -> str:
    """Validate and normalize a component version."""
    spec = component_by_id(component_id)
    if spec.version_scheme == "semver":
        if not _SEMVER_RE.fullmatch(version):
            raise ValueError(
                f"Invalid {spec.display_name} version '{version}'; expected MAJOR.MINOR.PATCH"
            )
        return version
    if not _CHROME_RE.fullmatch(version):
        raise ValueError(
            f"Invalid {spec.display_name} version '{version}'; expected 1-4 integers"
        )
    parts = [int(part) for part in version.split(".")]
    parts.extend([0] * (4 - len(parts)))
    if any(part > 65535 for part in parts):
        raise ValueError(
            f"Invalid {spec.display_name} version '{version}'; components exceed 65535"
        )
    return ".".join(str(part) for part in parts)


def increment_component_version(component_id: str, version: str) -> str:
    """Advance a component according to its release progression."""
    normalized = normalize_component_version(component_id, version)
    parts = [int(part) for part in normalized.split(".")]
    index = 2
    if (
        parts[index] == 65535
        and component_by_id(component_id).version_scheme == "chrome"
    ):
        raise ValueError(f"Cannot increment component version '{normalized}'")
    parts[index] += 1
    if len(parts) == 4:
        parts[3] = 0
    return ".".join(str(part) for part in parts)


def component_package_version(component_id: str, version: str) -> str:
    """Encode a release version for its source package manifest."""
    normalized = normalize_component_version(component_id, version)
    if component_by_id(component_id).version_scheme == "semver":
        return normalized
    major, minor, patch, build = normalized.split(".")
    base = f"{major}.{minor}.{patch}"
    return base if build == "0" else f"{base}+{build}"


def component_version_from_package(component_id: str, version: str) -> str:
    """Decode a source package version into its release identity."""
    if component_by_id(component_id).version_scheme == "semver":
        return normalize_component_version(component_id, version)
    match = _CHROME_PACKAGE_RE.fullmatch(version)
    if match is not None:
        major, minor, patch, build = match.groups()
        return normalize_component_version(
            component_id,
            f"{major}.{minor}.{patch}.{build or '0'}",
        )
    return normalize_component_version(component_id, version)


def _version_key(component_id: str, version: str) -> tuple[int, ...]:
    return tuple(
        int(part)
        for part in normalize_component_version(component_id, version).split(".")
    )


def _component_allocations(
    component_id: str, allocations: Sequence[AllocationRecord]
) -> tuple[AllocationRecord, ...]:
    records = tuple(
        record for record in allocations if record.component == component_id
    )
    for record in records:
        normalize_component_version(component_id, record.version)
    return records


def _next_unallocated(
    component_id: str,
    version: str,
    allocations: Sequence[AllocationRecord],
) -> str:
    blocked = {
        normalize_component_version(component_id, record.version)
        for record in _component_allocations(component_id, allocations)
        if record.blocks
    }
    current = normalize_component_version(component_id, version)
    highest = max(
        (current, *blocked),
        key=lambda value: _version_key(component_id, value),
    )
    candidate = increment_component_version(component_id, highest)
    while candidate in blocked:
        candidate = increment_component_version(component_id, candidate)
    return candidate


def resolve_candidate_versions(
    *,
    product_id: str,
    committed_versions: Mapping[str, str],
    allocations: Sequence[AllocationRecord],
    candidate_id: str,
) -> dict[str, str]:
    """Allocate or recover the component versions for one candidate."""
    if not candidate_id:
        raise ValueError("candidate_id is required")
    specs = components_for_candidate(product_id)
    expected = {spec.id for spec in specs}
    reused = {
        record.component: normalize_component_version(record.component, record.version)
        for record in allocations
        if record.kind == "candidate" and record.candidate_id == candidate_id
    }
    if reused:
        if set(reused) != expected:
            missing = ", ".join(sorted(expected - set(reused)))
            extra = ", ".join(sorted(set(reused) - expected))
            detail = f"missing={missing or '-'}, extra={extra or '-'}"
            raise ValueError(f"Candidate reservation is incomplete: {detail}")
        return {spec.id: reused[spec.id] for spec in specs}

    result: dict[str, str] = {}
    for spec in specs:
        try:
            committed = committed_versions[spec.id]
        except KeyError as exc:
            raise ValueError(f"Missing committed version for {spec.id}") from exc
        result[spec.id] = _next_unallocated(spec.id, committed, allocations)
    return result


def resolve_standalone_version(
    *,
    component_id: str,
    committed_version: str,
    allocations: Sequence[AllocationRecord],
    requested_version: str = "",
    source_sha: str = "",
) -> str:
    """Resolve a standalone component release version."""
    records = _component_allocations(component_id, allocations)
    public_versions = tuple(
        normalize_component_version(component_id, record.version)
        for record in records
        if record.public
    )

    def require_not_older(version: str) -> str:
        if public_versions:
            newest = max(
                public_versions,
                key=lambda value: _version_key(component_id, value),
            )
            if _version_key(component_id, version) < _version_key(component_id, newest):
                raise ValueError(
                    f"{component_id} version {version} is older than newest "
                    f"public version {newest}"
                )
        return version

    if requested_version:
        requested = normalize_component_version(component_id, requested_version)
        collisions = [
            record
            for record in records
            if record.blocks
            and normalize_component_version(component_id, record.version) == requested
        ]
        if not collisions:
            return require_not_older(requested)
        if any(record.reuse_forbidden for record in collisions):
            raise ValueError(f"{component_id} version {requested} is already allocated")
        canonical_reference = component_by_id(component_id).tag_prefix + requested
        if (
            source_sha
            and all(
                record.reference == canonical_reference
                and record.source_sha == source_sha
                for record in collisions
            )
            and any(record.reusable for record in collisions)
        ):
            return require_not_older(requested)
        raise ValueError(f"{component_id} version {requested} is already allocated")

    reusable_versions = {
        normalize_component_version(component_id, record.version)
        for record in records
        if record.reusable and source_sha and record.source_sha == source_sha
    }
    for reusable_version in sorted(
        reusable_versions,
        key=lambda version: _version_key(component_id, version),
        reverse=True,
    ):
        same_version = [
            record
            for record in records
            if normalize_component_version(component_id, record.version)
            == reusable_version
        ]
        if any(record.reuse_forbidden for record in same_version):
            continue
        resources = [
            record
            for record in records
            if record.kind == "resource"
            and normalize_component_version(component_id, record.version)
            == reusable_version
        ]
        if resources and not all(
            record.reusable and record.source_sha == source_sha for record in resources
        ):
            continue
        return require_not_older(reusable_version)

    committed = normalize_component_version(component_id, committed_version)
    blocked = {
        normalize_component_version(component_id, record.version)
        for record in records
        if record.blocks
    }
    if not blocked or _version_key(component_id, committed) > max(
        (_version_key(component_id, version) for version in blocked)
    ):
        return committed
    highest = max(
        (committed, *blocked),
        key=lambda version: _version_key(component_id, version),
    )
    return _next_unallocated(component_id, highest, records)


def read_component_version(repo_root: Path, component_id: str) -> str:
    """Read a component version from its source manifest."""
    spec = component_by_id(component_id)
    path = repo_root / spec.manifest_path
    if path.suffix == ".json":
        document = json.loads(path.read_text(encoding="utf-8"))
        version = document.get("version")
    else:
        match = re.search(
            r'(?ms)^\[package\]\s*$.*?^version\s*=\s*"([^"]+)"',
            path.read_text(encoding="utf-8"),
        )
        version = match.group(1) if match else None
    if not isinstance(version, str):
        raise ValueError(f"Missing version in {path}")
    return component_version_from_package(component_id, version)


def _stamp_json_manifest(path: Path, version: str) -> None:
    document = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(document.get("version"), str):
        raise ValueError(f"Missing version in {path}")
    document["version"] = version
    path.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")


def _stamp_bun_lock(path: Path, workspace_path: str, version: str) -> None:
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    marker = f'    "{workspace_path}": {{'
    try:
        start = next(
            index for index, line in enumerate(lines) if line.rstrip() == marker
        )
    except StopIteration as exc:
        raise ValueError(f"Workspace {workspace_path} not found in {path}") from exc
    end = len(lines)
    for index in range(start + 1, len(lines)):
        if lines[index].startswith('    "') or lines[index].startswith("  }"):
            end = index
            break
    matches = [
        index
        for index in range(start + 1, end)
        if re.fullmatch(r'\s+"version": "[^"]+",?\r?\n?', lines[index])
    ]
    if len(matches) != 1:
        raise ValueError(
            f"Expected one version for workspace {workspace_path} in {path}"
        )
    index = matches[0]
    newline = "\n" if lines[index].endswith("\n") else ""
    comma = "," if lines[index].rstrip().endswith(",") else ""
    indent = lines[index][: len(lines[index]) - len(lines[index].lstrip())]
    lines[index] = f'{indent}"version": "{version}"{comma}{newline}'
    path.write_text("".join(lines), encoding="utf-8")


def _replace_package_version(
    text: str, package_header: str, package_name: str, version: str
) -> str:
    blocks = list(
        re.finditer(
            rf"(?ms)^{re.escape(package_header)}\s*$.*?(?=^\[|\Z)",
            text,
        )
    )
    matching = []
    for block in blocks:
        name = re.search(r'^name\s*=\s*"([^"]+)"', block.group(0), re.MULTILINE)
        if package_header == "[package]" or (
            name is not None and name.group(1) == package_name
        ):
            matching.append(block)
    if len(matching) != 1:
        raise ValueError(f"Expected one package entry for {package_name}")
    block = matching[0]
    replacement, count = re.subn(
        r'(?m)^(version\s*=\s*")[^"]+("\s*)$',
        rf"\g<1>{version}\g<2>",
        block.group(0),
        count=1,
    )
    if count != 1:
        raise ValueError(f"Missing version for package {package_name}")
    return text[: block.start()] + replacement + text[block.end() :]


def stamp_component(
    repo_root: Path, component_id: str, version: str
) -> tuple[Path, Path]:
    """Stamp a component manifest and its matching lockfile entry."""
    spec = component_by_id(component_id)
    normalized = normalize_component_version(component_id, version)
    manifest = repo_root / spec.manifest_path
    lockfile = repo_root / spec.lockfile_path
    if manifest.suffix == ".json":
        package_version = component_package_version(component_id, normalized)
        _stamp_json_manifest(manifest, package_version)
        _stamp_bun_lock(lockfile, spec.workspace_path, package_version)
    else:
        manifest.write_text(
            _replace_package_version(
                manifest.read_text(encoding="utf-8"),
                "[package]",
                spec.package_name,
                normalized,
            ),
            encoding="utf-8",
        )
        lockfile.write_text(
            _replace_package_version(
                lockfile.read_text(encoding="utf-8"),
                "[[package]]",
                spec.package_name,
                normalized,
            ),
            encoding="utf-8",
        )
    return manifest, lockfile
