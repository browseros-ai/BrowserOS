#!/usr/bin/env python3
"""Clean module for BrowserOS build system"""

from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.utils import run_command, log_info, log_success, log_warning, safe_rmtree
from ..storage.download import managed_binary_families


@step("clean", phase="setup")
class CleanModule(Step):
    produces = []
    requires = []
    description = "Clean build artifacts, reset git state, and prune orphaned resources"

    def validate(self, ctx: Context) -> None:
        if not ctx.chromium_src.exists():
            raise ValidationError(f"Chromium source not found: {ctx.chromium_src}")

    def execute(self, ctx: Context) -> None:
        log_info("🧹 Cleaning build artifacts...")

        out_path = ctx.chromium_src / ctx.out_dir
        if out_path.exists():
            safe_rmtree(out_path)
            log_success("Cleaned build directory")

        log_info("\n🔀 Resetting git branch and removing tracked files...")
        self._git_reset(ctx)

        log_info("\n🧹 Cleaning Sparkle build artifacts...")
        self._clean_sparkle(ctx)

        log_info("\n🧹 Pruning orphaned resource binaries...")
        self._prune_orphan_binary_families(ctx)

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

    def _git_reset(self, ctx: Context) -> None:
        run_command(["git", "reset", "--hard", "HEAD"], cwd=ctx.chromium_src)

        # Reset all dirty submodules so gclient sync doesn't choke
        log_info("🧹 Resetting dirty submodules...")
        run_command(
            ["git", "submodule", "foreach", "--recursive",
             "git checkout -- . && git clean -fd"],
            cwd=ctx.chromium_src,
        )

        log_info("🧹 Running git clean with exclusions...")
        run_command(
            [
                "git",
                "clean",
                "-fdx",
                "chrome/",
                "components/",
                "third_party/",
                "--exclude=build_tools/",
                "--exclude=uc_staging/",
                "--exclude=buildtools/",
                "--exclude=tools/",
                "--exclude=build/",
            ],
            cwd=ctx.chromium_src,
        )
        log_success("Git reset and clean complete")
