#!/usr/bin/env python3
"""Storage CLI - Push third-party resources to R2 for build:server ingestion."""

import hashlib
import json
import os
import tarfile
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import requests
import typer

from ..common.env import EnvConfig
from ..common.utils import log_error, log_info, log_success, log_warning
from ..modules.storage.r2 import (
    BOTO3_AVAILABLE,
    get_r2_client,
    upload_file_to_r2,
)

LIMA_RELEASE_BASE = "https://github.com/lima-vm/lima/releases/download"
LIMA_R2_PREFIX = "artifacts/vendor/third_party/lima"
LIMA_MANIFEST_KEY = f"{LIMA_R2_PREFIX}/manifest.json"
LIMA_HTTP_TIMEOUT_S = 60


@dataclass(frozen=True)
class LimaArch:
    """Arch-pair: the suffix Lima uses upstream and the suffix we use in R2."""

    internal: str  # "arm64" | "x64" — how our R2 keys name it
    upstream: str  # "Darwin-arm64" | "Darwin-x86_64" — Lima's tarball suffix


LIMA_ARCHES: Tuple[LimaArch, ...] = (
    LimaArch(internal="arm64", upstream="Darwin-arm64"),
    LimaArch(internal="x64", upstream="Darwin-x86_64"),
)


app = typer.Typer(
    help="Upload third-party resources to Cloudflare R2",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)


@app.command("lima")
def upload_lima(
    version: str = typer.Option(
        ...,
        "--version",
        "-v",
        help="Lima release tag, e.g. v1.2.3",
    ),
    dry_run: bool = typer.Option(
        False,
        "--dry-run",
        help="Download + verify only; skip R2 uploads.",
    ),
) -> None:
    """Download limactl from a Lima GitHub release and push to R2."""
    if not BOTO3_AVAILABLE:
        log_error("boto3 not installed — run: pip install boto3")
        raise typer.Exit(1)

    env = EnvConfig()
    if not env.has_r2_config():
        log_error(
            "R2 configuration missing. Required: "
            "R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY"
        )
        raise typer.Exit(1)

    tag = _normalize_version_tag(version)
    client = None if dry_run else get_r2_client(env)
    if not dry_run and client is None:
        log_error("Failed to create R2 client")
        raise typer.Exit(1)

    with tempfile.TemporaryDirectory(prefix="lima-upload-") as tmp:
        tmp_dir = Path(tmp)
        checksums = _fetch_checksums(tag, tmp_dir)
        uploaded_keys: List[str] = []
        object_shas: Dict[str, str] = {}
        tarball_shas: Dict[str, str] = {}

        try:
            for arch in LIMA_ARCHES:
                tarball_sha, object_sha, r2_key = _process_arch(
                    tag, arch, tmp_dir, checksums, client, env, dry_run
                )
                tarball_shas[arch.internal] = tarball_sha
                object_shas[arch.internal] = object_sha
                if not dry_run:
                    uploaded_keys.append(r2_key)

            manifest = _build_manifest(tag, tarball_shas, object_shas)
            _upload_manifest(client, env, manifest, tmp_dir, dry_run)
        # Any failure mid-loop (download, sha verify, extract, upload) must
        # roll back prior arch uploads so R2 never holds a mixed-version pair.
        except Exception as exc:
            if not dry_run and uploaded_keys:
                log_warning(f"Upload failed — rolling back {len(uploaded_keys)} object(s)")
                _rollback(client, env.r2_bucket, uploaded_keys)
            log_error(f"Lima upload aborted: {exc}")
            raise typer.Exit(1)

    log_success(f"Lima {tag} uploaded for {[a.internal for a in LIMA_ARCHES]}")


def _normalize_version_tag(version: str) -> str:
    return version if version.startswith("v") else f"v{version}"


def _fetch_checksums(tag: str, tmp_dir: Path) -> Dict[str, str]:
    url = f"{LIMA_RELEASE_BASE}/{tag}/SHA256SUMS"
    dest = tmp_dir / "SHA256SUMS"
    log_info(f"Fetching {url}")
    _download(url, dest)
    return _parse_checksums(dest.read_text(encoding="utf-8"))


def _parse_checksums(contents: str) -> Dict[str, str]:
    """Parse lines like '<sha256>  lima-1.2.3-Darwin-arm64.tar.gz'."""
    entries: Dict[str, str] = {}
    for raw_line in contents.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        parts = line.split(None, 1)
        if len(parts) != 2:
            raise RuntimeError(f"Malformed SHA256SUMS line: {raw_line!r}")
        sha, name = parts[0].lower(), parts[1].lstrip("*").strip()
        if len(sha) != 64 or not all(c in "0123456789abcdef" for c in sha):
            raise RuntimeError(f"Invalid sha256 in SHA256SUMS: {raw_line!r}")
        entries[name] = sha
    return entries


def _process_arch(
    tag: str,
    arch: LimaArch,
    tmp_dir: Path,
    checksums: Dict[str, str],
    client: Any,
    env: EnvConfig,
    dry_run: bool,
) -> Tuple[str, str, str]:
    version_num = tag.lstrip("v")
    tarball_name = f"lima-{version_num}-{arch.upstream}.tar.gz"
    expected_sha = checksums.get(tarball_name)
    if not expected_sha:
        raise RuntimeError(
            f"{tarball_name} missing from SHA256SUMS (is the version tag correct?)"
        )

    tarball_path = tmp_dir / tarball_name
    url = f"{LIMA_RELEASE_BASE}/{tag}/{tarball_name}"
    log_info(f"Downloading {url}")
    _download(url, tarball_path)

    actual_sha = _sha256_file(tarball_path)
    if actual_sha != expected_sha:
        raise RuntimeError(
            f"sha256 mismatch for {tarball_name}: "
            f"expected {expected_sha}, got {actual_sha}"
        )

    limactl_path = tmp_dir / f"limactl-darwin-{arch.internal}"
    _extract_limactl(tarball_path, limactl_path)
    object_sha = _sha256_file(limactl_path)

    r2_key = f"{LIMA_R2_PREFIX}/limactl-darwin-{arch.internal}"
    if dry_run:
        log_info(f"[dry-run] skipped upload of {r2_key}")
    else:
        if not upload_file_to_r2(client, limactl_path, r2_key, env.r2_bucket):
            raise RuntimeError(f"Failed to upload {r2_key}")

    return actual_sha, object_sha, r2_key


def _extract_limactl(tarball_path: Path, dest: Path) -> None:
    """Extract the single `bin/limactl` entry to dest."""
    with tarfile.open(tarball_path, "r:gz") as tar:
        member = _find_limactl_member(tar)
        extracted = tar.extractfile(member)
        if extracted is None:
            raise RuntimeError(f"{member.name} is not a regular file")
        with extracted as src, open(dest, "wb") as out:
            while chunk := src.read(1024 * 1024):
                out.write(chunk)
    dest.chmod(0o755)


def _find_limactl_member(tar: tarfile.TarFile) -> tarfile.TarInfo:
    for member in tar.getmembers():
        if not member.isfile():
            continue
        parts = Path(member.name).parts
        if len(parts) >= 2 and parts[-2:] == ("bin", "limactl"):
            return member
    raise RuntimeError("bin/limactl not found in Lima tarball")


def _build_manifest(
    tag: str,
    tarball_shas: Dict[str, str],
    object_shas: Dict[str, str],
) -> Dict[str, Any]:
    return {
        "lima_version": tag,
        "tarball_shas_upstream": tarball_shas,
        "r2_object_shas": object_shas,
        "uploaded_at": datetime.now(timezone.utc).isoformat(),
        # Prefer CI context so we don't leak an individual's OS login when
        # running locally. manifest.json is surfaced via the public CDN.
        "uploaded_by": os.environ.get("GITHUB_ACTOR") or "local",
    }


def _upload_manifest(
    client: Any,
    env: EnvConfig,
    manifest: Dict[str, Any],
    tmp_dir: Path,
    dry_run: bool,
) -> None:
    manifest_path = tmp_dir / "manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    if dry_run:
        log_info(f"[dry-run] manifest would be: {manifest}")
        return
    if not upload_file_to_r2(client, manifest_path, LIMA_MANIFEST_KEY, env.r2_bucket):
        raise RuntimeError(f"Failed to upload {LIMA_MANIFEST_KEY}")


def _rollback(client: Any, bucket: str, keys: List[str]) -> None:
    for key in keys:
        try:
            client.delete_object(Bucket=bucket, Key=key)
            log_info(f"Rolled back {key}")
        except Exception as exc:
            log_warning(f"Rollback failed for {key}: {exc}")


def _download(url: str, dest: Path, *, timeout: Optional[int] = None) -> None:
    response = requests.get(url, stream=True, timeout=timeout or LIMA_HTTP_TIMEOUT_S)
    response.raise_for_status()
    dest.parent.mkdir(parents=True, exist_ok=True)
    with open(dest, "wb") as out:
        for chunk in response.iter_content(chunk_size=1024 * 1024):
            if chunk:
                out.write(chunk)


def _sha256_file(path: Path) -> str:
    sha = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            sha.update(chunk)
    return sha.hexdigest()
