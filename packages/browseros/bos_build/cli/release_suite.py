#!/usr/bin/env python3
"""Commands for inspecting and reconciling family release transactions."""

import json
import os
import subprocess
from pathlib import Path
from typing import Optional

import typer

from ..lib.paths import get_package_root
from ..lib.env import EnvConfig
from ..lib.r2 import get_r2_client
from ..lib.utils import log_error
from ..release.suite import (
    GitHubSuiteBackend,
    SuiteRecord,
    SuiteRequest,
    inspect_transaction,
    merge_transaction,
    reconcile_transaction,
)
from ..release.suite_artifact import (
    R2ImmutableObjectBackend,
    publish_suite_browser_artifact,
)
from ..release.suite_rolling import (
    GitHubRollingReleaseBackend,
    RollingReleaseRequest,
    reconcile_rolling_release,
)


app = typer.Typer(
    help="Inspect, reconcile, and merge one BrowserOS family release transaction",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


def _repo_root(path: Optional[Path]) -> Path:
    return (path or get_package_root().parent.parent).resolve()


def _request(
    mode: str, source_sha: str, default_branch: str, dispatch_ref: str
) -> SuiteRequest:
    return SuiteRequest(
        mode=mode,  # type: ignore[arg-type]
        source_sha=source_sha,
        default_branch=default_branch,
        dispatch_ref=dispatch_ref,
    )


def _write_record(path: Optional[Path], record: SuiteRecord) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(record.to_json(), encoding="utf-8")


def _write_outputs(path: Optional[Path], record: SuiteRecord) -> None:
    output = path
    if output is None and os.environ.get("GITHUB_OUTPUT"):
        output = Path(os.environ["GITHUB_OUTPUT"])
    if output is None:
        return
    versions = record.component_versions
    values = {
        "transaction_id": record.transaction_id,
        "mode": record.mode,
        "source_sha": record.source_sha,
        "reservation_sha": record.reservation_sha,
        "state_sha": record.state_sha,
        "state_ref": record.state_ref(),
        "branch": record.branch,
        "browser_version": record.browser_version,
        "build_offset": str(record.build_offset),
        "server_version": versions["server"],
        "agent_version": versions["agent"],
        "claw_server_version": versions["claw-server-rust"],
        "browserclaw_version": versions["browserclaw"],
        # The historical output name belongs to BrowserOS neo. BrowserOS has a
        # separate onboarding app and therefore needs its own workflow output.
        "onboarding_version": versions["claw-onboard"],
        "app_onboarding_version": versions["app-onboard"],
        "component_versions": json.dumps(dict(versions), sort_keys=True),
        "pull_request_number": str(record.pull_request_number),
        "pull_request_url": record.pull_request_url,
        "state": record.state,
        "merge_sha": record.merge_sha,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    with open(output, "a", encoding="utf-8") as stream:
        for key, value in values.items():
            stream.write(f"{key}={value}\n")


def _write_summary(path: Optional[Path], record: SuiteRecord) -> None:
    output = path
    if output is None and os.environ.get("GITHUB_STEP_SUMMARY"):
        output = Path(os.environ["GITHUB_STEP_SUMMARY"])
    if output is None:
        return
    output.parent.mkdir(parents=True, exist_ok=True)
    with open(output, "a", encoding="utf-8") as stream:
        stream.write(record.summary())


def _emit(
    record: SuiteRecord,
    *,
    output: Optional[Path],
    github_output: Optional[Path],
    github_summary: Optional[Path],
) -> None:
    _write_record(output, record)
    _write_outputs(github_output, record)
    _write_summary(github_summary, record)
    typer.echo(record.to_json(), nl=False)


def _backend(repo_root: Optional[Path], repo: str, default_branch: str):
    return GitHubSuiteBackend(_repo_root(repo_root), repo, default_branch)


def _handle_error(exc: Exception) -> None:
    log_error(str(exc))
    raise typer.Exit(1)


@app.command("publish-browser-artifact")
def publish_browser_artifact(
    record_path: Path = typer.Option(..., "--record"),
    product: str = typer.Option(..., "--product"),
    artifact_root: Path = typer.Option(..., "--artifact-root"),
) -> None:
    """Conditionally publish one signed suite artifact and its exact receipt."""
    try:
        record = SuiteRecord.from_path(record_path)
        env = EnvConfig()
        if not env.has_r2_config():
            raise RuntimeError("R2 configuration not set")
        client = get_r2_client(env)
        if client is None:
            raise RuntimeError("Failed to create R2 client")
        publication = publish_suite_browser_artifact(
            record,
            product,
            artifact_root,
            R2ImmutableObjectBackend(client, env.r2_bucket),
        )
        typer.echo(publication.to_json(), nl=False)
    except (OSError, RuntimeError, TypeError, ValueError, json.JSONDecodeError) as exc:
        _handle_error(exc)


@app.command("reconcile-rolling-release")
def reconcile_rolling_release_command(
    tag: str = typer.Option(..., "--tag"),
    title: str = typer.Option(..., "--title"),
    source_sha: str = typer.Option(..., "--source-sha"),
    browser_version: str = typer.Option(..., "--browser-version"),
    artifact_root: Path = typer.Option(..., "--artifact-root"),
    repo: str = typer.Option(..., "--repo"),
    repo_root: Optional[Path] = typer.Option(None, "--repo-root"),
) -> None:
    """Monotonically create, resume, or verify one rolling nightly release."""
    try:
        artifacts = sorted(artifact_root.rglob("*.dmg"))
        if len(artifacts) != 1:
            raise ValueError(
                f"Rolling release expects exactly one DMG, found {len(artifacts)}"
            )
        request = RollingReleaseRequest(
            tag=tag,
            title=title,
            source_sha=source_sha,
            browser_version=browser_version,
            asset=artifacts[0],
        )
        outcome = reconcile_rolling_release(
            request,
            GitHubRollingReleaseBackend(repo, _repo_root(repo_root)),
        )
        typer.echo(outcome)
    except (
        OSError,
        RuntimeError,
        TypeError,
        ValueError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
    ) as exc:
        _handle_error(exc)


@app.command("inspect")
def inspect(
    mode: str = typer.Option(..., "--mode", help="Suite mode: nightly or full"),
    source_sha: str = typer.Option(..., "--source-sha"),
    default_branch: str = typer.Option(..., "--default-branch"),
    dispatch_ref: str = typer.Option(..., "--dispatch-ref"),
    repo: str = typer.Option(..., "--repo"),
    repo_root: Optional[Path] = typer.Option(None, "--repo-root"),
    output: Optional[Path] = typer.Option(None, "--output"),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
    github_summary: Optional[Path] = typer.Option(None, "--github-summary"),
) -> None:
    """Read the canonical transaction without allocating or changing it."""
    try:
        request = _request(mode, source_sha, default_branch, dispatch_ref)
        record = inspect_transaction(request, _backend(repo_root, repo, default_branch))
        _emit(
            record,
            output=output,
            github_output=github_output,
            github_summary=github_summary,
        )
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as exc:
        _handle_error(exc)


@app.command("reconcile")
def reconcile(
    mode: str = typer.Option(..., "--mode", help="Suite mode: nightly or full"),
    source_sha: str = typer.Option(..., "--source-sha"),
    default_branch: str = typer.Option(..., "--default-branch"),
    dispatch_ref: str = typer.Option(..., "--dispatch-ref"),
    repo: str = typer.Option(..., "--repo"),
    output: Path = typer.Option(..., "--output"),
    repo_root: Optional[Path] = typer.Option(None, "--repo-root"),
    state_root: Optional[Path] = typer.Option(
        None,
        "--state-root",
        help="Repository tree containing the complete final tracked snapshot set",
    ),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
    github_summary: Optional[Path] = typer.Option(None, "--github-summary"),
) -> None:
    """Create/recover the reservation and optionally reconcile final snapshots."""
    try:
        request = _request(mode, source_sha, default_branch, dispatch_ref)
        record = reconcile_transaction(
            request,
            _backend(repo_root, repo, default_branch),
            state_root=state_root.resolve() if state_root is not None else None,
        )
        _emit(
            record,
            output=output,
            github_output=github_output,
            github_summary=github_summary,
        )
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as exc:
        _handle_error(exc)


@app.command("merge")
def merge(
    record: Path = typer.Option(..., "--record"),
    gate: Path = typer.Option(..., "--gate"),
    repo: str = typer.Option(..., "--repo"),
    repo_root: Optional[Path] = typer.Option(None, "--repo-root"),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
    github_summary: Optional[Path] = typer.Option(None, "--github-summary"),
) -> None:
    """Exact-head squash-merge a transaction whose family gate passed."""
    try:
        current = SuiteRecord.from_path(record)
        gate_document = json.loads(gate.read_text(encoding="utf-8"))
        if not isinstance(gate_document, dict):
            raise ValueError("Suite gate must be a JSON object")
        merged = merge_transaction(
            current,
            gate_document,
            _backend(repo_root, repo, current.default_branch),
        )
        _emit(
            merged,
            output=record,
            github_output=github_output,
            github_summary=github_summary,
        )
    except (
        OSError,
        ValueError,
        RuntimeError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
    ) as exc:
        _handle_error(exc)
