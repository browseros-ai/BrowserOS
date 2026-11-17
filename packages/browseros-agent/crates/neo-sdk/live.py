#!/usr/bin/env python3
"""Compile the live SDK tests once, then give each suite a private browser.

Cases within a suite run serially because independent CDP clients attach to the
same browser tabs: one client's teardown can clear another's emulation. Cases
that deliberately create multiple clients retain that coverage inside the case.
"""

import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time


AGENT = Path(__file__).resolve().parents[2]
SDK = Path(__file__).resolve().parent
SUMMARY = re.compile(
    r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; "
    r"(\d+) measured; (\d+) filtered out; finished in ([\d.]+)s"
)


def stop(child):
    """Let the launcher drain its private browser before escalating this owned group."""
    if child.poll() is not None:
        return
    os.killpg(child.pid, signal.SIGTERM)
    try:
        child.wait(timeout=45)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        child.wait()


def execute(command, environment, log, timeout):
    """Retain the real exit and stop only this owned process group on timeout."""
    started = time.monotonic()
    with log.open("w") as output:
        child = subprocess.Popen(
            command,
            cwd=AGENT,
            env=environment,
            stdout=output,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        timed_out = False
        try:
            # Once a process exists, even a failed ownership-record write must drain it.
            log.with_suffix(".owner.json").write_text(
                json.dumps({"pid": child.pid, "command": command}) + "\n"
            )
            code = child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            # Popen has not reaped the live child, so its process-group id still belongs
            # to this invocation. TERM lets test:browser write its cleanup receipt.
            stop(child)
            code = child.returncode
        except BaseException:
            stop(child)
            raise
    return {
        "command": command,
        "exitCode": code,
        "timedOut": timed_out,
        "wallSeconds": round(time.monotonic() - started, 3),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=AGENT / ".llm/sdk-live")
    parser.add_argument("--repeat", type=int, default=1)
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("--repeat must be positive")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    suites = sorted(
        path.stem for path in (SDK / "neo-playwright/tests").glob("*_live.rs")
    )
    if not suites:
        parser.error("no live suites found")

    environment = os.environ.copy()
    environment.update(
        CARGO_BUILD_JOBS="4", CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0"
    )
    # The 112-million-pixel tall image exercises PNG's row filters and deflate encoder.
    # Optimize only those test dependencies, preserving the test program and its oracles;
    # an unoptimized codec can consume the screenshot's entire operation deadline.
    cargo = [
        "cargo",
        "+1.98.0",
        "test",
        "--locked",
        "-p",
        "neo-playwright",
        "--config",
        "profile.test.package.png.opt-level=3",
        "--config",
        "profile.test.package.miniz_oxide.opt-level=3",
    ]
    compile_command = ["nice", "-n", "10", *cargo, "--no-run"]
    for suite in suites:
        compile_command.extend(["--test", suite])
    report = {"suites": suites, "testThreads": 1, "runs": []}
    print(f"SDK live: compile {len(suites)} suites", flush=True)
    report["compile"] = execute(
        compile_command, environment, output / "compile.log", 900
    )
    (output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    if report["compile"]["exitCode"]:
        print((output / "compile.log").read_text())
        return 1

    browser = environment.get("NEO_E2E_BROWSER")
    lines = [
        "| Suite | Status | Pass | Fail | Ignored | Wall seconds |",
        "| --- | --- | ---: | ---: | ---: | ---: |",
    ]
    for repeat in range(1, args.repeat + 1):
        for suite in suites:
            child_environment = environment.copy()
            if suite == "expect_live":
                # This suite alone exercises the bundled takeover extension. The wrapper
                # keeps test:browser's profile, port, server-disable flags and cleanup.
                if browser:
                    child_environment["NEO_TAKEOVER_BROWSER"] = browser
                child_environment["NEO_E2E_BROWSER"] = str(
                    SDK / "neo-playwright/tests/support/neo-with-takeover.sh"
                )
            label = f"{repeat}-{suite}"
            log = output / f"{label}.log"
            command = [
                "bun",
                "run",
                "test:browser",
                "--",
                *cargo,
                "--test",
                suite,
                "--",
                "--ignored",
                "--test-threads=1",
            ]
            print(f"SDK live: {label}", flush=True)
            result = execute(command, child_environment, log, 360)
            text = log.read_text(errors="replace")
            print(text, flush=True)
            summaries = list(SUMMARY.finditer(text))
            result.update(suite=suite, repeat=repeat)
            if len(summaries) == 1:
                match = summaries[0]
                result.update(
                    passed=int(match[2]),
                    failed=int(match[3]),
                    ignored=int(match[4]),
                    filtered=int(match[6]),
                    testSeconds=float(match[7]),
                )
            # An empty selection or a crashed harness is not a green suite. Preserve any
            # later suite's evidence even when this one fails.
            result["ok"] = (
                result["exitCode"] == 0
                and not result["timedOut"]
                and result.get("passed", 0) > 0
                and result.get("failed") == 0
                and result.get("ignored") == 0
            )
            report["runs"].append(result)
            (output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
            lines.append(
                f"| {label} | {'PASS' if result['ok'] else 'FAIL'} | "
                f"{result.get('passed', '?')} | {result.get('failed', '?')} | "
                f"{result.get('ignored', '?')} | {result['wallSeconds']} |"
            )
            (output / "summary.md").write_text("\n".join(lines) + "\n")
    if summary := environment.get("GITHUB_STEP_SUMMARY"):
        with Path(summary).open("a") as destination:
            destination.write("\n".join(lines) + "\n")
    return int(any(not result["ok"] for result in report["runs"]))


def interrupted(signum, _frame):
    """Unwind execute so a terminated standalone runner also cleans its browser."""
    raise SystemExit(128 + signum)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    raise SystemExit(main())
