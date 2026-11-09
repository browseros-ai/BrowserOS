"""Validate an external Chromium patch package before build steps can use it.

The upstream pin, canonical store, ownership map, and platform series are one
input. Product resources and build configuration belong to the build package.
"""

from pathlib import Path
import re
import subprocess

import yaml

from ..lib.versions import load_chromium_version


def load_external_patch_version(root: Path) -> tuple[str, dict[str, str]]:
    """Validate the selected package's pin, store metadata, and series layout."""
    try:
        # A root symlink is resolved by Context. Links inside the package would
        # let mutable bytes escape the directory fingerprint used for resume.
        for name in (
            "CHROMIUM_VERSION",
            "BASE_COMMIT",
            "chromium_patches",
            "v8_patches",
            "series_patches",
        ):
            path = root / name
            _reject_symlink(path)
            if path.is_dir():
                for child in path.rglob("*"):
                    _reject_symlink(child)
        version, parts = load_chromium_version(root)
        if not re.fullmatch(r"\d+\.\d+\.\d+\.\d+", version):
            raise ValueError("CHROMIUM_VERSION must contain a four-part version")
        base = (root / "BASE_COMMIT").read_text(encoding="utf-8").strip()
        if not re.fullmatch(r"[0-9a-f]{40}", base):
            raise ValueError("BASE_COMMIT must contain a full commit SHA")
        store = root / "chromium_patches"
        metadata = yaml.safe_load((store / ".store.yaml").read_text(encoding="utf-8"))
        if not isinstance(metadata, dict) or (
            metadata.get("base_commit") != base
            or metadata.get("base_version") != version
        ):
            raise ValueError("BASE_COMMIT, CHROMIUM_VERSION, and .store.yaml disagree")
        _validate_ownership(store)
        load_v8_patch_base(root)
        if not (root / "series_patches" / "series").is_file():
            raise ValueError("series_patches/series is missing")
    except (OSError, ValueError, KeyError, yaml.YAMLError) as exc:
        raise ValueError(f"Invalid Chromium patch root {root}: {exc}") from exc
    return version, parts


def _reject_symlink(path: Path) -> None:
    if path.is_symlink():
        raise ValueError(f"symlinks are not supported in patch inputs: {path}")


def _validate_ownership(store: Path) -> None:
    data = yaml.safe_load((store / ".features.yaml").read_text(encoding="utf-8"))
    features = data.get("features") if isinstance(data, dict) else None
    if not isinstance(features, dict) or any(
        not isinstance(name, str)
        or not isinstance(feature, dict)
        or not isinstance(feature.get("files"), list)
        or not all(isinstance(path, str) for path in feature["files"])
        for name, feature in features.items()
    ):
        raise ValueError(
            f"{store}/.features.yaml must contain a feature map with file lists"
        )


def load_v8_patch_base(root: Path) -> str | None:
    """Validate the optional nested store without requiring a source checkout."""
    store = root / "v8_patches"
    if not store.exists():
        return None
    try:
        data = yaml.safe_load((store / ".store.yaml").read_text(encoding="utf-8"))
        if not isinstance(data, dict):
            raise ValueError(".store.yaml must contain a mapping")
        base, version = data.get("base_commit"), data.get("base_version")
        if not isinstance(base, str) or not re.fullmatch(r"[0-9a-f]{40}", base):
            raise ValueError("base_commit must contain a full commit SHA")
        if not isinstance(version, str) or not re.fullmatch(
            r"\d+\.\d+\.\d+\.\d+", version
        ):
            raise ValueError("base_version must contain a four-part version")
        _validate_ownership(store)
    except (OSError, ValueError, yaml.YAMLError) as exc:
        raise ValueError(f"Invalid V8 patch store {store}: {exc}") from exc
    return base


def validate_v8_gitlink(
    root: Path, chromium_src: Path, *, allow_missing_base: bool = False
) -> None:
    """Check a locally available Chromium pin before preparation mutates source."""
    v8_base = load_v8_patch_base(root)
    if v8_base is None:
        return
    base = (root / "BASE_COMMIT").read_text(encoding="utf-8").strip()
    if not (chromium_src / ".git").exists():
        available = False
    else:
        available = (
            subprocess.run(
                ["git", "cat-file", "-e", f"{base}^{{commit}}"],
                cwd=chromium_src,
                capture_output=True,
            ).returncode
            == 0
        )
    if not available:
        if allow_missing_base:
            return
        raise ValueError(f"Chromium patch base {base} is unavailable for V8 validation")
    pin = subprocess.run(
        ["git", "ls-tree", base, "--", "v8"],
        cwd=chromium_src,
        capture_output=True,
        text=True,
    )
    if pin.returncode != 0 or pin.stdout.split() != ["160000", "commit", v8_base, "v8"]:
        raise ValueError("v8_patches base does not match Chromium's pinned V8 gitlink")
