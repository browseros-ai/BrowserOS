#!/usr/bin/env python3
"""Prepare pinned Chromium source for local builds, caches and CI workspaces.

This is the single reset/checkout/sync implementation. Callers own the checkout
lock for the entire operation (build adapters already hold it); only explicit
reset intent permits discarding source changes. Output cleanup is separate.
"""

import ast
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

GCLIENT_SPEC = (
    """solutions = [
  {
    "name": "src",
    "url": "%s",
    "deps_file": "DEPS",
    "managed": False,
    "custom_deps": {},
    "custom_vars": {},
  },
]
"""
    % CHROMIUM_SRC_URL
)

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


def _repair_cached_depot_tools(depot_tools: Path) -> None:
    """Normalize only line-ending drift in an explicitly disposable checkout."""
    worktree = subprocess.run(
        ["git", "rev-parse", "--is-inside-work-tree"],
        cwd=depot_tools,
        capture_output=True,
        text=True,
    )
    head = subprocess.run(
        ["git", "rev-parse", "--verify", "HEAD"],
        cwd=depot_tools,
        capture_output=True,
        text=True,
    )
    if (
        worktree.returncode != 0
        or worktree.stdout.strip() != "true"
        or head.returncode != 0
    ):
        raise RuntimeError(
            f"cached depot_tools is not a valid Git worktree: {depot_tools}"
        )

    index_state = subprocess.run(
        ["git", "ls-files", "-v", "-z"],
        cwd=depot_tools,
        capture_output=True,
        text=True,
    )
    if index_state.returncode != 0:
        raise RuntimeError(
            "could not inspect cached depot_tools index flags: "
            f"{index_state.stderr.strip()}"
        )
    unsafe_index_entries = [
        entry
        for entry in index_state.stdout.split("\0")
        if entry and not entry.startswith("H ")
    ]
    if unsafe_index_entries:
        markers = sorted({entry[0] for entry in unsafe_index_entries})
        raise RuntimeError(
            "cached depot_tools has non-default index flags "
            f"({', '.join(markers)}); refusing line-ending repair"
        )

    status = subprocess.run(
        ["git", "status", "--porcelain=v1", "--untracked-files=no"],
        cwd=depot_tools,
        capture_output=True,
        text=True,
    )
    if status.returncode != 0:
        raise RuntimeError(
            "could not inspect cached depot_tools tracked files: "
            f"{status.stderr.strip()}"
        )
    if not status.stdout:
        return

    for diff_args in (
        [
            "diff",
            "--quiet",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=none",
            "--ignore-cr-at-eol",
            "--",
        ],
        [
            "diff",
            "--cached",
            "--quiet",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=none",
            "--ignore-cr-at-eol",
            "--",
        ],
    ):
        diff = subprocess.run(["git", *diff_args], cwd=depot_tools)
        if diff.returncode == 1:
            raise RuntimeError(
                "cached depot_tools has substantive tracked changes; "
                "refusing line-ending repair"
            )
        if diff.returncode != 0:
            raise RuntimeError(
                "could not classify cached depot_tools changes "
                f"(git {' '.join(diff_args)} exited {diff.returncode})"
            )

    log_info("[source] Normalizing cached depot_tools line endings...")
    run(["git", "reset", "--hard", "HEAD"], cwd=depot_tools)

    verified = subprocess.run(
        ["git", "status", "--porcelain=v1", "--untracked-files=no"],
        cwd=depot_tools,
        capture_output=True,
        text=True,
    )
    if verified.returncode != 0 or verified.stdout:
        raise RuntimeError(
            "cached depot_tools line-ending repair did not produce "
            "a clean tracked worktree"
        )


def ensure_depot_tools(
    root: Path,
    *,
    repair_cached_depot_tools: bool = False,
) -> Path:
    depot_tools = root / "depot_tools"
    if not (depot_tools / ".git").exists():
        log_info("[source] Cloning depot_tools...")
        run(
            ["git", "clone", "--depth", "1", DEPOT_TOOLS_URL, str(depot_tools)],
            cwd=root,
        )
    else:
        log_info("[source] depot_tools already present")
        if repair_cached_depot_tools:
            _repair_cached_depot_tools(depot_tools)

    append_github_file("GITHUB_PATH", str(depot_tools))
    if sys.platform == "win32":
        append_github_file("GITHUB_ENV", "DEPOT_TOOLS_WIN_TOOLCHAIN=0")
    return depot_tools


def ensure_gclient_config(
    root: Path, required_cpus: tuple[str, ...] = (), *, source_name: str = "src"
) -> None:
    """Preserve custom solutions/vars and change only a literal target CPU list.

    .gclient is executable Python. We do not execute it while inspecting it or
    guess at dynamic assignments: unsupported CPU declarations fail before any
    source reset, rather than silently replacing developer configuration.
    """
    gclient_file = root / ".gclient"
    content = (
        gclient_file.read_text()
        if gclient_file.exists()
        else (
            GCLIENT_SPEC
            if source_name == "src"
            else GCLIENT_SPEC.replace('"name": "src"', f'"name": {source_name!r}')
        )
    )
    tree = ast.parse(content, filename=str(gclient_file))
    if required_cpus:
        writes = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Name)
            and node.id == "target_cpus"
            and isinstance(node.ctx, ast.Store)
        ]
        assignments = [
            node
            for node in tree.body
            if isinstance(node, ast.Assign)
            and len(node.targets) == 1
            and isinstance(node.targets[0], ast.Name)
            and node.targets[0].id == "target_cpus"
        ]
        if writes and (len(writes) != 1 or len(assignments) != 1):
            raise ValueError(
                f"{gclient_file}: target_cpus must be one top-level literal list"
            )
        if assignments:
            assignment = assignments[0]
            try:
                cpus = ast.literal_eval(assignment.value)
            except (ValueError, TypeError) as exc:
                raise ValueError(
                    f"{gclient_file}: dynamic target_cpus is unsupported"
                ) from exc
            if not isinstance(cpus, (list, tuple)) or not all(
                isinstance(cpu, str) for cpu in cpus
            ):
                raise ValueError(f"{gclient_file}: target_cpus must contain strings")
            merged = sorted(set(cpus) | set(required_cpus))
            if set(merged) != set(cpus):
                # AST columns are UTF-8 byte offsets. Replace only the value,
                # retaining comments and every unrelated setting byte-for-byte.
                lines = content.encode().splitlines(keepends=True)
                node = assignment.value
                assert node.end_lineno is not None and node.end_col_offset is not None
                first = sum(map(len, lines[: node.lineno - 1])) + node.col_offset
                last = sum(map(len, lines[: node.end_lineno - 1])) + node.end_col_offset
                data = content.encode()
                content = (data[:first] + repr(merged).encode() + data[last:]).decode()
        else:
            content += f"\ntarget_cpus = {list(required_cpus)!r}\n"
    if not gclient_file.exists() or gclient_file.read_text() != content:
        log_info(f"[source] Updating {gclient_file}")
        gclient_file.write_text(content)


def _git_output(args, cwd: Path) -> str:
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else ""


def _managed_repositories(src: Path) -> list[Path]:
    """Use gclient's last inventory, including dependencies removed by a new pin.

    Dependencies are often standalone Git repositories, not Git submodules.
    Missing entries after an interrupted first sync are reconciled by the final
    `sync --reset --force`; existing submodules are also reset separately.
    """
    entries_path = src.parent / ".gclient_entries"
    if not entries_path.exists():
        return []
    tree = ast.parse(entries_path.read_text(), filename=str(entries_path))
    entries = None
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(
            isinstance(target, ast.Name) and target.id == "entries"
            for target in node.targets
        ):
            entries = ast.literal_eval(node.value)
    if not isinstance(entries, dict):
        raise ValueError(f"Invalid gclient dependency inventory: {entries_path}")
    repositories = []
    for name in entries:
        path = (src.parent / name).resolve()
        if path == src:
            continue
        if not path.is_relative_to(src):
            raise ValueError(f"gclient dependency escapes Chromium src: {name}")
        if (path / ".git").exists():
            repositories.append(path)
    return sorted(set(repositories), key=lambda path: len(path.parts), reverse=True)


def reset_source(src: Path) -> None:
    """Discard source changes without claiming dependencies are synchronized.

    Used both by explicit `clean` maintenance and prepare-with-reset. Inventory
    validation precedes mutation; ignored hook-managed files are removed only in
    the historical source-clean paths, and sync/hooks must follow this operation.
    """
    src = src.resolve()
    repositories = _managed_repositories(src)
    run(["git", "reset", "--hard", "HEAD"], cwd=src)
    for repo in repositories:
        run(["git", "reset", "--hard", "HEAD"], cwd=repo)
        run(["git", "clean", "-fd"], cwd=repo)
    run(
        [
            "git",
            "submodule",
            "foreach",
            "--recursive",
            "git reset --hard HEAD && git clean -fd",
        ],
        cwd=src,
    )
    run(["git", "clean", "-fd"], cwd=src)
    run(
        [
            "git",
            "clean",
            "-fdx",
            "--exclude=build_tools/",
            "--exclude=uc_staging/",
            "--exclude=buildtools/",
            "--exclude=tools/",
            "--exclude=build/",
            "--",
            "chrome/",
            "components/",
            "third_party/",
        ],
        cwd=src,
    )


def _fetch_pin(src: Path, version: str, strategy: str) -> Path:
    """Acquire the exact pin without changing an existing working tree."""
    if strategy not in STRATEGIES:
        raise ValueError(f"Unknown strategy '{strategy}'. Valid: {STRATEGIES}")
    root = src.parent
    tag_ref = f"refs/tags/{version}"
    # Validate before passing a pin from a file to Git as a refspec.
    run(["git", "check-ref-format", tag_ref], cwd=root)
    fresh = not (src / ".git").exists()
    if fresh:
        if src.exists() and any(src.iterdir()):
            raise ValueError(f"Refusing to initialize nonempty source directory: {src}")
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
        # --depth on an existing full clone would truncate its history. Shallow
        # is an acquisition policy, never permission to shrink a developer repo.
        shallow = (
            _git_output(["rev-parse", "--is-shallow-repository"], cwd=src) == "true"
        )
        if strategy == "shallow" and (
            fresh
            or shallow
            or not _git_output(["rev-parse", "--verify", "HEAD"], cwd=src)
        ):
            fetch += ["--depth", "2"]
        fetch += ["--no-tags", "origin", f"+{tag_ref}:{tag_ref}"]
        run(fetch, cwd=src)
    return src


def checkout(
    src: Path,
    version: str,
    strategy: str = "shallow",
    *,
    reset: bool = False,
    branch: Optional[str] = None,
) -> Path:
    """Select only the requested tag; preserve work unless reset is explicit."""
    src = _fetch_pin(src, version, strategy)
    tag_ref = f"refs/tags/{version}"
    if reset and _git_output(["rev-parse", "--verify", "HEAD"], cwd=src):
        reset_source(src)
    if branch:
        run(["git", "checkout", "-B", branch, tag_ref], cwd=src)
    else:
        run(["git", "checkout", "--detach", tag_ref], cwd=src)
    return src


def sync(src: Path, depot_tools: Optional[Path] = None, *, reset: bool = False) -> None:
    """Restore dependencies and run hooks after all destructive source cleanup."""
    depot_tools = depot_tools or src.parent / "depot_tools"
    env = os.environ.copy()
    env["PATH"] = str(depot_tools) + os.pathsep + env.get("PATH", "")
    env["DEPOT_TOOLS_WIN_TOOLCHAIN"] = "0"
    gclient = depot_tools / ("gclient.bat" if sys.platform == "win32" else "gclient")
    args = [str(gclient), "sync", "-D", "--no-history", "--shallow"]
    if reset:
        # Force also visits unchanged revisions and partially populated caches.
        args += ["--reset", "--force"]
    run(args, cwd=src, env=env)


def prepare(
    src: Path,
    version: str,
    strategy: str = "shallow",
    step_name: str = "all",
    repair_cached_depot_tools: bool = False,
    *,
    reset: bool = False,
    branch: Optional[str] = None,
) -> Path:
    """Prepare source under the caller's checkout lock and return its path.

    Only `all` promises prepared source. Legacy split adapters remain available;
    they do not advertise a complete handoff. No-clean conflicts propagate from
    Git/gclient without escalating to destructive recovery.
    """
    if step_name not in ("fetch", "checkout", "sync", "all"):
        raise ValueError(f"Unknown source step: {step_name}")
    if strategy not in STRATEGIES:
        raise ValueError(f"Unknown strategy: {strategy}")
    # Resolve the selected source itself before deriving its parent. Resolving
    # only the parent loses relative/symlink identity and can mutate a sibling
    # checkout outside the lock held by the build command.
    src = src.expanduser().resolve()
    root = src.parent
    if src == root or (src.exists() and not src.is_dir()):
        raise ValueError(f"Invalid Chromium source directory: {src}")
    if src.exists() and not (src / ".git").exists() and any(src.iterdir()):
        raise ValueError(
            f"Refusing to provision a nonempty non-Git source directory: {src}"
        )
    root.mkdir(parents=True, exist_ok=True)
    ensure_gclient_config(
        root,
        ("x64", "arm64") if sys.platform.startswith("linux") else (),
        source_name=src.name,
    )
    depot_tools = ensure_depot_tools(
        root, repair_cached_depot_tools=repair_cached_depot_tools
    )
    if step_name == "fetch":
        _fetch_pin(src, version, strategy)
        if not _git_output(["rev-parse", "--verify", "HEAD"], cwd=src):
            checkout(src, version, strategy)
    elif step_name in ("checkout", "all"):
        checkout(src, version, strategy, reset=reset, branch=branch)
    elif reset:
        reset_source(src)
    if step_name in ("sync", "all"):
        sync(src, depot_tools, reset=reset)
        head = _git_output(["rev-parse", "HEAD"], cwd=src)
        pin = _git_output(
            ["rev-parse", "--verify", f"refs/tags/{version}^{{commit}}"], cwd=src
        )
        if not pin or head != pin:
            raise RuntimeError(
                f"Chromium HEAD {head} does not match pin {version} ({pin}) after sync"
            )
        if branch and _git_output(["branch", "--show-current"], cwd=src) != branch:
            raise RuntimeError(
                f"gclient sync changed the requested local branch: {branch}"
            )
        log_success(f"Prepared Chromium {version}: {src} ({head})")
    elif step_name == "fetch":
        log_success(
            f"Chromium pin available (selection and sync still required): {src}"
        )
    else:
        log_success(f"Chromium checkout selected (sync still required): {src}")
    return src


def ensure(
    root: Path,
    version: str,
    strategy: str = "shallow",
    step_name: str = "all",
    repair_cached_depot_tools: bool = False,
    *,
    reset: bool = False,
    branch: Optional[str] = None,
) -> Path:
    """Root-based provisioning adapter used by the source CLI and CI cache.

    Builds call prepare with their exact source path; root-based callers retain
    the public gclient-root/src layout without duplicating preparation logic.
    """
    return prepare(
        root.expanduser() / "src",
        version,
        strategy,
        step_name,
        repair_cached_depot_tools,
        reset=reset,
        branch=branch,
    )


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
    description = "Acquire shallow Chromium source before optional cleanup"

    def execute(self, ctx: Context) -> None:
        # The legacy planner orders source_checkout -> optional clean ->
        # source_sync. Stage the pin without switching an existing dirty tree;
        # source_sync selects it after clean. A fresh repo needs an initial
        # HEAD so the unchanged clean maintenance step can reset it.
        prepare(
            ctx.chromium_src,
            ctx.chromium_version,
            strategy="shallow",
            step_name="fetch",
        )


@step("source_sync", phase="source", optional=True)
class SourceSyncModule(_SourceStep):
    description = "gclient sync the provisioned chromium checkout"

    def execute(self, ctx: Context) -> None:
        prepare(
            ctx.chromium_src,
            ctx.chromium_version,
            strategy="shallow",
            step_name="all",
        )
