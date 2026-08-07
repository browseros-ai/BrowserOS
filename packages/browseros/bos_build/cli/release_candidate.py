#!/usr/bin/env python3
"""Browser release candidate commands."""

import json
import os
import subprocess
from pathlib import Path
from typing import Optional

import typer

from ..lib.paths import get_package_root
from ..lib.utils import log_error
from ..release.candidate import (
    CandidateRecord,
    CandidateRequest,
    GitHubCandidateBackend,
    ensure_candidate,
    merge_candidate,
)


app = typer.Typer(
    help="Create, recover, and merge immutable browser release candidates",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


def _repo_root(path: Optional[Path]) -> Path:
    return (path or get_package_root().parent.parent).resolve()


def _write_record(path: Path, record: CandidateRecord) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(record.to_json(), encoding="utf-8")


def _write_outputs(path: Optional[Path], record: CandidateRecord) -> None:
    output = path
    if output is None and os.environ.get("GITHUB_OUTPUT"):
        output = Path(os.environ["GITHUB_OUTPUT"])
    if output is None:
        return
    values = {
        "parent_sha": record.parent_sha,
        "candidate_sha": record.candidate_sha,
        "branch": record.branch,
        "browser_version": record.browser_version,
        "merge_sha": record.merge_sha,
        "pull_request_number": str(record.pull_request_number),
        "pull_request_url": record.pull_request_url,
        "state": record.state,
    }
    with open(output, "a", encoding="utf-8") as stream:
        for key, value in values.items():
            stream.write(f"{key}={value}\n")


def _write_summary(path: Optional[Path], record: CandidateRecord) -> None:
    output = path
    if output is None and os.environ.get("GITHUB_STEP_SUMMARY"):
        output = Path(os.environ["GITHUB_STEP_SUMMARY"])
    if output is None:
        return
    output.parent.mkdir(parents=True, exist_ok=True)
    with open(output, "a", encoding="utf-8") as stream:
        stream.write(record.summary())


@app.command("ensure")
def ensure(
    product: str = typer.Option(..., "--product"),
    parent_sha: str = typer.Option(..., "--parent-sha"),
    default_branch: str = typer.Option(..., "--default-branch"),
    dispatch_ref: str = typer.Option(..., "--dispatch-ref"),
    repo: str = typer.Option(..., "--repo"),
    output: Path = typer.Option(..., "--output"),
    repo_root: Optional[Path] = typer.Option(None, "--repo-root"),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
    github_summary: Optional[Path] = typer.Option(None, "--github-summary"),
) -> None:
    """Create or recover one candidate and emit its immutable record."""
    try:
        root = _repo_root(repo_root)
        backend = GitHubCandidateBackend(root, repo, default_branch)
        record = ensure_candidate(
            CandidateRequest(
                product=product,
                parent_sha=parent_sha,
                default_branch=default_branch,
                dispatch_ref=dispatch_ref,
            ),
            backend,
        )
        _write_record(output, record)
        _write_outputs(github_output, record)
        _write_summary(github_summary, record)
        typer.echo(record.to_json(), nl=False)
    except (
        OSError,
        ValueError,
        RuntimeError,
        subprocess.CalledProcessError,
    ) as exc:
        log_error(str(exc))
        raise typer.Exit(1)


@app.command("merge")
def merge(
    record: Path = typer.Option(..., "--record"),
    gate: Path = typer.Option(..., "--gate"),
    repo: str = typer.Option(..., "--repo"),
    repo_root: Optional[Path] = typer.Option(None, "--repo-root"),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
    github_summary: Optional[Path] = typer.Option(None, "--github-summary"),
) -> None:
    """Merge an unchanged candidate after a complete browser gate."""
    try:
        candidate = CandidateRecord.from_path(record)
        gate_document = json.loads(gate.read_text(encoding="utf-8"))
        if not isinstance(gate_document, dict):
            raise ValueError("Gate record must be a JSON object")
        backend = GitHubCandidateBackend(
            _repo_root(repo_root), repo, candidate.default_branch
        )
        merged = merge_candidate(candidate, gate_document, backend)
        _write_record(record, merged)
        _write_outputs(github_output, merged)
        _write_summary(github_summary, merged)
        typer.echo(merged.to_json(), nl=False)
    except (
        OSError,
        ValueError,
        RuntimeError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
    ) as exc:
        log_error(str(exc))
        raise typer.Exit(1)
