#!/usr/bin/env python3
"""Storage CLI - Push third-party resources to R2 for build:server ingestion."""

import hashlib
import json
import os
import tarfile
import tempfile
import zipfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import requests
import typer

from .support import EnvConfig
from .support import log_error, log_info, log_success, log_warning
from .support import (
    BOTO3_AVAILABLE,
    get_r2_client,
    upload_file_to_r2,
)

LIMA_RELEASE_BASE = "https://github.com/lima-vm/lima/releases/download"
LIMA_R2_PREFIX = "artifacts/vendor/third_party/lima"
LIMA_MANIFEST_KEY = f"{LIMA_R2_PREFIX}/manifest.json"
HTTP_TIMEOUT_S = 60
BUN_RELEASE_BASE = "https://github.com/oven-sh/bun/releases/download"
BUN_R2_PREFIX = "artifacts/vendor/third_party/bun"
BUN_MANIFEST_KEY = f"{BUN_R2_PREFIX}/manifest.json"
BUN_HTTP_TIMEOUT_S = 120
CODEX_RELEASE_BASE = "https://github.com/openai/codex/releases/download"
CODEX_DEFAULT_TAG = "rust-v0.136.0"
CODEX_R2_PREFIX = "artifacts/vendor/third_party/codex"
CODEX_MANIFEST_KEY = f"{CODEX_R2_PREFIX}/manifest.json"
CODEX_HTTP_TIMEOUT_S = 120
CLAUDE_CODE_RELEASE_BASE = "https://downloads.claude.ai/claude-code-releases"
CLAUDE_CODE_DEFAULT_VERSION = "2.1.159"
CLAUDE_CODE_R2_PREFIX = "artifacts/vendor/third_party/claude-code"
CLAUDE_CODE_MANIFEST_KEY = f"{CLAUDE_CODE_R2_PREFIX}/manifest.json"
CLAUDE_CODE_HTTP_TIMEOUT_S = 300


@dataclass(frozen=True)
class LimaArch:
    """Arch-pair: the suffix Lima uses upstream and the suffix we use in R2."""

    internal: str  # "arm64" | "x64" — how our R2 keys name it
    upstream: str  # "Darwin-arm64" | "Darwin-x86_64" — Lima's tarball suffix
    linux_guest_arch: str  # "aarch64" | "x86_64" — Lima guest agent arch


LIMA_ARCHES: Tuple[LimaArch, ...] = (
    LimaArch(internal="arm64", upstream="Darwin-arm64", linux_guest_arch="aarch64"),
    LimaArch(internal="x64", upstream="Darwin-x86_64", linux_guest_arch="x86_64"),
)


@dataclass(frozen=True)
class BunTarget:
    """Mapping from a Bun release asset to the R2 object staged into bundles."""

    internal: str  # "darwin-arm64" | "linux-x64" — how manifests identify it
    upstream: str  # "darwin-aarch64" | "linux-x64-baseline" — Bun's zip suffix
    r2_name: str  # object basename under BUN_R2_PREFIX
    binary_name: str = "bun"  # Windows zips contain bun.exe


BUN_TARGETS: Tuple[BunTarget, ...] = (
    BunTarget(
        internal="darwin-arm64",
        upstream="darwin-aarch64",
        r2_name="bun-darwin-arm64",
    ),
    BunTarget(
        internal="darwin-x64",
        upstream="darwin-x64",
        r2_name="bun-darwin-x64",
    ),
    BunTarget(
        internal="linux-arm64",
        upstream="linux-aarch64",
        r2_name="bun-linux-arm64",
    ),
    BunTarget(
        internal="linux-x64",
        upstream="linux-x64-baseline",
        r2_name="bun-linux-x64-baseline",
    ),
    BunTarget(
        internal="windows-x64",
        upstream="windows-x64-baseline",
        r2_name="bun-windows-x64-baseline.exe",
        binary_name="bun.exe",
    ),
)


@dataclass(frozen=True)
class CodexPlatform:
    """BrowserOS target plus the upstream Codex package coordinates."""

    target: str
    upstream: str


CODEX_PLATFORMS: Tuple[CodexPlatform, ...] = (
    CodexPlatform(
        target="darwin-arm64",
        upstream="aarch64-apple-darwin",
    ),
    CodexPlatform(
        target="darwin-x64",
        upstream="x86_64-apple-darwin",
    ),
    CodexPlatform(
        target="linux-arm64",
        upstream="aarch64-unknown-linux-musl",
    ),
    CodexPlatform(
        target="linux-x64",
        upstream="x86_64-unknown-linux-musl",
    ),
    CodexPlatform(
        target="windows-x64",
        upstream="x86_64-pc-windows-msvc",
    ),
)


@dataclass(frozen=True)
class ClaudeCodePlatform:
    """BrowserOS target plus the upstream Claude Code platform name."""

    target: str
    upstream: str


CLAUDE_CODE_PLATFORMS: Tuple[ClaudeCodePlatform, ...] = (
    ClaudeCodePlatform(target="darwin-arm64", upstream="darwin-arm64"),
    ClaudeCodePlatform(target="darwin-x64", upstream="darwin-x64"),
    ClaudeCodePlatform(target="linux-arm64", upstream="linux-arm64"),
    ClaudeCodePlatform(target="linux-x64", upstream="linux-x64"),
    ClaudeCodePlatform(target="windows-x64", upstream="win32-x64"),
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
        object_shas: Dict[str, Dict[str, str]] = {}
        tarball_shas: Dict[str, str] = {}

        try:
            for arch in LIMA_ARCHES:
                tarball_sha, arch_object_shas, _ = _process_arch(
                    tag,
                    arch,
                    tmp_dir,
                    checksums,
                    client,
                    env,
                    dry_run,
                    uploaded_keys,
                )
                tarball_shas[arch.internal] = tarball_sha
                object_shas[arch.internal] = arch_object_shas

            manifest = _build_manifest(tag, tarball_shas, object_shas)
            _upload_manifest(client, env, manifest, tmp_dir, dry_run)
        # Any failure mid-loop (download, sha verify, extract, upload) must
        # roll back prior arch uploads so R2 never holds a mixed-version pair.
        except Exception as exc:
            if not dry_run and uploaded_keys:
                log_warning(
                    f"Upload failed — rolling back {len(uploaded_keys)} object(s)"
                )
                _rollback(client, env.r2_bucket, uploaded_keys)
            log_error(f"Lima upload aborted: {exc}")
            raise typer.Exit(1)

    log_success(f"Lima {tag} uploaded for {[a.internal for a in LIMA_ARCHES]}")


@app.command("bun")
def upload_bun(
    version: str = typer.Option(
        ...,
        "--version",
        "-v",
        help="Bun release tag, e.g. bun-v1.2.15",
    ),
    dry_run: bool = typer.Option(
        False,
        "--dry-run",
        help="Download + verify only; skip R2 uploads.",
    ),
) -> None:
    """Download Bun from an upstream GitHub release and push target binaries to R2."""
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

    tag = _normalize_bun_version_tag(version)
    client = None if dry_run else get_r2_client(env)
    if not dry_run and client is None:
        log_error("Failed to create R2 client")
        raise typer.Exit(1)

    with tempfile.TemporaryDirectory(prefix="bun-upload-") as tmp:
        tmp_dir = Path(tmp)
        checksums = _fetch_bun_checksums(tag, tmp_dir)
        uploaded_keys: List[str] = []
        object_shas: Dict[str, str] = {}
        zip_shas: Dict[str, str] = {}

        try:
            for target in BUN_TARGETS:
                zip_sha, binary_sha, _ = _process_bun_target(
                    tag,
                    target,
                    tmp_dir,
                    checksums,
                    client,
                    env,
                    dry_run,
                    uploaded_keys,
                )
                zip_shas[target.internal] = zip_sha
                object_shas[target.internal] = binary_sha

            manifest = _build_bun_manifest(tag, zip_shas, object_shas)
            _upload_bun_manifest(client, env, manifest, tmp_dir, dry_run)
        except Exception as exc:
            if not dry_run and uploaded_keys:
                log_warning(
                    f"Upload failed — rolling back {len(uploaded_keys)} object(s)"
                )
                _rollback(client, env.r2_bucket, uploaded_keys)
            log_error(f"Bun upload aborted: {exc}")
            raise typer.Exit(1)

    log_success(f"Bun {tag} uploaded for {[t.internal for t in BUN_TARGETS]}")


@app.command("codex")
def upload_codex(
    version: str = typer.Option(
        CODEX_DEFAULT_TAG,
        "--version",
        "-v",
        help="Codex release tag, e.g. rust-v0.136.0",
    ),
    dry_run: bool = typer.Option(
        False,
        "--dry-run",
        help="Download + verify only; skip R2 uploads.",
    ),
) -> None:
    """Download Codex release packages and push normalized native binaries to R2."""
    tag = _normalize_codex_release_tag(version)
    env, client = _prepare_upload_client(dry_run)

    with tempfile.TemporaryDirectory(prefix="codex-upload-") as tmp:
        tmp_dir = Path(tmp)
        checksums = _fetch_codex_package_checksums(tag, tmp_dir)
        uploaded_keys: List[str] = []
        package_shas: Dict[str, str] = {}
        object_shas: Dict[str, str] = {}

        try:
            for platform in CODEX_PLATFORMS:
                package_sha, binary_sha, _ = _process_codex_platform(
                    tag,
                    platform,
                    tmp_dir,
                    checksums,
                    client,
                    env,
                    dry_run,
                    uploaded_keys,
                )
                package_shas[platform.target] = package_sha
                object_shas[platform.target] = binary_sha

            manifest = _build_codex_manifest(tag, package_shas, object_shas)
            _upload_codex_manifest(client, env, manifest, tmp_dir, dry_run)
        except Exception as exc:
            if not dry_run and uploaded_keys:
                log_warning(
                    f"Upload failed — rolling back {len(uploaded_keys)} object(s)"
                )
                _rollback(client, env.r2_bucket, uploaded_keys)
            log_error(f"Codex upload aborted: {exc}")
            raise typer.Exit(1)

    log_success(f"Codex {tag} uploaded for {[p.target for p in CODEX_PLATFORMS]}")


@app.command("claude-code")
def upload_claude_code(
    version: str = typer.Option(
        CLAUDE_CODE_DEFAULT_VERSION,
        "--version",
        "-v",
        help="Claude Code release version, e.g. 2.1.159",
    ),
    dry_run: bool = typer.Option(
        False,
        "--dry-run",
        help="Download + verify only; skip R2 uploads.",
    ),
) -> None:
    """Download Claude Code release binaries and push normalized objects to R2."""
    version = version.strip()
    env, client = _prepare_upload_client(dry_run)

    with tempfile.TemporaryDirectory(prefix="claude-code-upload-") as tmp:
        tmp_dir = Path(tmp)
        manifest = _fetch_claude_code_manifest(version, tmp_dir)
        uploaded_keys: List[str] = []
        object_shas: Dict[str, str] = {}
        platform_info: Dict[str, Dict[str, str]] = {}

        try:
            for platform in CLAUDE_CODE_PLATFORMS:
                binary_sha, _ = _process_claude_code_platform(
                    version,
                    platform,
                    manifest,
                    tmp_dir,
                    client,
                    env,
                    dry_run,
                    uploaded_keys,
                )
                object_shas[platform.target] = binary_sha
                platform_info[platform.target] = _claude_code_manifest_info(
                    manifest, platform
                )

            upload_manifest = _build_claude_code_manifest(
                version, object_shas, platform_info
            )
            _upload_claude_code_manifest(client, env, upload_manifest, tmp_dir, dry_run)
        except Exception as exc:
            if not dry_run and uploaded_keys:
                log_warning(
                    f"Upload failed — rolling back {len(uploaded_keys)} object(s)"
                )
                _rollback(client, env.r2_bucket, uploaded_keys)
            log_error(f"Claude Code upload aborted: {exc}")
            raise typer.Exit(1)

    log_success(
        f"Claude Code {version} uploaded for {[p.target for p in CLAUDE_CODE_PLATFORMS]}"
    )


def _normalize_version_tag(version: str) -> str:
    return version if version.startswith("v") else f"v{version}"


def _normalize_bun_version_tag(version: str) -> str:
    if version.startswith("bun-v"):
        return version
    if version.startswith("bun-"):
        return f"bun-v{version[len('bun-') :]}"
    return f"bun-{_normalize_version_tag(version)}"


def _normalize_codex_release_tag(version: str) -> str:
    if version.startswith("rust-v"):
        return version
    if version.startswith("rust-"):
        suffix = version[len("rust-") :]
        return f"rust-{_normalize_version_tag(suffix)}"
    return f"rust-{_normalize_version_tag(version)}"


def _codex_cli_version(tag: str) -> str:
    return tag[len("rust-v") :] if tag.startswith("rust-v") else tag


def _prepare_upload_client(dry_run: bool) -> Tuple[EnvConfig, Any]:
    """Create the R2 client for real uploads while keeping dry-runs local."""
    env = EnvConfig()
    if dry_run:
        return env, None
    if not BOTO3_AVAILABLE:
        log_error("boto3 not installed — run: pip install boto3")
        raise typer.Exit(1)
    if not env.has_r2_config():
        log_error(
            "R2 configuration missing. Required: "
            "R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY"
        )
        raise typer.Exit(1)
    client = get_r2_client(env)
    if client is None:
        log_error("Failed to create R2 client")
        raise typer.Exit(1)
    return env, client


def _fetch_checksums(tag: str, tmp_dir: Path) -> Dict[str, str]:
    url = f"{LIMA_RELEASE_BASE}/{tag}/SHA256SUMS"
    dest = tmp_dir / "SHA256SUMS"
    log_info(f"Fetching {url}")
    _download(url, dest)
    return _parse_checksums(dest.read_text(encoding="utf-8"))


def _fetch_bun_checksums(tag: str, tmp_dir: Path) -> Dict[str, str]:
    url = f"{BUN_RELEASE_BASE}/{tag}/SHASUMS256.txt"
    dest = tmp_dir / "SHASUMS256.txt"
    log_info(f"Fetching {url}")
    _download(url, dest)
    return _parse_checksums(dest.read_text(encoding="utf-8"))


def _fetch_codex_package_checksums(tag: str, tmp_dir: Path) -> Dict[str, str]:
    url = f"{CODEX_RELEASE_BASE}/{tag}/codex-package_SHA256SUMS"
    dest = tmp_dir / "codex-package_SHA256SUMS"
    log_info(f"Fetching {url}")
    _download(url, dest)
    return _parse_checksums(dest.read_text(encoding="utf-8"))


def _fetch_claude_code_manifest(version: str, tmp_dir: Path) -> Dict[str, Any]:
    url = f"{CLAUDE_CODE_RELEASE_BASE}/{version}/manifest.json"
    dest = tmp_dir / "claude-code-manifest.json"
    log_info(f"Fetching {url}")
    _download(url, dest, timeout=CLAUDE_CODE_HTTP_TIMEOUT_S)
    manifest = json.loads(dest.read_text(encoding="utf-8"))
    if not isinstance(manifest, dict):
        raise RuntimeError("Claude Code manifest must be a JSON object")
    return manifest


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
    uploaded_keys: Optional[List[str]] = None,
) -> Tuple[str, Dict[str, str], List[str]]:
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

    guest_agent_name = f"lima-guestagent.Linux-{arch.linux_guest_arch}.gz"
    runtime_files = [
        (
            "limactl",
            "bin/limactl",
            tmp_dir / f"limactl-darwin-{arch.internal}",
            f"{LIMA_R2_PREFIX}/limactl-darwin-{arch.internal}",
        ),
        (
            "guest_agent",
            f"share/lima/{guest_agent_name}",
            tmp_dir / guest_agent_name,
            f"{LIMA_R2_PREFIX}/{guest_agent_name}",
        ),
    ]

    object_shas: Dict[str, str] = {}
    r2_keys: List[str] = []
    for name, logical_path, local_path, r2_key in runtime_files:
        _extract_lima_file(tarball_path, logical_path, local_path)
        object_shas[name] = _sha256_file(local_path)
        r2_keys.append(r2_key)

    for _, _, local_path, r2_key in runtime_files:
        if dry_run:
            log_info(f"[dry-run] skipped upload of {r2_key}")
            continue
        if not upload_file_to_r2(client, local_path, r2_key, env.r2_bucket):
            raise RuntimeError(f"Failed to upload {r2_key}")
        if uploaded_keys is not None:
            uploaded_keys.append(r2_key)

    return actual_sha, object_shas, r2_keys


def _process_bun_target(
    tag: str,
    target: BunTarget,
    tmp_dir: Path,
    checksums: Dict[str, str],
    client: Any,
    env: EnvConfig,
    dry_run: bool,
    uploaded_keys: Optional[List[str]] = None,
) -> Tuple[str, str, str]:
    zip_name = f"bun-{target.upstream}.zip"
    expected_sha = checksums.get(zip_name)
    if not expected_sha:
        raise RuntimeError(
            f"{zip_name} missing from SHASUMS256.txt (is the version tag correct?)"
        )

    zip_path = tmp_dir / zip_name
    url = f"{BUN_RELEASE_BASE}/{tag}/{zip_name}"
    log_info(f"Downloading {url}")
    _download(url, zip_path, timeout=BUN_HTTP_TIMEOUT_S)

    actual_sha = _sha256_file(zip_path)
    if actual_sha != expected_sha:
        raise RuntimeError(
            f"sha256 mismatch for {zip_name}: expected {expected_sha}, got {actual_sha}"
        )

    local_path = tmp_dir / target.r2_name
    r2_key = f"{BUN_R2_PREFIX}/{target.r2_name}"
    _extract_bun_file(zip_path, local_path, target.binary_name)
    binary_sha = _sha256_file(local_path)

    if dry_run:
        log_info(f"[dry-run] skipped upload of {r2_key}")
    elif not upload_file_to_r2(client, local_path, r2_key, env.r2_bucket):
        raise RuntimeError(f"Failed to upload {r2_key}")
    elif uploaded_keys is not None:
        uploaded_keys.append(r2_key)

    return actual_sha, binary_sha, r2_key


def _process_codex_platform(
    tag: str,
    platform: CodexPlatform,
    tmp_dir: Path,
    checksums: Dict[str, str],
    client: Any,
    env: EnvConfig,
    dry_run: bool,
    uploaded_keys: Optional[List[str]] = None,
) -> Tuple[str, str, str]:
    package_name = f"codex-package-{platform.upstream}.tar.gz"
    expected_sha = checksums.get(package_name)
    if not expected_sha:
        raise RuntimeError(
            f"{package_name} missing from codex-package_SHA256SUMS "
            "(is the version tag correct?)"
        )

    package_path = tmp_dir / package_name
    url = f"{CODEX_RELEASE_BASE}/{tag}/{package_name}"
    log_info(f"Downloading {url}")
    _download(url, package_path, timeout=CODEX_HTTP_TIMEOUT_S)

    actual_sha = _sha256_file(package_path)
    if actual_sha != expected_sha:
        raise RuntimeError(
            f"sha256 mismatch for {package_name}: expected {expected_sha}, got {actual_sha}"
        )

    local_path = tmp_dir / _platform_binary_object_name("codex", platform.target)
    r2_key = (
        f"{CODEX_R2_PREFIX}/{_platform_binary_object_name('codex', platform.target)}"
    )
    _extract_codex_file(package_path, local_path)
    binary_sha = _sha256_file(local_path)

    if dry_run:
        log_info(f"[dry-run] skipped upload of {r2_key}")
    elif not upload_file_to_r2(client, local_path, r2_key, env.r2_bucket):
        raise RuntimeError(f"Failed to upload {r2_key}")
    elif uploaded_keys is not None:
        uploaded_keys.append(r2_key)

    return actual_sha, binary_sha, r2_key


def _process_claude_code_platform(
    version: str,
    platform: ClaudeCodePlatform,
    manifest: Dict[str, Any],
    tmp_dir: Path,
    client: Any,
    env: EnvConfig,
    dry_run: bool,
    uploaded_keys: Optional[List[str]] = None,
) -> Tuple[str, str]:
    entry = _claude_code_manifest_entry(manifest, platform)
    binary_name = entry["binary"]
    expected_sha = entry["checksum"]
    expected_size = entry["size"]

    local_path = tmp_dir / f"{platform.upstream}-{binary_name}"
    url = f"{CLAUDE_CODE_RELEASE_BASE}/{version}/{platform.upstream}/{binary_name}"
    log_info(f"Downloading {url}")
    _download(url, local_path, timeout=CLAUDE_CODE_HTTP_TIMEOUT_S)

    actual_size = local_path.stat().st_size
    if actual_size != expected_size:
        raise RuntimeError(
            f"size mismatch for Claude Code {platform.upstream}: "
            f"expected {expected_size}, got {actual_size}"
        )

    actual_sha = _sha256_file(local_path)
    if actual_sha != expected_sha:
        raise RuntimeError(
            f"sha256 mismatch for Claude Code {platform.upstream}: "
            f"expected {expected_sha}, got {actual_sha}"
        )

    if not binary_name.endswith(".exe"):
        local_path.chmod(0o755)

    r2_key = f"{CLAUDE_CODE_R2_PREFIX}/{_platform_binary_object_name('claude', platform.target)}"
    if dry_run:
        log_info(f"[dry-run] skipped upload of {r2_key}")
    elif not upload_file_to_r2(client, local_path, r2_key, env.r2_bucket):
        raise RuntimeError(f"Failed to upload {r2_key}")
    elif uploaded_keys is not None:
        uploaded_keys.append(r2_key)

    return actual_sha, r2_key


def _extract_lima_file(tarball_path: Path, logical_path: str, dest: Path) -> None:
    with tarfile.open(tarball_path, "r:gz") as tar:
        for member in tar.getmembers():
            if not member.isfile():
                continue
            if _logical_lima_path(member.name) != logical_path:
                continue
            extracted = tar.extractfile(member)
            if extracted is None:
                raise RuntimeError(f"{member.name} is not a regular file")
            dest.parent.mkdir(parents=True, exist_ok=True)
            with extracted as src, open(dest, "wb") as out:
                while chunk := src.read(1024 * 1024):
                    out.write(chunk)
            dest.chmod(member.mode & 0o777)
            return
    raise RuntimeError(f"{logical_path} not found in Lima tarball")


def _extract_bun_file(zip_path: Path, dest: Path, binary_name: str = "bun") -> None:
    with zipfile.ZipFile(zip_path) as archive:
        for member in archive.infolist():
            if member.is_dir():
                continue
            if _logical_bun_path(member.filename) != binary_name:
                continue
            dest.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(member) as src, open(dest, "wb") as out:
                while chunk := src.read(1024 * 1024):
                    out.write(chunk)
            if not binary_name.endswith(".exe"):
                dest.chmod(0o755)
            return
    raise RuntimeError(f"{binary_name} not found in Bun zip")


def _extract_codex_file(package_path: Path, dest: Path) -> None:
    """Extract the entrypoint declared by Codex's package metadata."""
    with tarfile.open(package_path, "r:gz") as tar:
        members = tar.getmembers()
        entrypoint = _read_codex_package_entrypoint(tar, members)
        for member in members:
            if not member.isfile():
                continue
            if _logical_archive_path(member.name) != entrypoint:
                continue
            extracted = tar.extractfile(member)
            if extracted is None:
                raise RuntimeError(f"{member.name} is not a regular file")
            dest.parent.mkdir(parents=True, exist_ok=True)
            with extracted as src, open(dest, "wb") as out:
                while chunk := src.read(1024 * 1024):
                    out.write(chunk)
            if not entrypoint.endswith(".exe"):
                dest.chmod(0o755)
            return
    raise RuntimeError(f"Codex entrypoint {entrypoint} not found in package")


def _logical_lima_path(member_name: str) -> str:
    parts = Path(member_name.lstrip("./")).parts
    if len(parts) > 1 and parts[0].startswith("lima-"):
        parts = parts[1:]
    return "/".join(parts)


def _logical_bun_path(member_name: str) -> str:
    parts = Path(member_name.lstrip("./")).parts
    if len(parts) > 1 and parts[0].startswith("bun-"):
        parts = parts[1:]
    return "/".join(parts)


def _logical_archive_path(member_name: str) -> str:
    return "/".join(Path(member_name.lstrip("./")).parts)


def _read_codex_package_entrypoint(
    tar: tarfile.TarFile,
    members: List[tarfile.TarInfo],
) -> str:
    for member in members:
        if not member.isfile():
            continue
        if _logical_archive_path(member.name) != "codex-package.json":
            continue
        extracted = tar.extractfile(member)
        if extracted is None:
            raise RuntimeError("codex-package.json is not a regular file")
        with extracted as src:
            metadata = json.loads(src.read().decode("utf-8"))
        entrypoint = metadata.get("entrypoint") if isinstance(metadata, dict) else None
        if not isinstance(entrypoint, str) or not entrypoint:
            raise RuntimeError("Codex package metadata is missing entrypoint")
        return entrypoint
    raise RuntimeError("codex-package.json not found in Codex package")


def _platform_binary_object_name(binary_stem: str, target: str) -> str:
    suffix = ".exe" if target.startswith("windows-") else ""
    return f"{binary_stem}-{target}{suffix}"


def _claude_code_manifest_entry(
    manifest: Dict[str, Any],
    platform: ClaudeCodePlatform,
) -> Dict[str, Any]:
    platforms = manifest.get("platforms")
    if not isinstance(platforms, dict):
        raise RuntimeError("Claude Code manifest is missing platforms")
    entry = platforms.get(platform.upstream)
    if not isinstance(entry, dict):
        raise RuntimeError(f"{platform.upstream} missing from Claude Code manifest")
    binary = entry.get("binary")
    checksum = entry.get("checksum")
    size = entry.get("size")
    if not isinstance(binary, str) or not binary:
        raise RuntimeError(
            f"Claude Code manifest has invalid binary for {platform.upstream}"
        )
    if not _is_sha256(checksum):
        raise RuntimeError(
            f"Claude Code manifest has invalid checksum for {platform.upstream}"
        )
    if not isinstance(size, int) or size < 0:
        raise RuntimeError(
            f"Claude Code manifest has invalid size for {platform.upstream}"
        )
    return {"binary": binary, "checksum": checksum, "size": size}


def _claude_code_manifest_info(
    manifest: Dict[str, Any],
    platform: ClaudeCodePlatform,
) -> Dict[str, str]:
    entry = _claude_code_manifest_entry(manifest, platform)
    return {"platform": platform.upstream, "binary": entry["binary"]}


def _is_sha256(value: Any) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(c in "0123456789abcdef" for c in value.lower())
    )


def _build_manifest(
    tag: str,
    tarball_shas: Dict[str, str],
    object_shas: Dict[str, Dict[str, str]],
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


def _build_bun_manifest(
    tag: str,
    zip_shas: Dict[str, str],
    object_shas: Dict[str, str],
) -> Dict[str, Any]:
    return {
        "bun_version": tag,
        "zip_shas_upstream": zip_shas,
        "r2_object_shas": object_shas,
        "uploaded_at": datetime.now(timezone.utc).isoformat(),
        "uploaded_by": os.environ.get("GITHUB_ACTOR") or "local",
    }


def _build_codex_manifest(
    tag: str,
    package_shas: Dict[str, str],
    object_shas: Dict[str, str],
) -> Dict[str, Any]:
    return {
        "codex_release_tag": tag,
        "codex_cli_version": _codex_cli_version(tag),
        "package_shas_upstream": package_shas,
        "r2_object_shas": object_shas,
        "uploaded_at": datetime.now(timezone.utc).isoformat(),
        "uploaded_by": os.environ.get("GITHUB_ACTOR") or "local",
    }


def _build_claude_code_manifest(
    version: str,
    binary_shas: Dict[str, str],
    platform_info: Dict[str, Dict[str, str]],
) -> Dict[str, Any]:
    return {
        "claude_code_version": version,
        "binary_shas_upstream": dict(binary_shas),
        "r2_object_shas": dict(binary_shas),
        "platforms": {target: dict(info) for target, info in platform_info.items()},
        "uploaded_at": datetime.now(timezone.utc).isoformat(),
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
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    if dry_run:
        log_info(f"[dry-run] manifest would be: {manifest}")
        return
    if not upload_file_to_r2(client, manifest_path, LIMA_MANIFEST_KEY, env.r2_bucket):
        raise RuntimeError(f"Failed to upload {LIMA_MANIFEST_KEY}")


def _upload_bun_manifest(
    client: Any,
    env: EnvConfig,
    manifest: Dict[str, Any],
    tmp_dir: Path,
    dry_run: bool,
) -> None:
    manifest_path = tmp_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    if dry_run:
        log_info(f"[dry-run] manifest would be: {manifest}")
        return
    if not upload_file_to_r2(client, manifest_path, BUN_MANIFEST_KEY, env.r2_bucket):
        raise RuntimeError(f"Failed to upload {BUN_MANIFEST_KEY}")


def _upload_codex_manifest(
    client: Any,
    env: EnvConfig,
    manifest: Dict[str, Any],
    tmp_dir: Path,
    dry_run: bool,
) -> None:
    manifest_path = tmp_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    if dry_run:
        log_info(f"[dry-run] manifest would be: {manifest}")
        return
    if not upload_file_to_r2(client, manifest_path, CODEX_MANIFEST_KEY, env.r2_bucket):
        raise RuntimeError(f"Failed to upload {CODEX_MANIFEST_KEY}")


def _upload_claude_code_manifest(
    client: Any,
    env: EnvConfig,
    manifest: Dict[str, Any],
    tmp_dir: Path,
    dry_run: bool,
) -> None:
    manifest_path = tmp_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    if dry_run:
        log_info(f"[dry-run] manifest would be: {manifest}")
        return
    if not upload_file_to_r2(
        client, manifest_path, CLAUDE_CODE_MANIFEST_KEY, env.r2_bucket
    ):
        raise RuntimeError(f"Failed to upload {CLAUDE_CODE_MANIFEST_KEY}")


def _rollback(client: Any, bucket: str, keys: List[str]) -> None:
    for key in keys:
        try:
            client.delete_object(Bucket=bucket, Key=key)
            log_info(f"Rolled back {key}")
        except Exception as exc:
            log_warning(f"Rollback failed for {key}: {exc}")


def _download(url: str, dest: Path, *, timeout: Optional[int] = None) -> None:
    response = requests.get(url, stream=True, timeout=timeout or HTTP_TIMEOUT_S)
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
