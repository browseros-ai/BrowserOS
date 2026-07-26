#!/usr/bin/env python3
"""Provision a Chromium checkout at the pinned version.

Absorbs scripts/ci/setup_chromium.py. Two strategies:

- shallow: ephemeral runners. Fetch exactly the pinned tag at depth 2
  (git_setup's `git fetch --tags` would pull objects for all ~70k
  chromium tags on a shallow clone). Depth 2 so positioning tools
  (git describe / lastchange) have a parent commit.
- full: fresh long-lived machines. Same flow with full-depth tag fetch.
  Existing developer checkouts keep using the git_setup step, which
  additionally maintains the local `browseros` branch.

checkout and sync are separate steps because the clean step must run
between them: clean deletes hook-managed toolchains (third_party/
llvm-build) that gclient sync then restores.
"""

import os
import subprocess
import sys
from pathlib import Path
from typing import Optional

from ...core.context import Context
from ...core.step import Step, ValidationError, step
from ...lib.utils import log_info, log_success

CHROMIUM_SRC_URL = "https://chromium.googlesource.com/chromium/src.git"
DEPOT_TOOLS_URL = "https://chromium.googlesource.com/chromium/tools/depot_tools.git"

GCLIENT_SPEC = """solutions = [
  {
    "name": "src",
    "url": "%s",
    "deps_file": "DEPS",
    "managed": False,
    "custom_deps": {},
    "custom_vars": {},
  },
]
""" % CHROMIUM_SRC_URL

STRATEGIES = ("shallow", "full")


def run(cmd, cwd: Path, env: Optional[dict] = None) -> None:
    log_info(f"[source] $ {' '.join(str(c) for c in cmd)}  (cwd={cwd})")
    subprocess.run(cmd, cwd=cwd, env=env, check=True)


def read_pinned_version(version_file: Path) -> str:
    """Parse MAJOR=/MINOR=/BUILD=/PATCH= lines into a version string."""
    parts = {}
    for line in version_file.read_text().strip().splitlines():
        key, value = line.split("=")
        parts[key.strip()] = value.strip()
    return f"{parts['MAJOR']}.{parts['MINOR']}.{parts['BUILD']}.{parts['PATCH']}"


def append_github_file(env_var: str, line: str) -> None:
    """Propagate PATH/env additions to later GitHub Actions steps."""
    path = os.environ.get(env_var)
    if not path:
        return
    with open(path, "a") as f:
        f.write(line + "\n")


def ensure_depot_tools(root: Path) -> Path:
    depot_tools = root / "depot_tools"
    if not (depot_tools / ".git").exists():
        log_info("[source] Cloning depot_tools...")
        run(
            ["git", "clone", "--depth", "1", DEPOT_TOOLS_URL, str(depot_tools)],
            cwd=root,
        )
    else:
        log_info("[source] depot_tools already present")

    append_github_file("GITHUB_PATH", str(depot_tools))
    if sys.platform == "win32":
        append_github_file("GITHUB_ENV", "DEPOT_TOOLS_WIN_TOOLCHAIN=0")
    return depot_tools


def ensure_gclient_config(root: Path) -> None:
    gclient_file = root / ".gclient"
    if gclient_file.exists() and gclient_file.read_text() == GCLIENT_SPEC:
        return
    log_info(f"[source] Writing {gclient_file}")
    gclient_file.write_text(GCLIENT_SPEC)


def _git_output(args, cwd: Path) -> str:
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else ""


def checkout(root: Path, version: str, strategy: str = "shallow") -> Path:
    """Ensure root/src is a checkout detached at the pinned tag."""
    if strategy not in STRATEGIES:
        raise ValueError(f"Unknown strategy '{strategy}'. Valid: {STRATEGIES}")

    src = root / "src"
    tag_ref = f"refs/tags/{version}"

    if not (src / ".git").exists():
        log_info(f"[source] Initializing fresh src checkout at {src}")
        src.mkdir(parents=True, exist_ok=True)
        run(["git", "init"], cwd=src)
        run(["git", "remote", "add", "origin", CHROMIUM_SRC_URL], cwd=src)
        if sys.platform == "win32":
            run(["git", "config", "core.longpaths", "true"], cwd=src)

    if not _git_output(
        ["rev-parse", "--verify", "--quiet", f"{tag_ref}^{{commit}}"], cwd=src
    ):
        log_info(f"[source] Fetching pinned tag {version} ({strategy})...")
        fetch = ["git", "fetch"]
        if strategy == "shallow":
            fetch += ["--depth", "2"]
        fetch += ["--no-tags", "origin", f"+{tag_ref}:{tag_ref}"]
        run(fetch, cwd=src)
    else:
        log_info(f"[source] Tag {version} already present")

    head = _git_output(["rev-parse", "HEAD"], cwd=src)
    tag_commit = _git_output(["rev-parse", f"{tag_ref}^{{commit}}"], cwd=src)
    if head != tag_commit:
        log_info(f"[source] Checking out {version}...")
        run(["git", "checkout", "--force", "--detach", tag_ref], cwd=src)
    else:
        log_info(f"[source] HEAD already at {version}")

    return src


def sync(root: Path, depot_tools: Optional[Path] = None) -> None:
    """gclient sync -D --no-history --shallow (matches git_setup's sync)."""
    depot_tools = depot_tools or root / "depot_tools"
    env = os.environ.copy()
    env["PATH"] = str(depot_tools) + os.pathsep + env.get("PATH", "")
    env["DEPOT_TOOLS_WIN_TOOLCHAIN"] = "0"

    gclient = depot_tools / ("gclient.bat" if sys.platform == "win32" else "gclient")
    run(
        [str(gclient), "sync", "-D", "--no-history", "--shallow"],
        cwd=root / "src",
        env=env,
    )


def ensure(
    root: Path,
    version: str,
    strategy: str = "shallow",
    step_name: str = "all",
) -> None:
    """Idempotently provision root: depot_tools + .gclient + src @ pin.

    step_name: "checkout", "sync", or "all" — split so callers can run
    the clean step between checkout and sync.
    """
    root.mkdir(parents=True, exist_ok=True)
    depot_tools = ensure_depot_tools(root)
    ensure_gclient_config(root)

    if step_name in ("checkout", "all"):
        src = checkout(root, version, strategy)
        log_success(f"Checkout ready: {src}")
    if step_name in ("sync", "all"):
        sync(root, depot_tools)
        log_success("gclient sync complete")


class _SourceStep(Step):
    """Pipeline adapters: gclient root is the parent of chromium_src.

    In-pipeline provisioning always uses the shallow strategy — it
    exists for ephemeral runners; dev boxes keep git_setup.
    """

    def validate(self, ctx: Context) -> None:
        if not ctx.chromium_version:
            raise ValidationError("Chromium version not set")


@step("source_checkout", phase="source", optional=True)
class SourceCheckoutModule(_SourceStep):
    description = "Provision depot_tools + chromium src at the pinned tag (shallow)"

    def execute(self, ctx: Context) -> None:
        ensure(
            ctx.chromium_src.parent,
            ctx.chromium_version,
            strategy="shallow",
            step_name="checkout",
        )


@step("source_sync", phase="source", optional=True)
class SourceSyncModule(_SourceStep):
    description = "gclient sync the provisioned chromium checkout"

    def execute(self, ctx: Context) -> None:
        ensure(
            ctx.chromium_src.parent,
            ctx.chromium_version,
            strategy="shallow",
            step_name="sync",
        )
