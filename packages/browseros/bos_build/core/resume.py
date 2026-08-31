#!/usr/bin/env python3
"""Versioned resume checkpoint validation."""

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Mapping, Sequence

import yaml

from .context import Context
from .step import Step
from ..lib.utils import get_platform
from ..products.resource_sources import source_resources_for_product

CHECKPOINT_SCHEMA = "browseros-build-resume-checkpoint-v1"
CONTRACT_SCHEMA = "browseros-build-resume-contract-v1"
CHECKPOINT_ROOT = ".browseros_resume"
_HASH_CHUNK_SIZE = 1024 * 1024
_CHROMIUM_MUTATION_EXCLUDES = ("out", "chrome/VERSION")
_CHROMIUM_MUTATION_STEPS = frozenset(
    {
        "chromium_replace",
        "string_replaces",
        "series_patches",
        "patches",
    }
)


class ResumeValidationError(ValueError):
    """Raised when a resume checkpoint cannot prove the requested state."""


@dataclass(frozen=True)
class ResumeState:
    full_arch_plans: tuple[tuple[str, tuple[str, ...]], ...]
    resume_from: str | None
    candidate: Mapping[str, Any]
    candidate_digest: str
    strict: bool = False
    unresumable_reason: str = ""

    @property
    def enabled(self) -> bool:
        return bool(self.candidate_digest) and not self.unresumable_reason


def make_resume_state(
    ctx: Context,
    full_arch_plans: Sequence[tuple[str, Sequence[str]]],
    *,
    resume_from: str | None,
    strict: bool,
) -> ResumeState:
    plans = tuple((arch, tuple(steps)) for arch, steps in full_arch_plans)
    try:
        candidate = _candidate_contract(ctx, plans)
    except _Unresumable as exc:
        return ResumeState(plans, resume_from, {}, "", strict, str(exc))
    return ResumeState(
        plans,
        resume_from,
        candidate,
        _json_digest(candidate),
        strict,
    )


def attach_resume_state(
    runs: Sequence[tuple[Context, Sequence[str]]],
    full_arch_plans: Sequence[tuple[str, Sequence[str]]],
    *,
    resume_from: str | None,
    strict: bool,
) -> None:
    if not runs:
        return
    base_state = make_resume_state(
        runs[0][0],
        full_arch_plans,
        resume_from=resume_from,
        strict=strict,
    )
    for ctx, _steps in runs:
        ctx.resume_state = base_state


def validate_resume_before_execution(
    runs: Sequence[tuple[Context, Sequence[str]]],
) -> None:
    states = [state for ctx, _steps in runs if (state := _state(ctx)) is not None]
    if not states or not states[0].strict:
        return
    state = states[0]
    if state.unresumable_reason:
        raise ResumeValidationError(
            _restart_message(
                "Resume state cannot be proven: " + state.unresumable_reason,
                state.resume_from or "the requested step",
            )
        )
    if not state.resume_from:
        return

    contexts = {ctx.architecture: ctx for ctx, _steps in runs}
    base_ctx = runs[0][0]
    for arch, completed_steps in _completed_steps_before(
        state.full_arch_plans, state.resume_from
    ):
        ctx = contexts.get(arch) or _context_for_arch(base_ctx, arch)
        latest_mutation = _latest_mutation_step(completed_steps)
        for step_name in completed_steps:
            checkpoint = _validated_checkpoint(
                ctx,
                step_name,
                state,
                validate_chromium_mutation=step_name == latest_mutation
                or step_name not in _CHROMIUM_MUTATION_STEPS,
            )
            if arch in contexts:
                _restore_registry(contexts[arch], checkpoint)


def validate_universal_inputs(ctx: Context, architectures: Sequence[str]) -> None:
    state = _state(ctx)
    if state is None:
        return
    if state.unresumable_reason:
        raise ResumeValidationError(
            _restart_message(
                "Universal input provenance cannot be proven: "
                + state.unresumable_reason,
                "clean",
            )
        )
    expected = tuple(architectures)
    seen = []
    for arch in expected:
        arch_ctx = _context_for_arch(ctx, arch)
        checkpoint = _validated_checkpoint(arch_ctx, "sign_macos", state)
        if checkpoint.get("architecture") != arch:
            raise _mismatch(
                arch,
                "sign_macos",
                f"architecture mismatch: expected {arch}, got "
                f"{checkpoint.get('architecture')!r}",
            )
        _require_attested_path(checkpoint, "signed_app", arch, "sign_macos")
        seen.append(checkpoint.get("architecture"))
    if tuple(seen) != expected:
        raise ResumeValidationError(
            _restart_message(
                f"Universal input architecture set mismatch: expected "
                f"{', '.join(expected)}, got {', '.join(str(item) for item in seen)}",
                "merge_universal",
            )
        )


def invalidate_checkpoints_from(
    ctx: Context,
    run_steps: Sequence[str],
    step_name: str,
) -> None:
    state = _state(ctx)
    if state is None:
        return
    try:
        start = list(run_steps).index(step_name)
    except ValueError:
        return
    for name in run_steps[start:]:
        checkpoint_path(ctx, name).unlink(missing_ok=True)


def write_step_checkpoint(
    ctx: Context,
    step: Step,
    run_steps: Sequence[str],
) -> None:
    state = _state(ctx)
    if state is None or not state.enabled:
        return
    step_name = step.name or step.__class__.__name__
    recorded_steps = _steps_for_arch(state.full_arch_plans, ctx.architecture)
    if not recorded_steps:
        recorded_steps = tuple(run_steps)
    document = {
        "schema": CHECKPOINT_SCHEMA,
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "candidate_digest": state.candidate_digest,
        "candidate": state.candidate,
        "product": ctx.product.id,
        "architecture": ctx.architecture,
        "build_type": ctx.build_type,
        "platform": get_platform(),
        "out_dir": ctx.out_dir,
        "step": step_name,
        "run_steps": list(recorded_steps),
        "chromium_checkout": _chromium_checkout_identity(ctx),
        "registry": _registry_entries(ctx, step),
        "snapshots": _step_snapshots(ctx, step_name),
    }
    _atomic_write_json(checkpoint_path(ctx, step_name), document)


def remove_checkpoint_dirs(ctx: Context, architectures: Sequence[str]) -> None:
    for architecture in architectures:
        shutil.rmtree(checkpoint_dir(ctx, architecture), ignore_errors=True)


def checkpoint_dir(ctx: Context, architecture: str | None = None) -> Path:
    return (
        ctx.chromium_src
        / "out"
        / CHECKPOINT_ROOT
        / ctx.product.id
        / ctx.build_type
        / (architecture or ctx.architecture)
    )


def checkpoint_path(
    ctx: Context,
    step_name: str,
    architecture: str | None = None,
) -> Path:
    return checkpoint_dir(ctx, architecture) / f"{step_name}.json"


class _Unresumable(RuntimeError):
    pass


def _state(ctx: Context) -> ResumeState | None:
    state = getattr(ctx, "resume_state", None)
    return state if isinstance(state, ResumeState) else None


def _candidate_contract(
    ctx: Context,
    full_arch_plans: tuple[tuple[str, tuple[str, ...]], ...],
) -> dict[str, Any]:
    return {
        "schema": CONTRACT_SCHEMA,
        "product": ctx.product.id,
        "build_type": ctx.build_type,
        "platform": get_platform(),
        "plan_architectures": list(ctx.plan_architectures),
        "full_arch_plans": [
            {"architecture": arch, "steps": list(steps)}
            for arch, steps in full_arch_plans
        ],
        "versions": {
            "chromium": ctx.chromium_version,
            "browseros_chromium": ctx.browseros_chromium_version,
            "semantic": ctx.semantic_version,
            "build_offset": ctx.browseros_build_offset,
        },
        "browseros_source": _browseros_source_identity(ctx),
        "chromium_pin": {"version": ctx.chromium_version},
        "configuration": _configuration_identity(ctx),
        "resources": _resource_identity(ctx),
    }


def _browseros_source_identity(ctx: Context) -> dict[str, Any]:
    repo_root = ctx.root_dir.parent.parent.resolve()
    head = _git_text(["rev-parse", "HEAD"], repo_root)
    if not _is_full_sha(head):
        raise _Unresumable("BrowserOS source checkout HEAD is not a full SHA")
    top = _git_text(["rev-parse", "--show-toplevel"], repo_root)
    paths = _git_changed_paths(repo_root)
    identity: dict[str, Any] = {
        "kind": "git-clean" if not paths else "git-dirty",
        "root": top,
        "head": head,
    }
    if paths:
        identity["dirty_digest"] = _changed_paths_digest(repo_root, paths)
        identity["dirty_paths"] = paths
    if ctx.source_sha and ctx.source_sha != head:
        raise _Unresumable("Context source SHA does not match BrowserOS HEAD")
    return identity


def _configuration_identity(ctx: Context) -> dict[str, Any]:
    paths = {
        "gn_flags": ctx.gn_flags_file,
        "copy_resources": ctx.get_copy_resources_config(),
        "download_resources": ctx.get_download_resources_config(),
    }
    return {
        "extra_gn_args": list(ctx.extra_gn_args),
        "files": {
            name: _file_identity(path) if path else {"missing": True}
            for name, path in paths.items()
        },
    }


def _resource_identity(ctx: Context) -> dict[str, Any]:
    if ctx.resource_mode == "source":
        identity: dict[str, Any] = {
            "mode": "source",
            "source_sha": ctx.source_sha,
            "prepared_resources": str(ctx.prepared_resources or ""),
        }
        if ctx.prepared_resources and ctx.prepared_resources.exists():
            from ..release.prepared_resources import load_prepared_resources

            manifest = load_prepared_resources(ctx.prepared_resources)
            identity.update(
                {
                    "prepared_digest": manifest.digest(),
                    "component_versions": dict(manifest.component_versions),
                }
            )
        return identity

    source = source_resources_for_product(ctx.product.id)
    server_version = (
        ctx.env.browseros_server_resource_version
        if ctx.product.id == "browseros"
        else ctx.env.browserclaw_server_resource_version
    )
    components = {
        source.server_component: server_version,
        source.extension_component: ctx.env.bundled_product_extension_version,
        source.onboarding_component: ctx.env.onboarding_resource_version,
    }
    missing = [name for name, value in components.items() if not value]
    if missing:
        raise _Unresumable(
            "published resources require exact component pins: "
            + ", ".join(sorted(missing))
        )
    return {
        "mode": "published",
        "component_versions": components,
        "bundled_manifest_url": ctx.get_extensions_manifest_url(),
    }


def _registry_entries(ctx: Context, step: Step) -> list[dict[str, Any]]:
    names = set(step.produces)
    step_name = step.name or step.__class__.__name__
    if step_name == "package_windows":
        names.add("built_app")
    if step_name == "bundled_extensions":
        names.add("common_manifest_digest")

    entries = []
    registry = ctx.artifact_registry
    for name in sorted(names):
        if not registry.has(name):
            continue
        value = registry.get(name)
        entry = _registry_entry(name, value)
        if entry is not None:
            entries.append(entry)
    return entries


def _registry_entry(name: str, value: Any) -> dict[str, Any] | None:
    if isinstance(value, Path):
        return {
            "name": name,
            "kind": "path",
            "path": str(value),
            "attestation": _path_attestation(value),
        }
    if name == "sparkle_signatures" and isinstance(value, dict):
        return {
            "name": name,
            "kind": "sparkle_signatures",
            "value": {
                str(filename): [str(signature), int(length)]
                for filename, (signature, length) in value.items()
            },
        }
    if name == "server_resources" and isinstance(value, dict):
        return {
            "name": name,
            "kind": "server_resources",
            "value": {
                str(target): {
                    "product": str(result.product),
                    "target": str(result.target),
                    "version": str(result.version),
                    "source_sha": str(result.source_sha),
                    "destination": str(result.destination),
                    "manifest_sha256": str(result.manifest_sha256),
                }
                for target, result in value.items()
            },
        }
    if _json_safe(value):
        return {"name": name, "kind": "json", "value": value}
    return None


def _step_snapshots(ctx: Context, step_name: str) -> list[dict[str, Any]]:
    snapshots: list[dict[str, Any]] = []
    if step_name == "configure":
        _append_existing_snapshot(snapshots, "args_gn", ctx.get_gn_args_file())
    elif step_name == "download_resources":
        for path in _download_resource_paths(ctx):
            _append_existing_snapshot(snapshots, "download_resource", path)
    elif step_name == "prepare_common_resources":
        if ctx.prepared_resources:
            _append_existing_snapshot(
                snapshots, "prepared_resources", ctx.prepared_resources
            )
    elif step_name == "prepare_server_resources":
        for value in ctx.artifact_registry.get("server_resources", {}).values():
            destination = getattr(value, "destination", None)
            if isinstance(destination, Path):
                _append_existing_snapshot(snapshots, "server_resource", destination)
    elif step_name == "resources":
        for path in _managed_copy_destinations(ctx):
            _append_existing_snapshot(snapshots, "managed_resource", path)
    elif step_name == "bundled_extensions":
        _append_existing_snapshot(
            snapshots,
            "bundled_extensions",
            ctx.chromium_src / "chrome/browser/browseros/bundled_extensions",
        )
    elif step_name in _CHROMIUM_MUTATION_STEPS:
        digest = _chromium_mutation_digest(ctx)
        if digest:
            snapshots.append({"kind": "chromium_mutation", **digest})
    return snapshots


def _append_existing_snapshot(
    snapshots: list[dict[str, Any]],
    role: str,
    path: Path,
) -> None:
    if path.exists() or path.is_symlink():
        snapshots.append(
            {
                "kind": "path",
                "role": role,
                "path": str(path),
                "attestation": _path_attestation(path),
            }
        )


def _validated_checkpoint(
    ctx: Context,
    step_name: str,
    state: ResumeState,
    *,
    validate_chromium_mutation: bool = True,
) -> Mapping[str, Any]:
    path = checkpoint_path(ctx, step_name)
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise ResumeValidationError(
            _restart_message(
                f"Resume checkpoint missing for {ctx.architecture}/{step_name}: {path}",
                step_name,
            )
        ) from exc
    except (OSError, json.JSONDecodeError) as exc:
        raise ResumeValidationError(
            _restart_message(
                f"Resume checkpoint corrupt for {ctx.architecture}/{step_name}: {path}",
                step_name,
            )
        ) from exc
    if not isinstance(document, dict) or document.get("schema") != CHECKPOINT_SCHEMA:
        raise _mismatch(ctx.architecture, step_name, "unsupported checkpoint schema")
    _validate_checkpoint_identity(ctx, step_name, state, document)
    _validate_chromium_checkout(ctx, step_name, document)
    for entry in document.get("registry", []):
        _validate_registry_entry(ctx.architecture, step_name, entry)
    for snapshot in document.get("snapshots", []):
        _validate_snapshot(
            ctx.architecture,
            step_name,
            snapshot,
            validate_chromium_mutation=validate_chromium_mutation,
        )
    return document


def _validate_checkpoint_identity(
    ctx: Context,
    step_name: str,
    state: ResumeState,
    document: Mapping[str, Any],
) -> None:
    if document.get("candidate_digest") != state.candidate_digest:
        detail = _candidate_mismatch_detail(document.get("candidate"), state.candidate)
        raise _mismatch(ctx.architecture, step_name, detail)
    expected = {
        "product": ctx.product.id,
        "architecture": ctx.architecture,
        "build_type": ctx.build_type,
        "platform": get_platform(),
        "step": step_name,
    }
    for field, value in expected.items():
        if document.get(field) != value:
            raise _mismatch(
                ctx.architecture,
                step_name,
                f"{field} mismatch: expected {value!r}, got {document.get(field)!r}",
            )
    expected_steps = list(_steps_for_arch(state.full_arch_plans, ctx.architecture))
    if document.get("run_steps") != expected_steps:
        raise _mismatch(ctx.architecture, step_name, "run step list mismatch")


def _validate_chromium_checkout(
    ctx: Context,
    step_name: str,
    document: Mapping[str, Any],
) -> None:
    recorded = document.get("chromium_checkout")
    if not isinstance(recorded, dict) or "head" not in recorded:
        return
    current = _chromium_checkout_identity(ctx)
    if current.get("head") != recorded.get("head"):
        raise _mismatch(
            ctx.architecture,
            step_name,
            f"Chromium checkout mismatch: expected {recorded.get('head')}, "
            f"got {current.get('head') or 'unavailable'}",
        )


def _validate_registry_entry(arch: str, step_name: str, entry: Any) -> None:
    if not isinstance(entry, dict):
        raise _mismatch(arch, step_name, "registry entry is invalid")
    if entry.get("kind") == "path":
        _validate_attestation(
            entry.get("attestation"),
            Path(str(entry.get("path", ""))),
            arch,
            step_name,
            str(entry.get("name", "artifact")),
        )
    elif entry.get("kind") in {"json", "sparkle_signatures", "server_resources"}:
        return
    else:
        raise _mismatch(arch, step_name, "registry entry kind is unsupported")


def _validate_snapshot(
    arch: str,
    step_name: str,
    snapshot: Any,
    *,
    validate_chromium_mutation: bool,
) -> None:
    if not isinstance(snapshot, dict):
        raise _mismatch(arch, step_name, "snapshot is invalid")
    if snapshot.get("kind") == "path":
        _validate_attestation(
            snapshot.get("attestation"),
            Path(str(snapshot.get("path", ""))),
            arch,
            step_name,
            str(snapshot.get("role", "snapshot")),
        )
        return
    if snapshot.get("kind") == "chromium_mutation":
        if not validate_chromium_mutation:
            return
        current = _chromium_mutation_digest_for_root(
            Path(str(snapshot.get("root", "")))
        )
        if not current or current.get("digest") != snapshot.get("digest"):
            raise _mismatch(arch, step_name, "Chromium mutation digest mismatch")
        return
    raise _mismatch(arch, step_name, "snapshot kind is unsupported")


def _restore_registry(ctx: Context, document: Mapping[str, Any]) -> None:
    for entry in document.get("registry", []):
        if not isinstance(entry, dict) or not isinstance(entry.get("name"), str):
            continue
        name = entry["name"]
        kind = entry.get("kind")
        if kind == "path":
            ctx.artifact_registry.add(name, Path(str(entry.get("path", ""))))
        elif kind == "json":
            ctx.artifact_registry.add(name, entry.get("value"))
        elif kind == "sparkle_signatures":
            value = entry.get("value", {})
            if isinstance(value, dict):
                ctx.artifact_registry.add(
                    name,
                    {
                        str(filename): (str(pair[0]), int(pair[1]))
                        for filename, pair in value.items()
                        if isinstance(pair, list) and len(pair) == 2
                    },
                )
        elif kind == "server_resources":
            value = entry.get("value", {})
            if isinstance(value, dict):
                ctx.artifact_registry.add(
                    name,
                    {
                        str(target): SimpleNamespace(
                            product=str(item["product"]),
                            target=str(item["target"]),
                            version=str(item["version"]),
                            source_sha=str(item["source_sha"]),
                            destination=Path(str(item["destination"])),
                            manifest_sha256=str(item["manifest_sha256"]),
                        )
                        for target, item in value.items()
                        if isinstance(item, dict)
                    },
                )


def _require_attested_path(
    checkpoint: Mapping[str, Any],
    artifact_name: str,
    arch: str,
    step_name: str,
) -> None:
    for entry in checkpoint.get("registry", []):
        if (
            isinstance(entry, dict)
            and entry.get("name") == artifact_name
            and entry.get("kind") == "path"
        ):
            _validate_registry_entry(arch, step_name, entry)
            return
    raise _mismatch(arch, step_name, f"missing attested {artifact_name}")


def _completed_steps_before(
    full_arch_plans: Sequence[tuple[str, Sequence[str]]],
    resume_from: str,
) -> list[tuple[str, tuple[str, ...]]]:
    completed = []
    for arch, steps in full_arch_plans:
        if resume_from in steps:
            completed.append((arch, tuple(steps[: list(steps).index(resume_from)])))
            return completed
        completed.append((arch, tuple(steps)))
    return completed


def _latest_mutation_step(completed_steps: Sequence[str]) -> str:
    for step_name in reversed(completed_steps):
        if step_name in _CHROMIUM_MUTATION_STEPS:
            return step_name
    return ""


def _steps_for_arch(
    full_arch_plans: Sequence[tuple[str, Sequence[str]]],
    architecture: str,
) -> tuple[str, ...]:
    for arch, steps in full_arch_plans:
        if arch == architecture:
            return tuple(steps)
    return ()


def _context_for_arch(ctx: Context, architecture: str) -> Context:
    sibling = Context(
        root_dir=ctx.root_dir,
        chromium_src=ctx.chromium_src,
        architecture=architecture,
        plan_architectures=ctx.plan_architectures,
        build_type=ctx.build_type,
        product=ctx.product,
        gn_flags_file=ctx.gn_flags_file,
        extra_gn_args=ctx.extra_gn_args,
        resource_mode=ctx.resource_mode,
        prepared_resources=ctx.prepared_resources,
        prepared_resources_supplied=ctx.prepared_resources_supplied,
        source_sha=ctx.source_sha,
    )
    sibling.resume_state = ctx.resume_state
    return sibling


def _candidate_mismatch_detail(
    actual: Any,
    expected: Mapping[str, Any],
) -> str:
    if not isinstance(actual, dict):
        return "candidate contract is missing"
    labels = {
        "product": "product",
        "build_type": "build type",
        "platform": "platform",
        "plan_architectures": "plan architecture",
        "full_arch_plans": "resolved build plan",
        "versions": "version",
        "browseros_source": "BrowserOS source",
        "chromium_pin": "Chromium pin",
        "configuration": "build configuration",
        "resources": "resource identity",
    }
    for key, label in labels.items():
        if actual.get(key) != expected.get(key):
            return f"{label} mismatch"
    return "candidate contract digest mismatch"


def _mismatch(arch: str, step_name: str, detail: str) -> ResumeValidationError:
    return ResumeValidationError(
        _restart_message(
            f"Resume checkpoint mismatch for {arch}/{step_name}: {detail}",
            step_name,
        )
    )


def _restart_message(detail: str, step_name: str) -> str:
    return (
        f"{detail}. Safe restart: rerun with --from {step_name} after rebuilding "
        "that boundary, or rerun without --from to rebuild verified checkpoints."
    )


def _path_attestation(path: Path) -> dict[str, Any]:
    path = Path(path)
    if path.is_symlink():
        return {
            "type": "symlink",
            "target": os.readlink(path),
        }
    if path.is_file():
        return {
            "type": "file",
            "size": path.stat().st_size,
            "mode": path.stat().st_mode & 0o777,
            "sha256": _sha256(path),
        }
    if path.is_dir():
        return {
            "type": "directory",
            "digest": _directory_digest(path),
        }
    raise FileNotFoundError(path)


def _validate_attestation(
    attestation: Any,
    path: Path,
    arch: str,
    step_name: str,
    label: str,
) -> None:
    if not isinstance(attestation, dict):
        raise _mismatch(arch, step_name, f"{label} attestation is invalid")
    try:
        current = _path_attestation(path)
    except OSError as exc:
        raise _mismatch(arch, step_name, f"{label} is missing: {path}") from exc
    if current != attestation:
        raise _mismatch(arch, step_name, f"{label} checksum mismatch: {path}")


def _directory_digest(root: Path) -> str:
    entries = []
    for path in sorted(
        root.rglob("*"), key=lambda item: item.relative_to(root).as_posix()
    ):
        rel = path.relative_to(root).as_posix()
        if path.is_symlink():
            entries.append(
                {"type": "symlink", "path": rel, "target": os.readlink(path)}
            )
        elif path.is_file():
            entries.append(
                {
                    "type": "file",
                    "path": rel,
                    "size": path.stat().st_size,
                    "mode": path.stat().st_mode & 0o777,
                    "sha256": _sha256(path),
                }
            )
        elif path.is_dir():
            entries.append(
                {
                    "type": "dir",
                    "path": rel,
                    "mode": path.stat().st_mode & 0o777,
                }
            )
    return _json_digest(entries)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        while chunk := stream.read(_HASH_CHUNK_SIZE):
            digest.update(chunk)
    return digest.hexdigest()


def _file_identity(path: Path) -> dict[str, Any]:
    return {
        "path": str(path),
        "exists": path.exists(),
        "sha256": _sha256(path) if path.is_file() else "",
    }


def _download_resource_paths(ctx: Context) -> list[Path]:
    config = _load_yaml_mapping(ctx.get_download_resources_config())
    operations = config.get("download_operations")
    if not isinstance(operations, list):
        return []
    target_archs = [ctx.architecture]
    if ctx.architecture == "universal" or "universal" in ctx.plan_architectures:
        target_archs = ["arm64", "x64", "universal"]
    paths = []
    for op in operations:
        if isinstance(op, dict) and _operation_applies(op, ctx, target_archs):
            destination = op.get("destination")
            if isinstance(destination, str):
                paths.append(ctx.root_dir / destination)
    return sorted(set(paths))


def _managed_copy_destinations(ctx: Context) -> list[Path]:
    config = _load_yaml_mapping(ctx.get_copy_resources_config())
    operations = config.get("copy_operations")
    if not isinstance(operations, list):
        return []
    paths = []
    for op in operations:
        if not isinstance(op, dict) or not _operation_applies(
            op, ctx, [ctx.architecture]
        ):
            continue
        destination = op.get("destination")
        if isinstance(destination, str):
            paths.append(ctx.chromium_src / destination)
    return sorted(set(paths))


def _operation_applies(
    op: Mapping[str, Any],
    ctx: Context,
    target_archs: Sequence[str],
) -> bool:
    os_condition = op.get("os")
    if os_condition and get_platform() not in os_condition:
        return False
    arch_condition = op.get("arch")
    if arch_condition and not any(arch in arch_condition for arch in target_archs):
        return False
    build_type_condition = op.get("build_type")
    if build_type_condition and build_type_condition != ctx.build_type:
        return False
    product_condition = op.get("product")
    if product_condition is None or ctx.build_type == "debug":
        return True
    products = (
        [product_condition]
        if isinstance(product_condition, str)
        else product_condition
    )
    return isinstance(products, list) and ctx.product.id in products


def _load_yaml_mapping(path: Path) -> dict[str, Any]:
    try:
        with open(path, "r") as stream:
            data = yaml.safe_load(stream)
    except (OSError, yaml.YAMLError):
        return {}
    return data if isinstance(data, dict) else {}


def _chromium_mutation_digest(ctx: Context) -> dict[str, Any] | None:
    return _chromium_mutation_digest_for_root(ctx.chromium_src)


def _chromium_mutation_digest_for_root(root: Path) -> dict[str, Any] | None:
    if not (root / ".git").exists():
        return None
    paths = _git_changed_paths(root, excludes=_CHROMIUM_MUTATION_EXCLUDES)
    return {
        "kind": "chromium_mutation",
        "root": str(root),
        "paths": paths,
        "digest": _changed_paths_digest(root, paths),
    }


def _chromium_checkout_identity(ctx: Context) -> dict[str, str]:
    try:
        head = _git_text(["rev-parse", "HEAD"], ctx.chromium_src)
    except _Unresumable:
        return {}
    return {"head": head} if _is_full_sha(head) else {}


def _git_changed_paths(
    root: Path,
    excludes: Sequence[str] = (),
) -> list[str]:
    pathspec = ["."]
    pathspec.extend(f":(exclude){item}" for item in excludes)
    changed = set(
        _git_paths(["diff", "--name-only", "-z", "HEAD", "--", *pathspec], root)
    )
    changed.update(
        _git_paths(
            ["diff", "--cached", "--name-only", "-z", "HEAD", "--", *pathspec],
            root,
        )
    )
    changed.update(
        _git_paths(
            ["ls-files", "--others", "--exclude-standard", "-z", "--", *pathspec],
            root,
        )
    )
    return sorted(path for path in changed if path)


def _changed_paths_digest(root: Path, paths: Sequence[str]) -> str:
    entries = []
    for rel in sorted(paths):
        path = root / rel
        if path.is_symlink():
            entries.append(
                {"type": "symlink", "path": rel, "target": os.readlink(path)}
            )
        elif path.is_file():
            entries.append(
                {
                    "type": "file",
                    "path": rel,
                    "size": path.stat().st_size,
                    "mode": path.stat().st_mode & 0o777,
                    "sha256": _sha256(path),
                }
            )
        elif path.exists():
            entries.append({"type": "other", "path": rel})
        else:
            entries.append({"type": "deleted", "path": rel})
    return _json_digest(entries)


def _git_text(args: Sequence[str], cwd: Path) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise _Unresumable(
            f"git {' '.join(args)} failed in {cwd}: {result.stderr.strip()}"
        )
    return result.stdout.strip()


def _git_paths(args: Sequence[str], cwd: Path) -> list[str]:
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True)
    if result.returncode != 0:
        raise _Unresumable(
            f"git {' '.join(args)} failed in {cwd}: "
            f"{result.stderr.decode(errors='replace').strip()}"
        )
    return [
        item.decode("utf-8", errors="surrogateescape")
        for item in result.stdout.split(b"\0")
        if item
    ]


def _is_full_sha(value: str) -> bool:
    return len(value) == 40 and all(ch in "0123456789abcdefABCDEF" for ch in value)


def _atomic_write_json(path: Path, document: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp_name = ""
    try:
        with tempfile.NamedTemporaryFile(
            "w",
            encoding="utf-8",
            dir=path.parent,
            prefix=f".{path.name}.",
            delete=False,
        ) as stream:
            temp_name = stream.name
            json.dump(document, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        Path(temp_name).replace(path)
        _fsync_dir(path.parent)
    finally:
        if temp_name:
            Path(temp_name).unlink(missing_ok=True)


def _fsync_dir(path: Path) -> None:
    if os.name == "nt":
        return
    fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _json_digest(value: Any) -> str:
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def _json_safe(value: Any) -> bool:
    try:
        json.dumps(value)
    except (TypeError, ValueError):
        return False
    return True
