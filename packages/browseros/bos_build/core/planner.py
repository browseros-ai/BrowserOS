#!/usr/bin/env python3
"""Pipeline planning: presets + switches replace per-config module lists.

The 20 release.*.yaml files this replaces encoded ~5 booleans as
hand-copied module lists. Pipeline shapes now live here as one pure
function; runtime variation is a flat Switches value (product, archs,
clean, provision, sign, upload) resolved CLI > profile > preset default.
The step orders produced are golden-tested against the old YAMLs.
"""

from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import yaml

from .step import all_steps
from ..lib.utils import get_platform, get_platform_arch

PRESETS = ("release", "debug")
PROVISION_MODES = ("none", "full", "shallow")
VALID_ARCHITECTURES = ("x64", "arm64", "universal")


@dataclass(frozen=True)
class Switches:
    """Resolved run configuration; None fields mean 'preset default'."""

    preset: str = "release"
    product: str = "browseros"
    architectures: Tuple[str, ...] = ()
    clean: Optional[bool] = None
    provision: Optional[str] = None
    download: Optional[bool] = None
    sign: Optional[bool] = None
    upload: Optional[bool] = None
    bundle_local_extensions: bool = False
    skip: Tuple[str, ...] = ()

    def resolved(self) -> "Switches":
        """Fill None fields with the preset's defaults."""
        if self.preset not in PRESETS:
            raise ValueError(
                f"Unknown preset '{self.preset}'. Valid: {', '.join(PRESETS)}"
            )
        defaults = _PRESET_DEFAULTS[self.preset]
        resolved = replace(
            self,
            architectures=self.architectures or (get_platform_arch(),),
            clean=self.clean if self.clean is not None else defaults["clean"],
            provision=self.provision or defaults["provision"],
            download=self.download if self.download is not None else True,
            sign=self.sign if self.sign is not None else defaults["sign"],
            upload=self.upload if self.upload is not None else defaults["upload"],
        )
        for arch in resolved.architectures:
            if arch not in VALID_ARCHITECTURES:
                raise ValueError(
                    f"Invalid architecture '{arch}'. "
                    f"Valid: {', '.join(VALID_ARCHITECTURES)}"
                )
        if resolved.provision not in PROVISION_MODES:
            raise ValueError(
                f"Invalid provision mode '{resolved.provision}'. "
                f"Valid: {', '.join(PROVISION_MODES)}"
            )
        registry = all_steps()
        for name in resolved.skip:
            # isinstance guards yaml surprises like `skip: [upload: true]`
            # (a dict entry), which would TypeError on the dict lookup.
            if not isinstance(name, str) or name not in registry:
                raise ValueError(
                    f"Unknown step '{name}' in skip. "
                    f"Valid steps: {', '.join(sorted(registry))}"
                )
        return resolved

    @property
    def build_type(self) -> str:
        return "debug" if self.preset == "debug" else "release"


_PRESET_DEFAULTS: Dict[str, Dict[str, Any]] = {
    "release": {"clean": True, "provision": "full", "sign": True, "upload": True},
    "debug": {"clean": False, "provision": "full", "sign": False, "upload": False},
}


def plan(switches: Switches, arch: str, platform: Optional[str] = None) -> List[str]:
    """Compose the ordered step list for one architecture.

    Encodes what the release.*.yaml matrix used to spell out per file:
    sparkle_setup is a macOS build dependency even unsigned; WinSparkle
    setup is a Windows build dependency; post-package sparkle_sign exists
    on signed Windows and macOS builds; unsigned Windows builds get
    mini_installer instead of sign_windows. Universal is not a flat
    pipeline — plan_runs() expands it into three sequential runs.
    """
    if arch == "universal":
        raise ValueError("universal is planned as multiple runs; use plan_runs()")
    platform = platform or get_platform()
    switches = switches.resolved()

    if switches.preset == "debug":
        steps = _plan_debug(switches, platform)
    else:
        steps = _plan_release(switches, platform)
    return _apply_skip(steps, switches.skip)


def plan_runs(
    switches: Switches, platform: Optional[str] = None
) -> List[Tuple[str, List[str]]]:
    """Per-run (arch, steps) plans — the shape cli/build.py executes.

    Universal expands into three sequential runs on one prepped chromium
    tree (arm64 build, x64 build, merge); every other architecture list
    maps to one flat plan() per arch.
    """
    platform = platform or get_platform()
    switches = switches.resolved()
    if "universal" in switches.architectures:
        if len(switches.architectures) > 1:
            raise ValueError("universal cannot be combined with other architectures")
        # plan() applies skip itself; the universal composer doesn't.
        return [
            (arch, _apply_skip(steps, switches.skip))
            for arch, steps in _plan_universal_runs(switches, platform)
        ]
    return [(arch, plan(switches, arch, platform)) for arch in switches.architectures]


def _plan_universal_runs(
    switches: Switches, platform: str
) -> List[Tuple[str, List[str]]]:
    """Universal = three runs sharing one chromium tree.

    Run 1 preps the tree once (repeating clean/git_setup/patches would
    reset it) and builds arm64; run 2 rebuilds the per-arch tail for x64
    (resources stages arch-specific binaries, so it repeats); run 3
    merges the pair into ctx(universal)'s app path and processes it like
    any other build. Error precedence preserved from the flat planner:
    preset, then platform, then sign.
    """
    if switches.preset == "debug":
        raise ValueError("universal architecture is not supported for debug builds")
    if platform != "macos":
        raise ValueError("universal architecture is only supported on macos")
    if not switches.sign:
        raise ValueError("universal builds are always signed; drop --no-sign")

    prep: List[str] = []
    prep.extend(_provision_steps(switches))
    prep.append("sparkle_setup")
    if switches.download:
        prep.append("download_resources")
    prep.extend(
        [
            "bundled_extensions",
            "chromium_replace",
            "string_replaces",
            "series_patches",
            "patches",
        ]
    )

    arch_tail = [
        "resources",
        "configure",
        "compile",
        "sign_macos",
        "package_macos",
        "sparkle_sign",
    ]
    merge_run = ["merge_universal", "sign_macos", "package_macos", "sparkle_sign"]
    if switches.upload:
        arch_tail.append("upload")
        merge_run.append("upload")

    return [
        ("arm64", prep + arch_tail),
        ("x64", list(arch_tail)),
        ("universal", merge_run),
    ]


def _plan_release(switches: Switches, platform: str) -> List[str]:
    steps: List[str] = []
    steps.extend(_provision_steps(switches))
    if platform == "macos":
        steps.append("sparkle_setup")
    if platform == "windows":
        # The release GN config always links WinSparkle.dll — the compile
        # needs the vendored library whether or not the build is signed
        # (ninja: 'third_party/winsparkle/x64/Release/WinSparkle.dll'
        # missing and no known rule to make it).
        steps.append("winsparkle_setup")

    if switches.download:
        steps.append("download_resources")
    steps.extend(
        [
            "resources",
            "bundled_extensions",
            "chromium_replace",
            "string_replaces",
            "series_patches",
            "patches",
        ]
    )

    steps.extend(["configure", "compile"])

    if platform == "macos" and switches.sign:
        steps.append("sign_macos")
    if platform == "windows":
        steps.append("sign_windows" if switches.sign else "mini_installer")

    steps.append(f"package_{platform}")

    if platform in ("macos", "windows") and switches.sign:
        steps.append("sparkle_sign")
    if switches.upload:
        steps.append("upload")
    return steps


def _plan_debug(switches: Switches, platform: str) -> List[str]:
    steps: List[str] = []
    steps.extend(_provision_steps(switches))
    if platform == "windows":
        steps.append("winsparkle_setup")
    if switches.download:
        steps.append("download_resources")
    steps.extend(
        [
            "resources",
            "bundled_extensions",
            "chromium_replace",
            "string_replaces",
            "patches",
            "configure",
            "compile",
        ]
    )
    if platform == "macos" and switches.sign:
        steps.append("sign_macos")
    if platform == "windows":
        steps.append("mini_installer")
    steps.append(f"package_{platform}")
    if switches.upload:
        steps.append("upload")
    return steps


def _provision_steps(switches: Switches) -> List[str]:
    """Provisioning prefix per strategy.

    shallow interleaves clean BETWEEN checkout and sync: clean deletes
    hook-managed toolchains (third_party/llvm-build) that sync restores.
    """
    if switches.provision == "shallow":
        steps = ["source_checkout"]
        if switches.clean:
            steps.append("clean")
        steps.append("source_sync")
        return steps

    steps = []
    if switches.clean:
        steps.append("clean")
    if switches.provision == "full":
        steps.append("git_setup")
    return steps


def _apply_skip(steps: List[str], skip: Tuple[str, ...]) -> List[str]:
    """Subtract skip AFTER composition: removing a step never re-triggers
    composition rules (skipping sign_windows won't add mini_installer)."""
    if not skip:
        return steps
    skipped = set(skip)
    return [s for s in steps if s not in skipped]


def slice_from(steps: List[str], start: str) -> List[str]:
    """Resume slice for --from: one run's composed plan from `start` onward.

    Applied after skip subtraction, so resuming from a skipped step fails
    with the absent-step error.
    """
    registry = all_steps()
    if start not in registry:
        raise ValueError(
            f"Unknown step '{start}'. Valid steps: {', '.join(sorted(registry))}"
        )
    if start not in steps:
        raise ValueError(
            f"Step '{start}' is not in the composed plan "
            f"({' → '.join(steps)}); nothing to resume from"
        )
    return steps[steps.index(start) :]


def slice_runs_from(
    runs: List[Tuple[str, List[str]]], start: str
) -> List[Tuple[str, List[str]]]:
    """Resume the run TIMELINE from `start`: runs execute sequentially, so
    earlier runs are dropped, the first run containing the step is sliced,
    and later runs stay whole (a universal merge failure resumes with just
    merge_universal → sign → package, no recompiles).
    """
    registry = all_steps()
    if start not in registry:
        raise ValueError(
            f"Unknown step '{start}'. Valid steps: {', '.join(sorted(registry))}"
        )
    for i, (arch, steps) in enumerate(runs):
        if start in steps:
            return [(arch, slice_from(steps, start))] + runs[i + 1 :]
    composed = "; ".join(f"{arch}: {' → '.join(steps)}" for arch, steps in runs)
    raise ValueError(
        f"Step '{start}' is not in the composed plan "
        f"({composed}); nothing to resume from"
    )


def required_env(step_names: List[str]) -> List[str]:
    """Union of env vars declared by the selected steps, in step order."""
    registry = all_steps()
    seen: List[str] = []
    for name in step_names:
        cls = registry.get(name)
        if cls is None:
            continue
        for var in cls.env:
            if var not in seen:
                seen.append(var)
    return seen


@dataclass(frozen=True)
class Profile:
    """Parsed profile file: saved switches, or an explicit modules pipeline."""

    switches: Switches
    modules: Optional[Tuple[str, ...]] = None
    build_type: Optional[str] = None


# Planner-owned keys: meaningless once modules: enumerates the pipeline.
_PLANNER_KEYS = (
    "preset",
    "clean",
    "provision",
    "download",
    "sign",
    "upload",
    "bundle_local_extensions",
    "skip",
)


def load_profile(path: Path) -> Profile:
    """Load a profile file.

    The default shape is saved CLI switches — no module lists, no
    inheritance. `modules:` is the explicit enumerated-pipeline opt-in
    ("you own this list now"): it replaces the planner, so every
    planner-owned key is rejected beside it and only product/arch/
    build_type remain. Unknown keys are rejected so typos fail loudly.
    """
    with open(path) as f:
        data = yaml.safe_load(f) or {}

    if not isinstance(data, dict):
        raise ValueError(f"Profile {path} must be a flat mapping")

    known = {"product", "arch", "modules", "build_type", *_PLANNER_KEYS}
    unknown = set(data) - known
    if unknown:
        raise ValueError(
            f"Unknown profile keys in {path}: {', '.join(sorted(unknown))}. "
            f"Valid: {', '.join(sorted(known))}"
        )

    if "modules" in data:
        return _load_modules_profile(path, data)

    if "build_type" in data:
        raise ValueError(
            f"Profile key 'build_type' in {path} requires 'modules:' — "
            "the preset owns build type otherwise"
        )

    return Profile(
        switches=Switches(
            preset=data.get("preset", "release"),
            product=data.get("product", "browseros"),
            architectures=_as_tuple(data.get("arch")),
            clean=data.get("clean"),
            provision=data.get("provision"),
            download=data.get("download"),
            sign=data.get("sign"),
            upload=data.get("upload"),
            bundle_local_extensions=data.get("bundle_local_extensions", False),
            skip=_as_tuple(data.get("skip")),
        )
    )


def _load_modules_profile(path: Path, data: Dict[str, Any]) -> Profile:
    banned = [k for k in _PLANNER_KEYS if k in data]
    if banned:
        raise ValueError(
            f"Profile keys {', '.join(banned)} in {path} do not combine with "
            "'modules:' — a modules profile owns its pipeline"
        )

    modules = data["modules"]
    if (
        not isinstance(modules, list)
        or not modules
        or not all(isinstance(m, str) for m in modules)
    ):
        raise ValueError(
            f"Profile key 'modules' in {path} must be a non-empty list of step names"
        )

    if isinstance(data.get("arch"), list):
        raise ValueError(
            f"Profile key 'arch' in {path} must be a single value with "
            "'modules:' — modules profiles are single-arch"
        )

    build_type = data.get("build_type")
    if build_type is not None and build_type not in ("debug", "release"):
        raise ValueError(
            f"Invalid build_type '{build_type}' in {path}. Valid: debug, release"
        )

    return Profile(
        switches=Switches(
            product=data.get("product", "browseros"),
            architectures=_as_tuple(data.get("arch")),
        ),
        modules=tuple(modules),
        build_type=build_type,
    )


def _as_tuple(value: Any) -> Tuple[str, ...]:
    if value is None:
        return ()
    if isinstance(value, list):
        return tuple(value)
    return (value,)


def preflight(step_names: List[str], platform: Optional[str] = None, ctx=None) -> None:
    """Static whole-pipeline checks before anything executes.

    Fails fast (listing every problem, not just the first) on: planned
    steps that don't apply to this platform, missing env vars from step
    metadata, and per-step preflight() hooks. Dynamic state produced
    mid-run is deliberately NOT checked here — that stays in each
    step's just-in-time validate().
    """
    import os

    platform = platform or get_platform()
    registry = all_steps()
    problems: List[str] = []

    for name in step_names:
        cls = registry.get(name)
        if cls is None:
            problems.append(f"unknown step: {name}")
            continue
        if cls.platforms is not None and platform not in cls.platforms:
            problems.append(
                f"step '{name}' does not apply to platform '{platform}' "
                f"(applies to: {', '.join(cls.platforms)})"
            )

    for var in required_env(step_names):
        if not os.environ.get(var):
            problems.append(f"missing required environment variable: {var}")

    if ctx is not None:
        for name in step_names:
            cls = registry.get(name)
            if cls is None:
                continue
            try:
                cls().preflight(ctx)
            except Exception as e:
                problems.append(f"preflight failed for '{name}': {e}")

    if problems:
        raise ValueError(
            "Preflight failed:\n" + "\n".join(f"  - {p}" for p in problems)
        )
