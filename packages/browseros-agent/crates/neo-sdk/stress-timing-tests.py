#!/usr/bin/env python3
"""Repeat exact Rust tests beside one CPU burner per logical core.

Run this inside a remote-builds job. Each test outcome and owned PID is recorded
in the output directory. A watchdog bounds deadlocks; only a successful Rust
assertion run counts as a pass.
"""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def interrupted(signum, _frame):
    """Unwind through cleanup when the caller cancels the run."""
    raise SystemExit(128 + signum)


def stop(process):
    if process.poll() is None:
        process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--test", nargs=2, action="append", required=True, metavar=("BINARY", "NAME")
    )
    parser.add_argument("--repeat", type=int, default=50)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not os.environ.get("REMOTE_BUILD_JOB"):
        parser.error("CPU stress runs must run inside a remote-builds job")
    if args.repeat < 1:
        parser.error("--repeat must be positive")
    args.output.mkdir(parents=True, exist_ok=False)
    signal.signal(signal.SIGTERM, interrupted)
    started = time.monotonic()
    burners = []
    results = []
    active = None
    try:
        for _ in range(os.cpu_count() or 1):
            burners.append(
                subprocess.Popen(
                    ["yes"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
                )
            )
            (args.output / "burner-pids.json").write_text(
                json.dumps([process.pid for process in burners])
            )
        print(f"Started {len(burners)} CPU burners; recorded every PID.", flush=True)
        for iteration in range(1, args.repeat + 1):
            for binary, name in args.test:
                remaining = 1170 - (time.monotonic() - started)
                if remaining <= 0:
                    raise TimeoutError("stress run reached its twenty-minute limit")
                before = time.monotonic()
                log = args.output / f"{iteration:02}-{name}.log"
                with log.open("w") as output:
                    active = subprocess.Popen(
                        [binary, name, "--exact", "--nocapture"],
                        stdout=output,
                        stderr=subprocess.STDOUT,
                    )
                    (args.output / "active-test-pid").write_text(str(active.pid))
                    try:
                        code = active.wait(timeout=min(45, remaining))
                    except subprocess.TimeoutExpired:
                        stop(active)
                        code = 124
                active = None
                passed = code == 0 and "1 passed; 0 failed;" in log.read_text()
                result = {
                    "iteration": iteration,
                    "test": name,
                    "exit": code,
                    "passed": passed,
                    "seconds": time.monotonic() - before,
                }
                results.append(result)
                with (args.output / "results.jsonl").open("a") as stream:
                    stream.write(json.dumps(result) + "\n")
            if iteration % 5 == 0:
                failures = sum(not row["passed"] for row in results)
                print(f"{iteration}/{args.repeat} rounds, {failures} failures", flush=True)
    finally:
        if active is not None:
            stop(active)
        # Keep ownership in the Popen objects. Never discover targets by name or port.
        for burner in burners:
            if burner.poll() is None:
                burner.terminate()
        for burner in burners:
            stop(burner)
        summary = {
            name: {
                "passed": sum(row["passed"] for row in results if row["test"] == name),
                "failed": sum(not row["passed"] for row in results if row["test"] == name),
            }
            for _, name in args.test
        }
        (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        (args.output / "burners-stopped.json").write_text(
            json.dumps({str(process.pid): process.returncode for process in burners})
        )
        print(json.dumps(summary), flush=True)
    return int(any(not row["passed"] for row in results))


if __name__ == "__main__":
    raise SystemExit(main())
