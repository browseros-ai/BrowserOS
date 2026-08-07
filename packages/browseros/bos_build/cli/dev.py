#!/usr/bin/env python3
"""
Dev CLI - Chromium patch extraction and patch-stack health

Extracts commits or files from a Chromium checkout into chromium_patches/,
and reports patch-stack health (doctor). Interactive patch application,
sync, and conflict handling live in the Rust tool (tools/bpatch, `bpatch`).
"""

import json
from pathlib import Path
from typing import Optional

import typer
from typer import Typer, Option, Argument

from ..core.context import Context
from ..lib.utils import log_info, log_error, log_success, log_warning


def create_build_context(chromium_src: Optional[Path] = None) -> Optional[Context]:
    """Create Context for dev CLI operations"""
    try:
        if not chromium_src:
            log_error("Chromium source directory not specified")
            log_info(
                "Use --chromium-src option to specify the Chromium source directory"
            )
            return None

        if not chromium_src.exists():
            log_error(f"Chromium source directory does not exist: {chromium_src}")
            return None

        return Context(
            chromium_src=chromium_src,
            architecture="",  # Not needed for patch operations
            build_type="debug",  # Not needed for patch operations
        )
    except Exception as e:
        log_error(f"Failed to create build context: {e}")
        return None


app = Typer(
    name="dev",
    help="BrowserOS dev CLI",
    no_args_is_help=True,
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


class State:
    def __init__(self):
        self.chromium_src: Optional[Path] = None
        self.verbose: bool = False
        self.quiet: bool = False


state = State()


@app.callback()
def main(
    chromium_src: Optional[Path] = Option(
        None,
        "--chromium-src",
        "-S",
        help="Path to Chromium source directory",
        exists=True,
    ),
    verbose: bool = Option(False, "--verbose", "-v", help="Enable verbose output"),
    quiet: bool = Option(False, "--quiet", "-q", help="Suppress non-essential output"),
):
    """
    Dev CLI - Chromium patch extraction and patch-stack health

    Extract patches from commits:
      browseros dev extract commit HEAD
      browseros dev extract range HEAD~5 HEAD
      browseros dev extract patch chrome/common/foo.h

    Check patch-stack health (read-only):
      browseros dev doctor
      browseros dev doctor --against ~/chromium/src --json

    Applying and syncing patches is handled by bpatch
    (packages/browseros/tools/bpatch).
    """
    state.chromium_src = chromium_src
    state.verbose = verbose
    state.quiet = quiet


extract_app = Typer(
    help="Extract patches from commits",
    no_args_is_help=True,
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)

app.add_typer(extract_app, name="extract")


def _render_doctor_report(report: dict) -> None:
    repo = report["repo"]
    scope = f" for feature '{report['feature']}'" if report["feature"] else ""
    if repo["findings"]:
        log_info(
            f"Repo checks{scope}: "
            f"{repo['errors']} error(s), {repo['warnings']} warning(s)"
        )
        for finding in repo["findings"]:
            if finding["severity"] == "error":
                log_error(f"  {finding['message']}")
            else:
                log_warning(f"  {finding['message']}")
    elif scope:
        log_success(f"Repo checks clean{scope}")
    else:
        log_success(
            f"Repo checks clean: {repo['patches']} patches "
            f"across {repo['features']} features"
        )

    apply_section = report["apply"]
    if apply_section is not None:
        if apply_section["failures"]:
            log_error(
                f"Apply check against {apply_section['against']}: "
                f"{apply_section['failed']}/{apply_section['total']} patches fail "
                f"({apply_section['features_affected']} feature(s) affected)"
            )
            by_feature: dict = {}
            for failure in apply_section["failures"]:
                by_feature.setdefault(failure["feature"], []).append(failure["patch"])
            for feature_name in sorted(by_feature):
                log_error(f"  {feature_name}:")
                for patch in by_feature[feature_name]:
                    log_error(f"    - {patch}")
        else:
            log_success(
                f"Apply check against {apply_section['against']}: "
                f"all {apply_section['total']} patches apply cleanly"
            )

    if report["healthy"]:
        log_success("Patch doctor: healthy")
    else:
        log_error("Patch doctor: unhealthy")


@app.command(name="doctor")
def doctor(
    against: Optional[Path] = Option(
        None,
        "--against",
        help="Chromium source tree to dry-run every patch against (git apply --check)",
        exists=True,
        file_okay=False,
    ),
    feature: Optional[str] = Option(
        None, "--feature", help="Restrict the report to one feature"
    ),
    json_output: bool = Option(
        False, "--json", help="Emit a machine-readable JSON report on stdout"
    ),
):
    """Read-only patch-stack health report.

    Verifies .features.yaml against the patches on disk (every entry resolves,
    every patch is claimed by a feature, claims don't overlap); with --against,
    dry-runs every patch and groups failures by feature. Read-only: nothing is
    written to the chromium tree. Exit 0 healthy / 1 findings / 2 usage or
    environment errors.
    """
    import yaml

    from ..lib.paths import get_package_root
    from ..patchkit.doctor import (
        build_report,
        check_apply,
        check_repo,
        load_features,
    )
    from ..patchkit.extract.utils import GitError, validate_git_repository

    if against and not validate_git_repository(against):
        log_error(
            f"--against {against} is not a git repository "
            "(expected a chromium src checkout)"
        )
        raise typer.Exit(2)

    root = get_package_root()
    patches_dir = root / "chromium_patches"
    try:
        features = load_features(root)
        findings = check_repo(features, patches_dir, feature)
        apply_report = (
            check_apply(features, patches_dir, against, feature) if against else None
        )
    except (ValueError, yaml.YAMLError, GitError) as e:
        log_error(str(e))
        raise typer.Exit(2)

    report = build_report(root, features, findings, apply_report, feature)
    if json_output:
        typer.echo(json.dumps(report, indent=2))
    else:
        _render_doctor_report(report)
    raise typer.Exit(0 if report["healthy"] else 1)


@extract_app.command(name="commit")
def extract_commit(
    commit: str = Argument(..., help="Git commit reference (e.g., HEAD)"),
    output: Optional[Path] = Option(None, "--output", "-o", help="Output directory"),
    interactive: bool = Option(
        True, "--interactive/--no-interactive", "-i/-n", help="Interactive mode"
    ),
    force: bool = Option(False, "--force", "-f", help="Overwrite existing patches"),
    include_binary: bool = Option(
        False, "--include-binary", help="Include binary files"
    ),
    base: Optional[str] = Option(
        None,
        "--base",
        help="Base commit to diff from for BASE_COMMIT-relative extraction (defaults to BASE_COMMIT)",
    ),
    feature: bool = Option(
        False, "--feature", help="Add extracted files to a feature in .features.yaml"
    ),
):
    """Extract patches from a single commit"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..patchkit.extract import ExtractCommitModule

    module = ExtractCommitModule()
    try:
        module.validate(ctx)
        module.execute(
            ctx,
            commit=commit,
            output=output,
            interactive=interactive,
            verbose=state.verbose,
            force=force,
            include_binary=include_binary,
            base=base,
            feature=feature,
        )
    except Exception as e:
        log_error(f"Failed to extract commit: {e}")
        raise typer.Exit(1)


@extract_app.command(name="patch")
def extract_patch_cmd(
    chromium_path: str = Argument(
        ..., help="Chromium file path (e.g., chrome/common/foo.h)"
    ),
    base: Optional[str] = Option(
        None,
        "--base",
        "-b",
        help="Base commit to diff against (defaults to BASE_COMMIT)",
    ),
    force: bool = Option(
        False, "--force", "-f", help="Overwrite existing patch without prompting"
    ),
    feature: bool = Option(
        False, "--feature", help="Add extracted file to a feature in .features.yaml"
    ),
):
    """Extract patch for a specific file"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..patchkit.extract import extract_single_file_patch

    success, error = extract_single_file_patch(ctx, chromium_path, base, force)
    if not success:
        log_error(error or "Unknown error")
        raise typer.Exit(1)
    log_success(f"Successfully extracted patch for: {chromium_path}")

    if feature:
        from ..patchkit.extract.common import resolve_base_commit
        from ..patchkit.extract.utils import GitError
        from ..patchkit.features_io import (
            add_files_to_feature,
            prompt_feature_selection,
        )

        try:
            resolved_base = resolve_base_commit(ctx, base)
        except GitError as e:
            log_error(str(e))
            raise typer.Exit(1)

        result = prompt_feature_selection(ctx, resolved_base[:12], None)
        if result is None:
            log_warning("Skipped adding file to feature")
        else:
            feature_name, description = result
            add_files_to_feature(ctx, feature_name, description, [chromium_path])


@extract_app.command(name="range")
def extract_range(
    start: str = Argument(..., help="Start commit (exclusive)"),
    end: str = Argument(..., help="End commit (inclusive)"),
    output: Optional[Path] = Option(None, "--output", "-o", help="Output directory"),
    interactive: bool = Option(
        True, "--interactive/--no-interactive", "-i/-n", help="Interactive mode"
    ),
    force: bool = Option(False, "--force", "-f", help="Overwrite existing patches"),
    include_binary: bool = Option(
        False, "--include-binary", help="Include binary files"
    ),
    squash: bool = Option(
        False, "--squash", help="Squash all commits into single patches"
    ),
    base: Optional[str] = Option(
        None,
        "--base",
        help="Base commit to diff from (defaults to BASE_COMMIT)",
    ),
    feature: bool = Option(
        False, "--feature", help="Add extracted files to a feature in .features.yaml"
    ),
):
    """Extract patches from a range of commits"""
    ctx = create_build_context(state.chromium_src)
    if not ctx:
        raise typer.Exit(1)

    from ..patchkit.extract import ExtractRangeModule

    module = ExtractRangeModule()
    try:
        module.validate(ctx)
        module.execute(
            ctx,
            start=start,
            end=end,
            output=output,
            interactive=interactive,
            verbose=state.verbose,
            force=force,
            include_binary=include_binary,
            squash=squash,
            base=base,
            feature=feature,
        )
    except Exception as e:
        log_error(f"Failed to extract range: {e}")
        raise typer.Exit(1)


if __name__ == "__main__":
    app()
