#!/usr/bin/env python3
"""merge_universal step: fold the per-arch release builds into one app.

Planner-emitted as run 3 of the universal pipeline (core/planner.
plan_runs): the two prior runs leave arm64 and x64 apps at their
deterministic out dirs, so inputs derive from product+arch — no
cross-run artifact plumbing. The merged app lands exactly where
ctx(universal).get_app_path() resolves; sign/package/upload then treat
it like any other build.
"""

import json
from pathlib import Path
from typing import Optional

from ...core.context import Context
from ...core.step import Step, ValidationError, step
from ..package.merge import merge_architectures
from ..storage.download import ARTIFACT_METADATA_NAME

UNIVERSAL_ARCHITECTURES = ("arm64", "x64")


def _universalizer_script(ctx: Context) -> Path:
    return ctx.root_dir / "bos_build/steps/package/universalizer_patched.py"


def _read_artifact_metadata(path: Path) -> Optional[dict]:
    """Parse an artifact-metadata.json, or None if absent/unreadable."""
    try:
        data = json.loads(path.read_bytes())
    except (OSError, ValueError):
        return None
    return data if isinstance(data, dict) else None


def _assert_server_bundles_aligned(binaries_dir: Path) -> None:
    """Fail if a family's darwin-arm64/-x64 bundles disagree on version.

    The universal merge folds both arch server bundles into one app. A
    skewed pair (fresh arm64 vs stale x64) either trips the universalizer
    on differing non-Mach-O files or, for all-Mach-O bundles, silently
    ships mismatched server versions (BrowserClaw 0.47.11). A correct
    universal build re-downloads both dirs, so this only fires on stale
    state left by a --from resume or --no-download flow. Families missing
    either metadata file are skipped (older layouts, non-artifact dirs).
    """
    if not binaries_dir.is_dir():
        return
    for family_dir in sorted(binaries_dir.iterdir()):
        if not family_dir.is_dir():
            continue
        arm_meta = _read_artifact_metadata(
            family_dir / "darwin-arm64" / ARTIFACT_METADATA_NAME
        )
        x64_meta = _read_artifact_metadata(
            family_dir / "darwin-x64" / ARTIFACT_METADATA_NAME
        )
        if arm_meta is None or x64_meta is None:
            continue
        arm_version = arm_meta.get("version")
        x64_version = x64_meta.get("version")
        if arm_version == x64_version:
            continue
        raise ValidationError(
            f"Skewed '{family_dir.name}' server bundle: "
            f"darwin-arm64 version {arm_version!r} "
            f"(generated {arm_meta.get('generatedAt', 'unknown')}) "
            f"!= darwin-x64 version {x64_version!r} "
            f"(generated {x64_meta.get('generatedAt', 'unknown')}). "
            "Re-run with resource downloads enabled so both arch bundles "
            "refresh from R2."
        )


def _arch_app_path(ctx: Context, arch: str) -> Path:
    """Sibling per-arch app path via Context, single-sourcing the out-dir scheme."""
    return Context(
        root_dir=ctx.root_dir,
        chromium_src=ctx.chromium_src,
        architecture=arch,
        build_type=ctx.build_type,
        product=ctx.product,
    ).get_app_path()


@step("merge_universal", phase="build", platforms=("macos",), optional=True)
class MergeUniversalModule(Step):
    produces = ["built_app"]
    requires = []
    description = "Merge arm64 + x64 release builds into a universal app"

    def preflight(self, ctx: Context) -> None:
        script = _universalizer_script(ctx)
        if not script.exists():
            raise ValidationError(f"Universalizer script not found: {script}")

    def validate(self, ctx: Context) -> None:
        # Input apps are produced by the previous runs of the universal
        # pipeline, so they are checked just-in-time here, not in preflight.
        if ctx.architecture != "universal":
            raise ValidationError(
                f"merge_universal needs a universal context, got '{ctx.architecture}'"
            )
        for arch in UNIVERSAL_ARCHITECTURES:
            app = _arch_app_path(ctx, arch)
            if not app.exists():
                raise ValidationError(f"{arch} app not found (build it first): {app}")
        _assert_server_bundles_aligned(ctx.root_dir / "resources" / "binaries")

    def execute(self, ctx: Context) -> None:
        arm64_app, x64_app = (
            _arch_app_path(ctx, arch) for arch in UNIVERSAL_ARCHITECTURES
        )
        output = ctx.get_app_path()
        if not merge_architectures(
            arch1_path=arm64_app,
            arch2_path=x64_app,
            output_path=output,
            universalizer_script=_universalizer_script(ctx),
        ):
            raise RuntimeError("Failed to merge architectures into universal app")
        ctx.artifact_registry.add("built_app", output)
