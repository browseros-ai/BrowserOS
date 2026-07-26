#!/usr/bin/env python3
"""Workspace helpers for extension builds: source checkout, version stamp,
build .env, and shell command execution. Port of the actions repo's
repo.py/manifest.py/builder.py with one behavior change — in-repo sources
build from the working tree instead of cloning this monorepo into itself."""

import json
import os
import re
import subprocess
from pathlib import Path
from typing import Callable, Iterable, List, Optional

from dotenv import set_key

from ...lib.utils import log_info, log_success, log_warning
from .specs import ExtensionSpec, ExternalRepoSource, InRepoSource

RunGit = Callable[..., None]

_URL_CREDENTIALS_RE = re.compile(r"://[^/@\s]+@")


def _redact_credentials(text: str) -> str:
    return _URL_CREDENTIALS_RE.sub("://***@", text)


def _format_git_error(args: List[str], stderr: str) -> str:
    """Git failure message with URL credentials masked.

    Clone args (and git's own stderr) can carry the GH_TOKEN-embedded URL;
    this string reaches Slack via the notify subscriber, which unlike the
    Actions log does no secret masking.
    """
    return _redact_credentials(f"git {' '.join(args)} failed: {stderr.strip()}")


def _run_git(args: List[str], cwd: Optional[Path] = None) -> None:
    result = subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True
    )
    if result.returncode != 0:
        raise RuntimeError(_format_git_error(args, result.stderr))


def clone_url(repo: str) -> str:
    """https clone URL; GH_TOKEN embeds as x-access-token for private repos."""
    token = os.environ.get("GH_TOKEN")
    if token:
        return f"https://x-access-token:{token}@github.com/{repo}.git"
    return f"https://github.com/{repo}.git"


def resolve_source(
    spec: ExtensionSpec,
    monorepo_root: Path,
    work_root: Path,
    branch_override: Optional[str],
    run_git: Optional[RunGit] = None,
) -> Path:
    """Return the source root to build from.

    In-repo specs build the monorepo working tree as-is (branch selection is
    the caller's checkout, e.g. the workflow's ref). External specs clone
    into work_root/repos/<name>, or fetch+reset an existing checkout.
    """
    git = run_git or _run_git
    source = spec.source

    if isinstance(source, InRepoSource):
        path = monorepo_root / source.path
        if not path.is_dir():
            raise FileNotFoundError(
                f"In-repo source for '{spec.name}' not found: {path}"
            )
        return path

    assert isinstance(source, ExternalRepoSource)
    branch = branch_override or source.branch
    dest = work_root / "repos" / source.repo.split("/")[-1]

    if dest.exists():
        log_info(f"Updating {source.repo} at {dest} ({branch})")
        git(["fetch", "origin", branch], cwd=dest)
        git(["checkout", branch], cwd=dest)
        git(["reset", "--hard", f"origin/{branch}"], cwd=dest)
    else:
        dest.parent.mkdir(parents=True, exist_ok=True)
        log_info(f"Cloning {source.repo} ({branch}) -> {dest}")
        git(["clone", "--branch", branch, clone_url(source.repo), str(dest)])
    return dest


def update_manifest_version(manifest_path: Path, version: str) -> None:
    """Stamp version into a JSON manifest/package.json, keeping other keys."""
    if not manifest_path.exists():
        raise FileNotFoundError(f"Manifest not found: {manifest_path}")

    manifest = json.loads(manifest_path.read_text())
    old = manifest.get("version", "unknown")
    manifest["version"] = version
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    log_success(f"{manifest_path.name}: version {old} -> {version}")


def require_env(name: str) -> str:
    """Return a non-placeholder env value or raise with the release convention."""
    value = os.environ.get(name, "").strip()
    # 0-1 chars is an unset or placeholder value ("0", "-"), never a real key.
    if len(value) <= 1:
        raise EnvironmentError(f"Missing or empty environment variable: {name}")
    return value


def write_env_file(
    directory: Path,
    names: Iterable[str],
    *,
    required_names: Iterable[str] = (),
) -> Path:
    """Recreate <directory>/.env from the named process env vars.

    Some bundler configs only read env from file (old builder.py constraint),
    so the build env is materialized next to the app. Required values are
    checked before replacing the file; other unset vars are skipped.
    """
    for name in required_names:
        require_env(name)

    env_path = directory / ".env"
    env_path.write_text("")
    for name in names:
        value = os.environ.get(name)
        if value:
            set_key(str(env_path), name, value)
        else:
            log_warning(f".env: skipped {name} (not set)")
    return env_path


def run_command(command: str, cwd: Path) -> None:
    """Run a spec's shell command (pre_build/build) in the source root."""
    log_info(f"$ {command}  (cwd: {cwd})")
    result = subprocess.run(command, shell=True, cwd=cwd)
    if result.returncode != 0:
        raise RuntimeError(
            f"Command failed with exit {result.returncode}: {command}"
        )
