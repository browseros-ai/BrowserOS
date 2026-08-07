#!/usr/bin/env python3
"""Patch-stack doctor: read-only health checks for .features.yaml ↔ chromium_patches/.

Answers "how healthy is the patch stack" without touching any tree:
repo-local consistency (every patch-backed feature entry resolves to a patch,
every patch is claimed, claims don't overlap) plus an optional dry-run
apply report against a chromium checkout. Pure functions returning
findings — callers render and decide exit codes.
"""

from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Dict, List, Optional, Set

from .batch_apply import MARKER_SUFFIXES, METADATA_ROOT_FILES
from .features_io import canonical_features_path, patch_backed_features
from .validation import validate_description, validate_feature_name

# Features whose description carries this prefix list files touched by
# series_patches/ quilt patches, not chromium_patches/ paths — they are
# exempt from patch-resolution checks and never claim on-disk patches.
SERIES_PREFIX = "series:"

UNCLASSIFIED = "(unclassified)"


@dataclass(frozen=True)
class Finding:
    check: str  # missing-patch | empty-dir | unclassified | multi-claim | invalid-feature
    severity: str  # error | warning
    message: str
    feature: Optional[str] = None
    path: Optional[str] = None


@dataclass(frozen=True)
class ApplyFailure:
    patch: str
    feature: str  # first claimant, or "(unclassified)"
    error: str


@dataclass(frozen=True)
class ApplyReport:
    against: str
    total: int
    clean: int
    failures: List[ApplyFailure]

    @property
    def features_affected(self) -> int:
        return len({failure.feature for failure in self.failures})


def is_series_feature(spec: Dict) -> bool:
    return str(spec.get("description") or "").startswith(SERIES_PREFIX)


def patch_base_paths(patches_dir: Path) -> Set[str]:
    """Relative paths of all patches on disk, markers mapped to their base path."""
    bases: Set[str] = set()
    if not patches_dir.exists():
        return bases
    for path in patches_dir.rglob("*"):
        if not path.is_file() or path.name.startswith("."):
            continue
        rel = path.relative_to(patches_dir).as_posix()
        if rel in METADATA_ROOT_FILES:
            continue
        for suffix in MARKER_SUFFIXES:
            if rel.endswith(suffix):
                rel = rel[: -len(suffix)]
                break
        bases.add(rel)
    return bases


def compute_claims(features: Dict, bases: Set[str]) -> Dict[str, List[str]]:
    """Map each on-disk base path to the sorted features claiming it."""
    claims: Dict[str, Set[str]] = {base: set() for base in bases}
    for name, spec in patch_backed_features(features).items():
        if is_series_feature(spec):
            continue
        for entry in spec.get("files") or []:
            if entry.endswith("/"):
                for base in bases:
                    if base.startswith(entry):
                        claims[base].add(name)
            elif entry in claims:
                claims[entry].add(name)
    return {base: sorted(owners) for base, owners in claims.items()}


def check_feature_metadata(
    features: Dict, feature: Optional[str] = None
) -> List[Finding]:
    findings = []
    for name, spec in patch_backed_features(features).items():
        if feature is not None and name != feature:
            continue
        valid, error = validate_feature_name(name)
        if not valid:
            findings.append(
                Finding("invalid-feature", "error", f"{name}: {error}", feature=name)
            )
        valid, error = validate_description(str(spec.get("description") or ""))
        if not valid:
            findings.append(
                Finding("invalid-feature", "error", f"{name}: {error}", feature=name)
            )
    return findings


def check_entries_resolve(
    features: Dict, bases: Set[str], feature: Optional[str] = None
) -> List[Finding]:
    findings = []
    for name, spec in patch_backed_features(features).items():
        if feature is not None and name != feature:
            continue
        if is_series_feature(spec):
            continue
        for entry in spec.get("files") or []:
            if entry.endswith("/"):
                if not any(base.startswith(entry) for base in bases):
                    findings.append(
                        Finding(
                            "empty-dir",
                            "error",
                            f"{name}: directory entry '{entry}' has no patches under it",
                            feature=name,
                            path=entry,
                        )
                    )
            elif entry not in bases:
                findings.append(
                    Finding(
                        "missing-patch",
                        "error",
                        f"{name}: no patch on disk for entry '{entry}'",
                        feature=name,
                        path=entry,
                    )
                )
    return findings


def check_classification(
    claims: Dict[str, List[str]], feature: Optional[str] = None
) -> List[Finding]:
    """Unclassified patches (error) and multi-claimed patches (warning)."""
    findings = []
    for base, owners in claims.items():
        if not owners:
            if feature is None:
                findings.append(
                    Finding(
                        "unclassified",
                        "error",
                        f"patch not claimed by any feature: {base}",
                        path=base,
                    )
                )
        elif len(owners) > 1 and (feature is None or feature in owners):
            findings.append(
                Finding(
                    "multi-claim",
                    "warning",
                    f"patch claimed by multiple features ({', '.join(owners)}): {base}",
                    path=base,
                )
            )
    return findings


def _require_known_feature(features: Dict, feature: Optional[str]) -> None:
    if feature is not None and feature not in features:
        raise ValueError(
            f"unknown feature '{feature}'. Valid: {', '.join(sorted(features))}"
        )


def check_repo(
    features: Dict, patches_dir: Path, feature: Optional[str] = None
) -> List[Finding]:
    """All repo-local checks; raises ValueError for an unknown feature filter."""
    features = patch_backed_features(features)
    _require_known_feature(features, feature)
    bases = patch_base_paths(patches_dir)
    claims = compute_claims(features, bases)
    findings = [
        *check_feature_metadata(features, feature),
        *check_entries_resolve(features, bases, feature),
        *check_classification(claims, feature),
    ]
    return sorted(findings, key=lambda f: (f.check, f.feature or "", f.path or ""))


def load_features(root_dir: Path) -> Dict:
    """Load and shape-validate the canonical patch-backed feature map."""
    from .features_io import load_features_yaml

    features_file = canonical_features_path(root_dir)
    data = load_features_yaml(features_file)
    if not isinstance(data, dict):
        raise ValueError(f"{features_file}: top level must be a mapping")
    features = data.get("features") or {}
    if not isinstance(features, dict):
        raise ValueError(f"{features_file}: 'features' must be a mapping")
    for name, spec in features.items():
        if not isinstance(name, str):
            # yaml parses bare on/yes/true/1 keys as non-strings
            raise ValueError(
                f"{features_file}: feature name must be a string (got {name!r})"
            )
        if not isinstance(spec, dict):
            raise ValueError(
                f"{features_file}: feature '{name}' must be a mapping "
                "with description/files"
            )
        files = spec.get("files") or []
        if not isinstance(files, list) or not all(
            isinstance(entry, str) for entry in files
        ):
            raise ValueError(
                f"{features_file}: feature '{name}' files must be a list of paths"
            )
    return patch_backed_features(features)


def diagnose_repo(root_dir: Path, feature: Optional[str] = None) -> List[Finding]:
    """Repo-local checks against a browseros package root."""
    return check_repo(load_features(root_dir), root_dir / "chromium_patches", feature)


def check_apply(
    features: Dict,
    patches_dir: Path,
    chromium_src: Path,
    feature: Optional[str] = None,
) -> ApplyReport:
    """Dry-run every patch against a chromium tree, grouping failures by feature."""
    from .batch_apply import check_patch_applies, find_patch_files

    features = patch_backed_features(features)
    _require_known_feature(features, feature)
    claims = compute_claims(features, patch_base_paths(patches_dir))
    total = 0
    failures = []
    for patch_path in find_patch_files(patches_dir):
        rel = patch_path.relative_to(patches_dir).as_posix()
        owners = claims.get(rel, [])
        if feature is not None and feature not in owners:
            continue
        total += 1
        ok, error = check_patch_applies(patch_path, chromium_src)
        if not ok:
            failures.append(
                ApplyFailure(
                    patch=rel,
                    feature=feature or (owners[0] if owners else UNCLASSIFIED),
                    error=(error or "").strip(),
                )
            )
    return ApplyReport(
        against=str(chromium_src),
        total=total,
        clean=total - len(failures),
        failures=failures,
    )


def build_report(
    root_dir: Path,
    features: Dict,
    findings: List[Finding],
    apply_report: Optional[ApplyReport] = None,
    feature: Optional[str] = None,
) -> Dict:
    """Assemble the stable machine-readable report consumed by --json and renderers."""
    from .batch_apply import find_patch_files

    features = patch_backed_features(features)
    errors = sum(1 for f in findings if f.severity == "error")
    warnings = sum(1 for f in findings if f.severity == "warning")
    report: Dict = {
        "root": str(root_dir),
        "feature": feature,
        "repo": {
            "patches": len(find_patch_files(root_dir / "chromium_patches")),
            "features": len(features),
            "errors": errors,
            "warnings": warnings,
            "findings": [asdict(f) for f in findings],
        },
        "apply": None,
        "healthy": errors == 0,
    }
    if apply_report is not None:
        report["apply"] = {
            "against": apply_report.against,
            "total": apply_report.total,
            "clean": apply_report.clean,
            "failed": len(apply_report.failures),
            "features_affected": apply_report.features_affected,
            "failures": [asdict(f) for f in apply_report.failures],
        }
        report["healthy"] = report["healthy"] and not apply_report.failures
    return report
