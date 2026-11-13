"""Own one disk-only dependency compiler cache inside a job's systemd cgroup.

Only immutable cache entries survive the job. Cargo output stays private, and the
selective rustc wrapper never caches workspace/path crates or rewrites compiler inputs.
"""
from __future__ import annotations

import contextlib
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

SCCACHE = "/usr/local/bin/sccache"
CONFIG = Path("/etc/browseros-remote-builds/sccache.toml")
ENABLED = Path("/etc/browseros-remote-builds/dependency-cache.enabled")
WRAPPER = Path(__file__).with_name("rustc-cache.sh")


def namespace_key(source: Path) -> str:
    """Lockfiles and the pinned compiler partition public dependency cache entries."""
    digest = hashlib.sha256(subprocess.check_output(["rustc", "+1.98.0", "-vV"]))
    for path in sorted(source.rglob("Cargo.lock")):
        digest.update(str(path.relative_to(source)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


@contextlib.contextmanager
def dependency_cache(source: Path, job_root: Path):
    """Supply stable selectors resolving within this UUID, then stop our server.

    Cargo hashes its target/build environment values for sccache. Relative values
    stay identical across jobs while resolving from each job's own invocation cwd.
    Keeping the foreground server here also keeps compiler children inside the
    payload's systemd cgroup, so lease expiry stops them before the slot is freed.
    """
    environment = dict(os.environ)
    if not ENABLED.exists() or environment.get("REMOTE_DEPENDENCY_CACHE") == "0":
        yield environment
        return
    cwd = Path.cwd().resolve()
    for name in (".remote-cargo-target", ".remote-cargo-build"):
        if not (cwd.parent / name).resolve().is_relative_to(job_root):
            raise ValueError("dependency-cache output would escape the job directory")
    artifacts = Path(environment["REMOTE_BUILD_ARTIFACTS"])
    artifacts.mkdir(parents=True, exist_ok=True)
    # Guest admission supplies a minimal environment. Clear any backend settings
    # nevertheless: this cache must never inherit a developer's cloud credentials.
    environment = {key: value for key, value in environment.items() if not key.startswith("SCCACHE_")}
    environment.update(SCCACHE_CONF=str(CONFIG), SCCACHE_SERVER_UDS=str(job_root / "sccache.sock"),
                       CARGO_TARGET_DIR="../.remote-cargo-target",
                       CARGO_BUILD_BUILD_DIR="../.remote-cargo-build",
                       CARGO_REMOTE_DEP_CACHE_KEY=namespace_key(source),
                       REMOTE_DEPENDENCY_CACHE="1", RUSTC_WRAPPER=str(WRAPPER))
    server = None
    metrics = {"enabled": True, "lockToolchainKey": environment["CARGO_REMOTE_DEP_CACHE_KEY"]}
    try:
        with (artifacts / "dependency-cache-server.log").open("w") as log:
            server = subprocess.Popen([SCCACHE], stdout=log, stderr=subprocess.STDOUT,
                                      env={**environment, "SCCACHE_START_SERVER": "1",
                                           "SCCACHE_NO_DAEMON": "1", "SCCACHE_IDLE_TIMEOUT": "0"})
        deadline = time.monotonic() + 20
        while not Path(environment["SCCACHE_SERVER_UDS"]).exists():
            if server.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("dependency compiler cache did not start; rerun with caching disabled")
            time.sleep(.1)
        if server.poll() is not None:
            raise RuntimeError("dependency cache server escaped foreground supervision")
        metrics["serverCgroup"] = Path(f"/proc/{server.pid}/cgroup").read_text()
        if metrics["serverCgroup"] != Path("/proc/self/cgroup").read_text():
            raise RuntimeError("dependency cache server escaped this job's systemd cgroup")
        print("Dependency cache: public packages only; workspace output is private to this job.", flush=True)
        yield environment
    finally:
        # Metrics/stop failures must not replace the workload's real exit status.
        # Systemd is the final cancellation fence even if the cache cannot respond.
        if server is not None:
            try:
                stats = subprocess.check_output([SCCACHE, "--show-stats", "--stats-format", "json"],
                                                env=environment, timeout=10, stderr=subprocess.DEVNULL)
                metrics["stats"] = json.loads(stats)
                subprocess.run([SCCACHE, "--stop-server"], env=environment,
                               capture_output=True, timeout=10)
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                metrics["diagnostic"] = str(error)
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait(timeout=5)
        (artifacts / "dependency-cache.json").write_text(json.dumps(metrics, indent=2))
