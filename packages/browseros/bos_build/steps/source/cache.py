#!/usr/bin/env python3
"""Chromium checkout cache in Cloudflare R2 for runners without WarpCache.

Absorbs scripts/ci/r2_cache.py. WarpBuild's cache action does not
support Windows runners, and GitHub's actions/cache caps at 10GB/repo —
useless for a ~60GB checkout. R2 has zero egress fees and the repo
already ships R2 credentials for release uploads, so Windows caches the
post-sync tree as a zstd tarball under ci-cache/chromium/.

Cache misses (and missing credentials) return cache-hit=false so a
nightly build degrades to a cold checkout instead of failing.
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from ...lib.utils import log_info

OBJECT_PREFIX = "ci-cache/chromium/"


def _log(msg: str) -> None:
    log_info(f"[source.cache] {msg}")


def write_github_output(name: str, value: str) -> None:
    output_path = os.environ.get("GITHUB_OUTPUT")
    if output_path:
        with open(output_path, "a") as f:
            f.write(f"{name}={value}\n")
    _log(f"output: {name}={value}")


def find_tool(name: str) -> str:
    # System32 tar.exe is bsdtar, which mishandles >260-char paths in the
    # chromium tree; prefer Git's bundled GNU tar on Windows.
    if name == "tar" and sys.platform == "win32":
        for candidate in (
            r"C:\Program Files\Git\usr\bin\tar.exe",
            r"C:\Program Files (x86)\Git\usr\bin\tar.exe",
        ):
            if Path(candidate).exists():
                return candidate
    path = shutil.which(name)
    if not path:
        raise SystemExit(f"[source.cache] required tool not found on PATH: {name}")
    return path


def _get_r2_client():
    account_id = os.environ.get("R2_ACCOUNT_ID")
    access_key = os.environ.get("R2_ACCESS_KEY_ID")
    secret_key = os.environ.get("R2_SECRET_ACCESS_KEY")
    if not (account_id and access_key and secret_key):
        return None

    import boto3
    from boto3.s3.transfer import TransferConfig

    client = boto3.client(
        "s3",
        endpoint_url=f"https://{account_id}.r2.cloudflarestorage.com",
        aws_access_key_id=access_key,
        aws_secret_access_key=secret_key,
        region_name="auto",
    )
    transfer_config = TransferConfig(
        multipart_threshold=64 * 1024 * 1024,
        multipart_chunksize=128 * 1024 * 1024,
        max_concurrency=16,
    )
    return client, transfer_config


def _object_exists(client, bucket: str, key: str) -> bool:
    try:
        client.head_object(Bucket=bucket, Key=key)
        return True
    except Exception:
        return False


def _format_pipeline_command(cmd, cwd: Path | None) -> str:
    command = " ".join(cmd)
    if cwd is None:
        return command
    return f"{command}  (cwd={cwd})"


def _run_pipeline(
    producer,
    consumer,
    *,
    producer_cwd: Path | None = None,
    consumer_cwd: Path | None = None,
) -> None:
    _log(
        "$ "
        f"{_format_pipeline_command(producer, producer_cwd)} | "
        f"{_format_pipeline_command(consumer, consumer_cwd)}"
    )
    p1 = subprocess.Popen(producer, stdout=subprocess.PIPE, cwd=producer_cwd)
    assert p1.stdout is not None  # guaranteed by stdout=PIPE
    p2 = subprocess.Popen(consumer, stdin=p1.stdout, cwd=consumer_cwd)
    p1.stdout.close()
    rc2 = p2.wait()
    rc1 = p1.wait()
    if rc1 != 0 or rc2 != 0:
        raise SystemExit(
            f"[source.cache] pipeline failed (producer={rc1}, consumer={rc2})"
        )


def _run_command(
    cmd,
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
) -> int:
    _log(f"$ {_format_pipeline_command(cmd, cwd)}")
    return subprocess.run(cmd, cwd=cwd, env=env, check=False).returncode


def _unlink_if_exists(path: Path) -> None:
    try:
        path.unlink()
    except FileNotFoundError:
        pass


def _restore_windows_tarball(tarball: Path, root: Path) -> None:
    tar_file = tarball.with_suffix("")
    try:
        rc = _run_command(
            [find_tool("zstd"), "-d", "-f", "-o", str(tar_file), str(tarball)]
        )
        if rc != 0:
            raise SystemExit(f"[source.cache] zstd decompression failed (rc={rc})")

        # Drop the compressed archive as soon as the rewindable tar exists.
        _unlink_if_exists(tarball)

        # --force-local: MSYS tar reads the colon in C:\... as a remote host
        # ("Cannot connect to C: resolve failed") without it.
        tar_cmd = [find_tool("tar"), "--force-local", "-xf", str(tar_file)]
        tar_env = {**os.environ, "MSYS": "winsymlinks:nativestrict"}
        _log("extract pass 1/2")
        first_rc = _run_command(tar_cmd, cwd=root, env=tar_env)
        if first_rc == 0:
            return

        _log("extract pass 1/2 failed; retrying full archive")
        _log("extract pass 2/2")
        second_rc = _run_command(tar_cmd, cwd=root, env=tar_env)
        if second_rc == 0:
            return

        raise SystemExit(
            "[source.cache] tar extraction failed after retry "
            f"(pass1={first_rc}, pass2={second_rc})"
        )
    finally:
        _unlink_if_exists(tar_file)


def restore(key: str, root: Path) -> bool:
    """Restore the cached checkout; returns cache-hit."""
    r2 = _get_r2_client()
    if r2 is None:
        _log("R2 credentials not set; skipping cache restore")
        write_github_output("cache-hit", "false")
        return False

    client, transfer_config = r2
    bucket = os.environ.get("R2_BUCKET", "browseros")
    object_key = f"{OBJECT_PREFIX}{key}.tar.zst"

    if not _object_exists(client, bucket, object_key):
        _log(f"Cache miss: s3://{bucket}/{object_key}")
        write_github_output("cache-hit", "false")
        return False

    root = root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    tarball = Path(tempfile.gettempdir()) / "chromium-cache.tar.zst"

    _log(f"Downloading s3://{bucket}/{object_key} -> {tarball}")
    client.download_file(bucket, object_key, str(tarball), Config=transfer_config)
    size_gb = tarball.stat().st_size / 1024**3
    _log(f"Downloaded {size_gb:.1f} GiB; extracting to {root}")

    if sys.platform == "win32":
        _restore_windows_tarball(tarball, root)
    else:
        _run_pipeline(
            [find_tool("zstd"), "-d", "-c", str(tarball)],
            [find_tool("tar"), "-xf", "-"],
            consumer_cwd=root,
        )
        tarball.unlink()
    write_github_output("cache-hit", "true")
    return True


def save(key: str, root: Path) -> None:
    """Save the checkout as a cache object (no overwrite)."""
    r2 = _get_r2_client()
    if r2 is None:
        _log("R2 credentials not set; skipping cache save")
        return

    client, transfer_config = r2
    bucket = os.environ.get("R2_BUCKET", "browseros")
    object_key = f"{OBJECT_PREFIX}{key}.tar.zst"

    if _object_exists(client, bucket, object_key):
        _log(f"Cache already exists, not overwriting: s3://{bucket}/{object_key}")
        return

    root = root.resolve()
    tarball = root.parent / "chromium-cache.tar.zst"

    _log(f"Archiving {root} -> {tarball}")
    _run_pipeline(
        [
            find_tool("tar"),
            "-cf",
            "-",
            "--exclude=./src/out",
            ".",
        ],
        [find_tool("zstd"), "-T0", "-3", "-f", "-o", str(tarball)],
        producer_cwd=root,
    )
    size_gb = tarball.stat().st_size / 1024**3
    _log(f"Uploading {size_gb:.1f} GiB -> s3://{bucket}/{object_key}")
    client.upload_file(str(tarball), bucket, object_key, Config=transfer_config)
    tarball.unlink()
    _log("Cache saved")
