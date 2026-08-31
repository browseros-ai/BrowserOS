#!/usr/bin/env python3
"""GitHub module - Create GitHub releases from R2 artifacts"""

import json
import re
import subprocess
import tempfile
from pathlib import Path
from typing import Callable, Dict, List, Mapping, Optional, Tuple
from urllib.parse import quote

from ..core.context import Context
from ..core.step import Step, ValidationError
from ..lib.utils import log_info, log_error, log_success, log_warning
from ..lib.r2 import BOTO3_AVAILABLE
from .common import (
    PLATFORMS,
    PLATFORM_DISPLAY_NAMES,
    fetch_all_release_metadata,
    generate_appcast_item,
    generate_release_notes,
    get_repo_from_git,
    check_gh_cli,
    validate_release_metadata,
)


CommandRunner = Callable[..., subprocess.CompletedProcess]


def _run_gh(command: List[str], runner: CommandRunner) -> str:
    result = runner(command, capture_output=True, text=True, check=True)
    return result.stdout.strip()


def list_github_releases(
    repo: str,
    *,
    runner: CommandRunner = subprocess.run,
) -> List[Mapping[str, object]]:
    """List every GitHub release with stable REST field names normalized."""
    pages = json.loads(
        _run_gh(
            [
                "gh",
                "api",
                "--paginate",
                "--slurp",
                f"repos/{repo}/releases?per_page=100",
            ],
            runner,
        )
        or "[]"
    )
    if not isinstance(pages, list) or not all(isinstance(page, list) for page in pages):
        raise RuntimeError("GitHub release response must contain arrays of pages")
    releases: List[Mapping[str, object]] = []
    for page in pages:
        for release in page:
            if not isinstance(release, dict):
                raise RuntimeError("GitHub release response contains an invalid entry")
            releases.append(
                {
                    "tagName": release.get("tag_name"),
                    "isDraft": release.get("draft"),
                    "targetCommitish": release.get("target_commitish"),
                }
            )
    return releases


def list_pull_requests(
    repo: str,
    *,
    state: str = "open",
    head: str = "",
    runner: CommandRunner = subprocess.run,
) -> List[Mapping[str, object]]:
    """List every matching pull request with reconciliation fields.

    Suite PRs are a durable version-allocation ledger, including closed records.
    GraphQL cursor pagination avoids silently releasing old allocations after an
    arbitrary fixed number of newer repository PRs.
    """
    try:
        owner, name = repo.split("/", 1)
        states = {
            "open": "OPEN",
            "closed": "CLOSED",
            "merged": "MERGED",
            "all": "OPEN, CLOSED, MERGED",
        }[state]
    except (KeyError, ValueError) as exc:
        raise ValueError(
            f"Unsupported pull request query: repo={repo}, state={state}"
        ) from exc
    query = f"""
query($owner: String!, $name: String!, $endCursor: String) {{
  repository(owner: $owner, name: $name) {{
    pullRequests(
      first: 100
      after: $endCursor
      states: [{states}]
      orderBy: {{field: CREATED_AT, direction: DESC}}
    ) {{
      nodes {{
        number url state isDraft headRefName headRefOid baseRefName body
        mergedAt mergeCommit {{ oid }} mergeable isCrossRepository
        headRepository {{ nameWithOwner }}
      }}
      pageInfo {{ hasNextPage endCursor }}
    }}
  }}
}}
""".strip()
    pages = json.loads(
        _run_gh(
            [
                "gh",
                "api",
                "graphql",
                "--paginate",
                "--slurp",
                "-F",
                f"owner={owner}",
                "-F",
                f"name={name}",
                "-f",
                f"query={query}",
            ],
            runner,
        )
        or "[]"
    )
    if not isinstance(pages, list):
        raise RuntimeError("GitHub pull request response must contain pages")
    records: List[Mapping[str, object]] = []
    for page in pages:
        if not isinstance(page, dict):
            raise RuntimeError("GitHub pull request page must be an object")
        data = page.get("data")
        repository = data.get("repository") if isinstance(data, dict) else None
        connection = (
            repository.get("pullRequests") if isinstance(repository, dict) else None
        )
        nodes = connection.get("nodes") if isinstance(connection, dict) else None
        if not isinstance(nodes, list) or not all(
            isinstance(item, dict) for item in nodes
        ):
            raise RuntimeError("GitHub pull request page contains invalid nodes")
        records.extend(
            item for item in nodes if not head or item.get("headRefName") == head
        )
    return records


def create_pull_request(
    *,
    repo: str,
    head: str,
    base: str,
    title: str,
    body: str,
    draft: bool = False,
    runner: CommandRunner = subprocess.run,
) -> str:
    """Create a pull request and return its URL."""
    command = [
        "gh",
        "pr",
        "create",
        "--repo",
        repo,
        "--head",
        head,
        "--base",
        base,
        "--title",
        title,
        "--body",
        body,
    ]
    if draft:
        command.append("--draft")
    return _run_gh(command, runner)


def edit_pull_request_body(
    *,
    repo: str,
    number: int,
    body: str,
    runner: CommandRunner = subprocess.run,
) -> None:
    """Replace pull request metadata without changing its branch."""
    _run_gh(
        [
            "gh",
            "pr",
            "edit",
            str(number),
            "--repo",
            repo,
            "--body",
            body,
        ],
        runner,
    )


def mark_pull_request_ready(
    repo: str,
    number: int,
    *,
    runner: CommandRunner = subprocess.run,
) -> None:
    """Move a draft pull request to ready without changing its branch."""
    _run_gh(
        ["gh", "pr", "ready", str(number), "--repo", repo],
        runner,
    )


def merge_pull_request(
    repo: str,
    number: int,
    *,
    expected_head_sha: str,
    runner: CommandRunner = subprocess.run,
) -> str:
    """Squash-merge a pull request and return the merge commit."""
    _run_gh(
        [
            "gh",
            "pr",
            "merge",
            str(number),
            "--repo",
            repo,
            "--squash",
            "--match-head-commit",
            expected_head_sha,
        ],
        runner,
    )
    document = json.loads(
        _run_gh(
            [
                "gh",
                "pr",
                "view",
                str(number),
                "--repo",
                repo,
                "--json",
                "mergeCommit",
            ],
            runner,
        )
        or "{}"
    )
    merge_commit = document.get("mergeCommit") if isinstance(document, dict) else None
    if isinstance(merge_commit, dict):
        sha = merge_commit.get("oid") or merge_commit.get("sha")
    else:
        sha = document.get("sha") if isinstance(document, dict) else None
    if not isinstance(sha, str) or not sha:
        raise RuntimeError(f"Pull request #{number} merged without a merge commit")
    return sha


def create_github_release(
    tag: str,
    repo: str,
    title: str,
    notes: str,
    draft: bool = True,
    target: str = "",
) -> Tuple[bool, str]:
    """Create GitHub release via gh CLI"""
    cmd = [
        "gh",
        "release",
        "create",
        tag,
        "--repo",
        repo,
        "--title",
        title,
        "--notes",
        notes,
    ]
    if draft:
        cmd.append("--draft")
    if target:
        cmd.extend(["--target", target])

    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return True, result.stdout.strip()
    except subprocess.CalledProcessError as e:
        stderr = e.stderr or str(e)
        normalized_error = stderr.lower().replace("_", " ")
        if "already exist" in normalized_error:
            return False, f"Release {tag} already exists"
        return False, stderr


def download_file(url: str, dest: Path) -> bool:
    """Download file from URL using curl"""
    try:
        subprocess.run(
            ["curl", "--fail", "--show-error", "-L", "-o", str(dest), url],
            check=True,
            capture_output=True,
        )
        return True
    except Exception:
        return False


def upload_to_github_release(tag: str, repo: str, file_path: Path) -> bool:
    """Upload file to existing GitHub release"""
    try:
        subprocess.run(
            ["gh", "release", "upload", tag, str(file_path), "--repo", repo],
            check=True,
            capture_output=True,
        )
        return True
    except Exception:
        return False


def normalize_version(version: str) -> str:
    """Normalize version to MAJOR.MINOR.BUILD (strip patch if present)"""
    parts = version.split(".")
    if len(parts) >= 3:
        return ".".join(parts[:3])
    return version


def github_release_tag(version: str, product_id: str) -> str:
    """Return the product-owned Git tag for a browser release."""
    normalized = normalize_version(version)
    if product_id == "browseros":
        return f"v{normalized}"
    return f"{product_id}/v{normalized}"


def inspect_github_release(tag: str, repo: str) -> Dict:
    """Read a release or raise; callers must never treat API failure as absence."""
    document = json.loads(
        _run_gh(
            [
                "gh",
                "release",
                "view",
                tag,
                "--repo",
                repo,
                "--json",
                "isDraft,targetCommitish,assets",
            ],
            subprocess.run,
        )
    )
    if not isinstance(document, dict):
        raise RuntimeError("GitHub release response must be an object")
    assets = {}
    for asset in document.get("assets", []):
        if not isinstance(asset, dict) or not asset.get("name"):
            continue
        digest = asset.get("digest")
        sha256 = ""
        if isinstance(digest, str) and digest.startswith("sha256:"):
            sha256 = digest.removeprefix("sha256:").lower()
        assets[str(asset["name"])] = {
            "sha256": sha256,
            "size": asset.get("size"),
        }
    return {
        "isDraft": document.get("isDraft"),
        "targetCommitish": document.get("targetCommitish"),
        "assets": list(assets),
        "asset_metadata": assets,
    }


def resolve_github_tag_target(tag: str, repo: str) -> Optional[str]:
    """Resolve a release tag to its commit, distinguishing 404 from API errors."""
    tag_uri = quote(tag, safe="")
    result = subprocess.run(
        [
            "gh",
            "api",
            "--include",
            f"repos/{repo}/commits/{tag_uri}",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    output = (result.stdout or "").replace("\r\n", "\n")
    match = re.match(r"HTTP/\S+\s+(\d{3})", output)
    status = match.group(1) if match else ""
    _, separator, body = output.partition("\n\n")
    document = None
    if separator:
        try:
            document = json.loads(body)
        except json.JSONDecodeError:
            document = None

    if status == "404":
        return None
    if (
        status == "422"
        and isinstance(document, dict)
        and document.get("message") == f"No commit found for SHA: {tag}"
    ):
        return None
    if status != "200" or result.returncode != 0:
        detail = (result.stderr or output or "no response").strip()
        raise RuntimeError(
            f"Could not resolve Git tag {tag} "
            f"(HTTP {status or 'unknown'}, gh rc={result.returncode}): {detail}"
        )

    if not separator:
        raise RuntimeError(f"GitHub returned no JSON body while resolving tag {tag}")
    if document is None:
        raise RuntimeError(f"GitHub returned invalid JSON while resolving tag {tag}")
    target = document.get("sha") if isinstance(document, dict) else None
    if not isinstance(target, str) or not target:
        raise RuntimeError(f"GitHub returned no commit SHA while resolving tag {tag}")
    return target


def verify_github_release_target(
    tag: str,
    repo: str,
    expected_target: str,
    release: Optional[Dict] = None,
) -> Dict:
    """Verify a draft points at the immutable workflow commit before mutation."""
    if not expected_target:
        return release or {}

    resolved_target = resolve_github_tag_target(tag, repo)
    current_release = release
    if resolved_target is None:
        current_release = current_release or inspect_github_release(tag, repo)
        resolved_target = current_release.get("targetCommitish")

    if resolved_target != expected_target:
        raise RuntimeError(
            f"Release {tag} resolves to {resolved_target or 'no target'}, "
            f"not requested target {expected_target}"
        )
    return current_release or {}


def edit_github_release(
    tag: str,
    repo: str,
    title: str,
    notes: str,
) -> None:
    """Refresh release presentation after validating draft ownership."""
    subprocess.run(
        [
            "gh",
            "release",
            "edit",
            tag,
            "--repo",
            repo,
            "--title",
            title,
            "--notes",
            notes,
        ],
        capture_output=True,
        text=True,
        check=True,
    )


def delete_github_release_asset(tag: str, repo: str, asset: str) -> None:
    """Delete one stale asset from a validated draft release."""
    subprocess.run(
        [
            "gh",
            "release",
            "delete-asset",
            tag,
            asset,
            "--repo",
            repo,
            "--yes",
        ],
        capture_output=True,
        text=True,
        check=True,
    )


def download_and_upload_artifacts(
    tag: str,
    repo: str,
    metadata: Dict[str, Dict],
    platforms: Optional[List[str]] = None,
) -> List[Tuple[str, bool]]:
    """Download artifacts from R2 and upload to GitHub release"""
    if platforms is None:
        platforms = PLATFORMS

    results = []

    with tempfile.TemporaryDirectory() as tmpdir:
        tmppath = Path(tmpdir)

        for platform in platforms:
            if platform not in metadata:
                continue

            for key, artifact in metadata[platform].get("artifacts", {}).items():
                url = artifact["url"]
                filename = artifact["filename"]
                local_path = tmppath / filename

                log_info(f"  Downloading {filename}...")
                if not download_file(url, local_path):
                    log_error(f"  Failed to download {filename}")
                    results.append((filename, False))
                    continue

                log_info(f"  Uploading {filename}...")
                if upload_to_github_release(tag, repo, local_path):
                    log_success(f"  Uploaded {filename}")
                    results.append((filename, True))
                else:
                    log_error(f"  Failed to upload {filename}")
                    results.append((filename, False))

    return results


class GithubModule(Step):
    """Create GitHub release from R2 artifacts"""

    produces = []
    requires = []
    description = "Create GitHub release from R2 artifacts"

    def __init__(
        self,
        draft: bool = True,
        skip_upload: bool = False,
        title: Optional[str] = None,
        platforms: Optional[str] = None,
        macos_arch: str = "universal",
        source_sha: str = "",
        workflow_run_id: str = "",
        workflow_run_attempt: str = "",
        target: str = "",
    ):
        self.draft = draft
        self.skip_upload = skip_upload
        self.title = title
        self.release_platforms = platforms
        self.macos_arch = macos_arch
        self.source_sha = source_sha
        self.workflow_run_id = workflow_run_id
        self.workflow_run_attempt = workflow_run_attempt
        self.target = target

    def validate(self, ctx: Context) -> None:
        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )

        if not ctx.env.has_r2_config():
            raise ValidationError("R2 configuration not set")

        if not ctx.release_version:
            raise ValidationError("--version is required")

        if not check_gh_cli():
            raise ValidationError(
                "gh CLI not found. Install from: https://cli.github.com"
            )

        if not ctx.github_repo:
            repo = get_repo_from_git()
            if not repo:
                raise ValidationError(
                    "Could not detect repo from git remote. Use --repo flag."
                )
            ctx.github_repo = repo

    def execute(self, ctx: Context) -> None:
        version = ctx.release_version
        tag_version = normalize_version(version)
        tag = github_release_tag(tag_version, ctx.product.id)
        repo = ctx.github_repo

        metadata = fetch_all_release_metadata(version, ctx.env, ctx.product.id)
        if not metadata:
            raise RuntimeError(f"No release metadata found for version {version}")
        expected_assets: Optional[set[str]] = None
        if self.release_platforms is not None:
            metadata = validate_release_metadata(
                metadata,
                version=version,
                product_id=ctx.product.id,
                platforms=self.release_platforms,
                macos_arch=self.macos_arch,
                source_sha=self.source_sha,
                workflow_run_id=self.workflow_run_id,
                workflow_run_attempt=self.workflow_run_attempt,
            )
            expected_assets = {
                artifact["filename"]
                for release in metadata.values()
                for artifact in release["artifacts"].values()
            }

        log_info(f"\n{'=' * 60}")
        log_info(f"Creating GitHub Release: {tag}")
        log_info(f"{'=' * 60}")

        for platform, release in metadata.items():
            artifacts = release.get("artifacts", {})
            log_info(
                f"  {PLATFORM_DISPLAY_NAMES[platform]}: {len(artifacts)} artifact(s)"
            )

        log_info(f"  Repo: {repo}")
        log_info(f"  Draft: {self.draft}")

        release_title = self.title or f"{ctx.product.display_name} v{tag_version}"
        notes = generate_release_notes(tag_version, metadata, ctx.product)

        log_info("\nCreating GitHub release...")
        success, result = create_github_release(
            tag,
            repo,
            release_title,
            notes,
            self.draft,
            self.target,
        )

        if success:
            log_success(f"Release created: {result}")
            try:
                verify_github_release_target(tag, repo, self.target)
            except Exception as exc:
                raise RuntimeError(
                    f"Could not verify release target for {tag}: {exc}"
                ) from exc
        else:
            if "already exists" in result:
                log_warning(result)
                try:
                    existing = inspect_github_release(tag, repo)
                except Exception as exc:
                    raise RuntimeError(
                        f"Could not inspect existing release {tag}: {exc}"
                    ) from exc
                if existing.get("isDraft") is not True:
                    raise RuntimeError(
                        f"Release {tag} already exists and is not a draft; "
                        "refusing to modify live release assets"
                    )
                try:
                    verify_github_release_target(
                        tag,
                        repo,
                        self.target,
                        release=existing,
                    )
                except Exception as exc:
                    raise RuntimeError(
                        f"Could not verify release target for {tag}: {exc}"
                    ) from exc
                if expected_assets is not None and not self.skip_upload:
                    existing_assets = set(existing.get("assets", []))
                    unexpected_assets = existing_assets - expected_assets
                    if self.release_platforms != "all" and unexpected_assets:
                        raise RuntimeError(
                            f"Draft release {tag} contains assets outside the "
                            f"selected {self.release_platforms} contract: "
                            f"{sorted(unexpected_assets)}. Refusing a partial "
                            "refresh that would delete other platforms; rerun "
                            "with --platforms all or use a new version."
                        )
                    for asset in sorted(existing_assets):
                        try:
                            delete_github_release_asset(tag, repo, asset)
                        except Exception as exc:
                            raise RuntimeError(
                                f"Failed to delete stale draft asset {asset}: {exc}"
                            ) from exc
                try:
                    edit_github_release(tag, repo, release_title, notes)
                except Exception as exc:
                    raise RuntimeError(
                        f"Failed to refresh draft release {tag}: {exc}"
                    ) from exc
            else:
                raise RuntimeError(f"Failed to create release: {result}")

        if not self.skip_upload:
            log_info("\nUploading artifacts to GitHub release...")
            results = download_and_upload_artifacts(
                tag,
                repo,
                metadata,
                platforms=list(metadata),
            )

            if not results:
                raise RuntimeError("No release artifacts found to upload")
            failed = [f for f, ok in results if not ok]
            if failed:
                raise RuntimeError(f"Failed to upload: {', '.join(failed)}")
            if expected_assets is not None:
                try:
                    final_release = inspect_github_release(tag, repo)
                except Exception as exc:
                    raise RuntimeError(
                        f"Could not verify release assets for {tag}: {exc}"
                    ) from exc
                if final_release.get("isDraft") is not True:
                    raise RuntimeError(
                        f"Release {tag} is no longer a draft; refusing to "
                        "accept the asset upload"
                    )
                try:
                    verify_github_release_target(
                        tag,
                        repo,
                        self.target,
                        release=final_release,
                    )
                except Exception as exc:
                    raise RuntimeError(
                        f"Could not verify final release target for {tag}: {exc}"
                    ) from exc
                actual_assets = set(final_release.get("assets", []))
                if actual_assets != expected_assets:
                    raise RuntimeError(
                        f"Release {tag} asset set mismatch: expected "
                        f"{sorted(expected_assets)}, got {sorted(actual_assets)}"
                    )

        if "macos" in metadata:
            log_info("\n" + "=" * 60)
            log_info("APPCAST SNIPPET")
            log_info("=" * 60)

            release = metadata["macos"]
            sparkle_version = release.get("sparkle_version", "")
            build_date = release.get("build_date", "")

            arch_to_file = {
                "arm64": "appcast.xml",
                "x64": "appcast-x86_64.xml",
                "universal": "appcast.xml",
            }

            for arch in ["arm64", "x64", "universal"]:
                if arch in release.get("artifacts", {}):
                    artifact = release["artifacts"][arch]
                    log_info(f"\n{arch_to_file[arch]} ({arch}):")
                    print(
                        generate_appcast_item(
                            artifact, tag_version, sparkle_version, build_date
                        )
                    )

        log_info(f"\n{'=' * 60}")
        log_success(f"Release {tag} complete!")
