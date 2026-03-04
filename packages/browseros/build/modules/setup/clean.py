#!/usr/bin/env python3
"""Clean module for BrowserOS build system"""

from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.utils import run_command, log_info, log_success, safe_rmtree


class CleanModule(CommandModule):
    produces = []
    requires = []
    description = "Clean build artifacts and reset git state"

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

    def _clean_sparkle(self, ctx: Context) -> None:
        sparkle_dir = ctx.get_sparkle_dir()
        if sparkle_dir.exists():
            safe_rmtree(sparkle_dir)
        log_success("Cleaned Sparkle build directory")

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
