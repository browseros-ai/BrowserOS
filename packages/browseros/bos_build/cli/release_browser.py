#!/usr/bin/env python3
"""Attest and finalize full browser releases."""

import json
import os
import subprocess
from pathlib import Path
from typing import List, Optional

import typer

from ..lib.utils import log_error
from ..release.browser_finalize import (
    GitHubDraftBackend,
    finalize_browser_release,
)
from ..release.candidate import CandidateRecord
from ..release.common import fetch_all_release_metadata
from ..release.lane import LaneGate, LaneManifest, gate_lane_manifests


app = typer.Typer(
    help="Gate native browser lanes and create an attested draft",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


def _optional_output(path: Optional[Path], environment_name: str) -> Optional[Path]:
    if path is not None:
        return path
    value = os.environ.get(environment_name)
    return Path(value) if value else None


def _append(path: Optional[Path], content: str) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "a", encoding="utf-8") as stream:
        stream.write(content)


def _write_values(path: Optional[Path], values: dict[str, str]) -> None:
    if path is None:
        return
    _append(path, "".join(f"{name}={value}\n" for name, value in values.items()))


@app.command("gate")
def gate(
    lanes: List[Path] = typer.Option(..., "--lane", help="Lane manifest (repeatable)"),
    output: Path = typer.Option(..., "--output"),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
    github_summary: Optional[Path] = typer.Option(None, "--github-summary"),
) -> None:
    """Require a complete, consistent native release matrix."""
    try:
        evidence = gate_lane_manifests(
            [LaneManifest.from_path(path) for path in lanes]
        )
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(evidence.to_json(), encoding="utf-8")
        _write_values(
            _optional_output(github_output, "GITHUB_OUTPUT"),
            {
                "product": evidence.product,
                "candidate_sha": evidence.candidate_sha,
                "browser_version": evidence.browser_version,
                "gate": str(output),
            },
        )
        _append(
            _optional_output(github_summary, "GITHUB_STEP_SUMMARY"), evidence.summary()
        )
        typer.echo(evidence.to_json(), nl=False)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        log_error(str(exc))
        raise typer.Exit(1)


@app.command("finalize")
def finalize(
    candidate_path: Path = typer.Option(..., "--candidate"),
    gate_path: Path = typer.Option(..., "--gate"),
    repo: str = typer.Option(..., "--repo"),
    preview_dir: Path = typer.Option(..., "--preview-dir"),
    output: Path = typer.Option(..., "--output"),
    github_output: Optional[Path] = typer.Option(None, "--github-output"),
    github_summary: Optional[Path] = typer.Option(None, "--github-summary"),
) -> None:
    """Reconcile a browser draft and render local appcast previews."""
    try:
        candidate = CandidateRecord.from_path(candidate_path)
        evidence = LaneGate.from_path(gate_path)
        metadata = fetch_all_release_metadata(
            candidate.browser_version, product_id=candidate.product
        )
        record = finalize_browser_release(
            candidate,
            evidence,
            metadata,
            preview_dir,
            GitHubDraftBackend(repo),
        )
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(record.to_json(), encoding="utf-8")
        _write_values(
            _optional_output(github_output, "GITHUB_OUTPUT"),
            {
                "release_tag": record.draft.tag,
                "release_url": record.draft.url,
                "candidate_sha": record.candidate_sha,
                "merge_sha": record.merge_sha,
                "finalization": str(output),
            },
        )
        _append(
            _optional_output(github_summary, "GITHUB_STEP_SUMMARY"), record.summary()
        )
        typer.echo(record.to_json(), nl=False)
    except (
        OSError,
        ValueError,
        RuntimeError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
    ) as exc:
        log_error(str(exc))
        raise typer.Exit(1)
