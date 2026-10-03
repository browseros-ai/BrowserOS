#!/usr/bin/env python3
"""Clean module for BrowserOS build system"""

from pathlib import Path

from ...core.resume import remove_checkpoint_dirs
from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.utils import log_info, log_success, log_warning, safe_rmtree
from ..storage.download import managed_binary_families
from ..source.provision import reset_source

UNIVERSAL_INPUT_ARCHITECTURES = ("arm64", "x64")


@step("clean", phase="setup")
class CleanModule(Step):
    produces = []
    requires = []
    description = "Clean build artifacts, reset git state, and prune orphaned resources"

    def validate(self, ctx: Context) -> None:
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

    def execute(self, ctx: Context) -> None:
        self.clean_outputs(ctx)
        reset_source(ctx.chromium_src)
        self._clean_sparkle(ctx)

    def clean_outputs(self, ctx: Context) -> None:
        """Remove product outputs/resources without deleting synced toolchains.

        Prepared CI handoffs call only this part before patching. The ordinary
        clean step additionally resets source and removes Sparkle staging.
        """
        log_info("🧹 Cleaning build artifacts...")

        for out_path in self._output_dirs(ctx):
            if not out_path.exists():
                continue
            safe_rmtree(out_path)
            log_success(
                f"Cleaned build directory: {out_path.relative_to(ctx.chromium_src)}"
            )
        remove_checkpoint_dirs(ctx, self._checkpoint_architectures(ctx))

        log_info("\n🧹 Pruning orphaned resource binaries...")
        self._prune_orphan_binary_families(ctx)

    def _output_dirs(self, ctx: Context) -> tuple[Path, ...]:
        dirs: list[Path] = []
        seen: set[Path] = set()
        for architecture in self._output_architectures(ctx):
            out_ctx = Context(
                root_dir=ctx.root_dir,
                chromium_src=ctx.chromium_src,
                architecture=architecture,
                build_type=ctx.build_type,
                product=ctx.product,
            )
            out_path = out_ctx.chromium_src / out_ctx.out_dir
            if out_path in seen:
                continue
            dirs.append(out_path)
            seen.add(out_path)
        return tuple(dirs)

    def _output_architectures(self, ctx: Context) -> tuple[str, ...]:
        if "universal" in ctx.plan_architectures:
            return UNIVERSAL_INPUT_ARCHITECTURES
        return (ctx.architecture,)

    def _checkpoint_architectures(self, ctx: Context) -> tuple[str, ...]:
        if "universal" in ctx.plan_architectures:
            return (*UNIVERSAL_INPUT_ARCHITECTURES, "universal")
        return (ctx.architecture,)

    def _prune_orphan_binary_families(self, ctx: Context) -> None:
        """Remove resources/binaries/<family> dirs the download config no longer lists.

        Retired families linger on persistent runners with stale per-arch
        metadata (the retired browseros_claw_server dir failed a BrowserClaw
        universal merge, run 29882827339). Only immediate child directories
        are pruned; loose files and family contents are left alone.
        """
        binaries_dir = ctx.root_dir / "resources" / "binaries"
        if not binaries_dir.is_dir():
            return

        config_path = ctx.get_download_resources_config()
        families = managed_binary_families(config_path)
        if not families:
            # Fail safe: an empty set means the managed families are unknown
            # (missing/malformed config), never that everything is an orphan.
            log_warning(
                f"No managed resource families found in {config_path}; "
                "skipping orphan pruning"
            )
            return

        for entry in sorted(binaries_dir.iterdir()):
            if entry.is_dir() and entry.name not in families:
                safe_rmtree(entry)
                log_success(f"Removed orphaned resource family: {entry.name}")

    def _clean_sparkle(self, ctx: Context) -> None:
        sparkle_dir = ctx.get_sparkle_dir()
        if sparkle_dir.exists():
            safe_rmtree(sparkle_dir)
        winsparkle_dir = ctx.get_winsparkle_dir()
        if winsparkle_dir.exists():
            safe_rmtree(winsparkle_dir)
        log_success("Cleaned Sparkle/WinSparkle build directories")


def clean_ci_outputs(src: Path) -> None:
    """Remove known product outputs/checkpoints from an owned CI source cache."""
    out = src / "out"
    if not out.is_dir():
        return
    for pattern in ("Default_browseros_*", "Default_browserclaw_*"):
        for path in out.glob(pattern):
            if path.is_symlink():
                path.unlink()
            elif path.is_dir():
                safe_rmtree(path)
    for product in ("browseros", "browserclaw"):
        path = out / ".browseros_resume" / product
        if path.is_symlink():
            path.unlink()
        elif path.exists():
            safe_rmtree(path)
