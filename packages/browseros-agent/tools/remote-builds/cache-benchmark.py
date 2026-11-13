#!/usr/bin/env python3
"""Measure dependency caching on one reserved VM with three independent source/output trees.

The job's systemd cgroup owns the foreground cache server and all compiler children.
Workspace crates bypass the cache. Relative Cargo selectors remain stable strings while
resolving beneath each distinct job/phase cwd; no compiler inputs are stripped or remapped.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time


def prove_invalidation() -> int:
    """Exercise real cache hits and stale-mtime invalidation with a tiny owned fixture.

    Three fresh outputs must print 11, 12, 23: the middle build reuses the unchanged
    dependency while recompiling workspace code, and the last changes dependency
    bytes while restoring its old mtime. No shared registry package is modified.
    """
    artifacts = Path(os.environ["REMOTE_BUILD_ARTIFACTS"])
    root = Path(os.environ["NEO_SDK_SOURCE_ROOT"]).parent / "cache-invalidation-proof"
    dependency = root / "cargo/registry/src/probe/dep"
    workspace = root / "workspace"
    dependency.mkdir(parents=True)
    workspace.mkdir()
    source = dependency / "lib.rs"
    source.write_text("pub const ANSWER: u32 = 1;\n")
    original_stat = source.stat()
    compiler = subprocess.check_output(["rustup", "which", "--toolchain", "1.98.0", "rustc"], text=True).strip()
    trace = artifacts / "invalidation-routing.txt"
    environment = {**os.environ, "CARGO_HOME": str(root / "cargo"),
                   "REMOTE_DEPENDENCY_CACHE": "1", "REMOTE_CACHE_TRACE": str(trace)}
    wrapper = "/usr/local/lib/browseros-remote-builds/rustc-cache.sh"

    def stats():
        value = subprocess.check_output(["/usr/local/bin/sccache", "--show-stats", "--stats-format", "json"],
                                        env=environment)
        return json.loads(value)["stats"]

    initial = stats()
    answers = []
    for variant in (1, 2, 3):
        output = root / ("output-" + str(variant))
        output.mkdir()
        if variant == 3:
            source.write_text("pub const ANSWER: u32 = 2;\n")
            os.utime(source, ns=(original_stat.st_atime_ns, original_stat.st_mtime_ns))
        subprocess.run([wrapper, compiler, "--crate-name", "cache_probe_dep", "--crate-type", "rlib",
                        "--edition=2021", "--emit=dep-info,link", str(source), "--out-dir", str(output)], cwd=dependency,
                       env={**environment, "CARGO_MANIFEST_DIR": str(dependency)}, check=True)
        main = workspace / "main.rs"
        main.write_text('fn main() { println!("{}", cache_probe_dep::ANSWER * 10 + ' + str(variant) + '); }\n')
        binary = output / "probe"
        subprocess.run([wrapper, compiler, "--edition=2021", str(main),
                        "--extern", "cache_probe_dep=" + str(output / "libcache_probe_dep.rlib"),
                        "-o", str(binary)], cwd=workspace,
                       env={**environment, "CARGO_MANIFEST_DIR": str(workspace)}, check=True)
        answers.append(int(subprocess.check_output([str(binary)], text=True)))
    final = stats()
    hits = final["cache_hits"]["counts"].get("Rust", 0) - initial["cache_hits"]["counts"].get("Rust", 0)
    misses = final["cache_misses"]["counts"].get("Rust", 0) - initial["cache_misses"]["counts"].get("Rust", 0)
    direct = trace.read_text().count("direct " + str(workspace)) == 3
    result = {"answers": answers, "rustHits": hits, "rustMisses": misses, "workspaceDirect": direct,
              "dependencyMtimeUnchanged": source.stat().st_mtime_ns == original_stat.st_mtime_ns}
    (artifacts / "cache-invalidation.json").write_text(json.dumps(result, indent=2))
    assert answers == [11, 12, 23] and hits >= 1 and misses >= 2 and direct, result
    print("CACHE_INVALIDATION_PASS=" + json.dumps(result), flush=True)
    return 0


def run() -> int:
    source = Path(os.environ["NEO_SDK_SOURCE_ROOT"])
    namespace = source.parent
    artifacts = Path(os.environ["REMOTE_BUILD_ARTIFACTS"])
    artifacts.mkdir(parents=True, exist_ok=True)
    root = namespace / "dependency-cache-proof"
    root.mkdir()
    toolchain = subprocess.check_output(["rustc", "+1.98.0", "-vV"])
    locks = [(str(path.relative_to(source)), path.read_bytes()) for path in sorted(source.rglob("Cargo.lock"))]
    digest = hashlib.sha256(toolchain)
    for name, content in locks:
        digest.update(name.encode() + b"\0" + content + b"\0")
    key = digest.hexdigest()
    environment = {name: value for name, value in os.environ.items() if not name.startswith("SCCACHE_")}
    environment.update(SCCACHE_CONF="/etc/browseros-remote-builds/sccache.toml",
                       SCCACHE_SERVER_UDS=str(namespace / "sccache-benchmark.sock"),
                       CARGO_TARGET_DIR="../.remote-cargo-target",
                       CARGO_BUILD_BUILD_DIR="../.remote-cargo-build",
                       CARGO_REMOTE_DEP_CACHE_KEY=key,
                       RUSTC_WRAPPER="/usr/local/lib/browseros-remote-builds/rustc-cache.sh")
    results = {"toolchain": toolchain.decode(), "lockToolchainKey": key, "phases": []}

    def stats():
        output = subprocess.check_output(["/usr/local/bin/sccache", "--show-stats", "--stats-format", "json"], env=environment)
        return json.loads(output)

    def input_digest(tree: Path):
        value = hashlib.sha256()
        for path in sorted(tree.rglob("*")):
            if path.is_file():
                value.update(str(path.relative_to(tree)).encode() + b"\0" + path.read_bytes() + b"\0")
        return value.hexdigest()

    server_environment = {**environment, "SCCACHE_START_SERVER": "1", "SCCACHE_NO_DAEMON": "1", "SCCACHE_IDLE_TIMEOUT": "0"}
    with (artifacts / "sccache-server.log").open("w") as output:
        server = subprocess.Popen(["/usr/local/bin/sccache"], env=server_environment, stdout=output, stderr=subprocess.STDOUT)
    try:
        deadline = time.monotonic() + 20
        while not Path(environment["SCCACHE_SERVER_UDS"]).exists():
            if server.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("foreground dependency cache server failed to start")
            time.sleep(.1)
        (artifacts / "stats-initial.json").write_text(json.dumps(stats(), indent=2))
        for label, enabled in (("before", False), ("prime", True), ("after", True)):
            tree = root / label / "source"
            shutil.copytree(source, tree, ignore=shutil.ignore_patterns(".remote-build-artifacts"))
            fingerprint = input_digest(tree)
            if results["phases"] and fingerprint != results["phases"][0]["inputSha256"]:
                raise RuntimeError("benchmark phases have different source inputs")
            trace = artifacts / (label + "-routing.txt")
            current = {**environment, "REMOTE_DEPENDENCY_CACHE": str(int(enabled)),
                       "REMOTE_CACHE_TRACE": str(trace), "NEO_SDK_SOURCE_ROOT": str(tree)}
            print(f"CACHE_BENCH_PHASE={label} START fresh source and private outputs", flush=True)
            started = time.monotonic()
            with (artifacts / (label + "-gate.log")).open("w") as log:
                process = subprocess.Popen(["bash", "crates/neo-sdk/check.sh"], cwd=tree / "packages/browseros-agent",
                                           env=current, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
                for line in process.stdout:
                    log.write(line)
                    print(f"[{label}] {line}", end="", flush=True)
                code = process.wait()
            seconds = time.monotonic() - started
            output = (artifacts / (label + "-gate.log")).read_text()
            phase = {"name": label, "cacheEnabled": enabled, "seconds": seconds, "exit": code,
                     "inputSha256": fingerprint, "stats": stats(),
                     "passedTestBinaries": len(re.findall(r"^test result: ok\.", output, re.M)),
                     "workspaceDirect": all("direct " + str(tree / "packages/browseros-agent/crates/neo-sdk" / package)
                                             in trace.read_text() for package in ("neo-sdk-api", "neo-service", "neo-server", "neo-playwright"))}
            results["phases"].append(phase)
            (artifacts / "cache-benchmark.json").write_text(json.dumps(results, indent=2))
            print(f"CACHE_BENCH_PHASE={label} END seconds={seconds:.3f} exit={code}", flush=True)
            if code != 0 or not phase["workspaceDirect"] or "check.sh: all checks passed" not in output:
                raise RuntimeError("benchmark gate failed or workspace routing was not direct")
        print("CACHE_BENCHMARK_COMPLETE=" + json.dumps({"before": results["phases"][0]["seconds"],
                                                      "after": results["phases"][2]["seconds"]}), flush=True)
        return 0
    finally:
        # Stopping the foreground server is bounded. The outer systemd cgroup is
        # the final fence for every compiler descendant before a slot is reusable.
        subprocess.run(["/usr/local/bin/sccache", "--stop-server"], env=environment, capture_output=True, timeout=10)
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.terminate()
            server.wait(timeout=5)


if __name__ == "__main__":
    raise SystemExit(prove_invalidation() if sys.argv[1:] == ["--probe-only"] else run())
