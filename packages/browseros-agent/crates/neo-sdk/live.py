#!/usr/bin/env python3
"""Compile the live SDK tests once, then give each suite a private browser.

Cases within a suite run serially because independent CDP clients attach to the
same browser tabs: one client's teardown can clear another's emulation. Cases
that deliberately create multiple clients retain that coverage inside the case.
"""

import argparse
from dataclasses import dataclass
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

EARLY_RETURN = "LIVE_TEST_EARLY_RETURN "


@dataclass(frozen=True)
class Suite:
    """One integration target and the libtest selection that exercises its browser rows."""
    package: str
    target: str
    ignored_only: bool = True


def assess_result(result, text):
    """Libtest counts an early Ok return as a pass; explicit receipts keep it out of coverage."""
    checked = dict(result)
    checked["earlyReturns"] = text.count(EARLY_RETURN)
    summaries = list(SUMMARY.finditer(text))
    if len(summaries) == 1:
        match = summaries[0]
        checked.update(
            nativePassed=int(match[2]),
            passed=max(0, int(match[2]) - checked["earlyReturns"]),
            failed=int(match[3]),
            ignored=int(match[4]),
            filtered=int(match[6]),
            testSeconds=float(match[7]),
        )
    checked["ok"] = (
        checked["exitCode"] == 0
        and not checked["timedOut"]
        and checked.get("passed", 0) > 0
        and checked.get("failed") == 0
        and checked.get("ignored") == 0
        and checked["earlyReturns"] == 0
    )
    return checked


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



def publish_summary(output, lines, environment):
    """Keep the same compile cost and suite counts in local artifacts and the CI summary."""
    text = "\n".join(lines) + "\n"
    (output / "summary.md").write_text(text)
    if summary := environment.get("GITHUB_STEP_SUMMARY"):
        with Path(summary).open("a") as destination:
            destination.write(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=AGENT / ".llm/sdk-live")
    parser.add_argument("--repeat", type=int, default=1)
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("--repeat must be positive")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    suites = [
        Suite("neo-playwright", path.stem)
        for path in sorted((SDK / "neo-playwright/tests").glob("*_live.rs"))
    ]
    if not suites:
        parser.error("no live suites found")
    suites.extend([
        Suite("neo-server", "acceptance_browseros"),
        Suite("neo-server", "probe_reply_browseros", ignored_only=False),
        Suite("neo-server", "snapshot_unmarked_browseros", ignored_only=False),
    ])

    environment = os.environ.copy()
    environment.setdefault("CARGO_BUILD_JOBS", "4")
    environment.setdefault("CARGO_PROFILE_DEV_DEBUG", "0")
    environment.setdefault("CARGO_PROFILE_TEST_DEBUG", "0")
    # Acceptance's owned-browser Rig can return early without this variable even
    # when --ignored selected its rows. Give both browser consumers the same binary.
    browser = environment.get("NEO_E2E_BROWSER") or environment.get("CLAW_SDK_E2E_BROWSER")
    if not browser or not Path(browser).is_file() or not os.access(browser, os.X_OK):
        parser.error("set NEO_E2E_BROWSER to the pinned browser executable")
    browser = str(Path(browser).resolve())
    environment.update(NEO_E2E_BROWSER=browser, CLAW_SDK_E2E_BROWSER=browser)
    # CI selects test-profile optimisation through its environment: the live host must
    # parse and distribute large console events within the same deadlines as production.
    # Keep the tall-image codecs at level 3 even when the rest of the test build uses 2.
    cargo = [
        "cargo",
        "+1.98.0",
        "test",
        "--locked",
        # Keep both packages selected in every invocation. Their unified dependency
        # features must match the precompile; --test still selects only this target.
        "-p",
        "neo-playwright",
        "-p",
        "neo-server",
        "--config",
        "profile.test.package.png.opt-level=3",
        "--config",
        "profile.test.package.miniz_oxide.opt-level=3",
    ]
    compile_command = ["nice", "-n", "10", *cargo, "--no-run"]
    for suite in suites:
        compile_command.extend(["--test", suite.target])
    report = {
        "suites": [
            {"package": suite.package, "target": suite.target, "ignoredOnly": suite.ignored_only}
            for suite in suites
        ],
        "testThreads": 1,
        "build": {
            "profile": "test",
            "testOptLevel": environment.get("CARGO_PROFILE_TEST_OPT_LEVEL", "0"),
            "devOptLevel": environment.get("CARGO_PROFILE_DEV_OPT_LEVEL", "0"),
            "jobs": environment["CARGO_BUILD_JOBS"],
        },
        "runs": [],
    }
    print(f"SDK live: compile {len(suites)} suites", flush=True)
    report["compile"] = execute(
        compile_command, environment, output / "compile.log", 900
    )
    (output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    build = report["build"]
    compile_note = (
        f"Compile {len(suites)} integration targets: {report['compile']['wallSeconds']} s; "
        f"test profile opt-level {build['testOptLevel']}, "
        f"dev opt-level {build['devOptLevel']}, {build['jobs']} build jobs."
    )
    print(compile_note, flush=True)
    lines = [
        "## SDK live suites", "", compile_note, "",
        "| Suite | Status | Pass | Fail | Early return | Ignored | Wall seconds |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: |",
    ]
    if report["compile"]["exitCode"]:
        print((output / "compile.log").read_text())
        publish_summary(output, lines, environment)
        return 1
    for repeat in range(1, args.repeat + 1):
        for suite in suites:
            child_environment = environment.copy()
            if suite.target == "expect_live":
                # This suite alone exercises the bundled takeover extension. The wrapper
                # keeps test:browser's profile, port, server-disable flags and cleanup.
                if browser:
                    child_environment["NEO_TAKEOVER_BROWSER"] = browser
                child_environment["NEO_E2E_BROWSER"] = str(
                    SDK / "neo-playwright/tests/support/neo-with-takeover.sh"
                )
            label = f"{repeat}-{suite.target}"
            log = output / f"{label}.log"
            command = [
                "bun",
                "run",
                "test:browser",
                "--",
                *cargo,
                "--test",
                suite.target,
                "--",
                *(["--ignored"] if suite.ignored_only else []),
                "--nocapture",
                "--test-threads=1",
            ]
            print(f"SDK live: {label}", flush=True)
            result = execute(command, child_environment, log, 360)
            text = log.read_text(errors="replace")
            print(text, flush=True)
            result.update(package=suite.package, suite=suite.target, repeat=repeat)
            # Every selected row must exercise coverage: missing output, no tests, ignored
            # rows and explicit precondition returns fail even when libtest returned zero.
            result = assess_result(result, text)
            report["runs"].append(result)
            (output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
            lines.append(
                f"| {label} | {'PASS' if result['ok'] else 'FAIL'} | "
                f"{result.get('passed', '?')} | {result.get('failed', '?')} | "
                f"{result['earlyReturns']} | {result.get('ignored', '?')} | {result['wallSeconds']} |"
            )
            (output / "summary.md").write_text("\n".join(lines) + "\n")
    publish_summary(output, lines, environment)
    return int(any(not result["ok"] for result in report["runs"]))


def interrupted(signum, _frame):
    """Unwind execute so a terminated standalone runner also cleans its browser."""
    raise SystemExit(128 + signum)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    raise SystemExit(main())
