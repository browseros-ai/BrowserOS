#!/usr/bin/env python3
"""Run one uploaded snapshot in a named systemd service and fence cancellation.

Systemd owns the process tree, including detached descendants. The per-job flock
covers the start handoff and completion; a durable cancel marker prevents a late
SSH invocation from starting a job after its slot has been recovered.
"""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import re
import signal
import shutil
import subprocess
import sys
import tempfile
import time

JOB_ID = re.compile(r"[a-f0-9]{32}\Z")
ROOT = Path("/var/lib/browseros-remote-builds/jobs")


def unit_name(job: str) -> str:
    if not JOB_ID.fullmatch(job):
        raise ValueError("invalid job identifier")
    return "remote-build-" + job + ".service"


def atomic_json(path: Path, value: dict) -> None:
    descriptor, temporary = tempfile.mkstemp(prefix=".job-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as output:
            json.dump(value, output)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        sync_directory(path.parent)
    finally:
        with contextlib.suppress(FileNotFoundError):
            os.unlink(temporary)


def sync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def active(unit: str) -> bool:
    probe = subprocess.run(["systemctl", "show", unit, "--property=ActiveState", "--value"],
                           capture_output=True, text=True)
    # Failure to inspect systemd is not evidence that the process tree stopped.
    return probe.returncode != 0 or probe.stdout.strip() not in {"inactive", "failed"}


def docker_ids(kind: str) -> set[str]:
    arguments = ["docker", kind, "ls", "--quiet"]
    if kind == "container":
        arguments.extend(["--all", "--no-trunc"])
    elif kind == "network":
        # Docker can recreate its built-in bridge with another ID on restart.
        # Only custom networks can belong to a job; never try to remove host defaults.
        arguments.extend(["--filter", "type=custom"])
    probe = subprocess.run(arguments,
                           capture_output=True, text=True, timeout=20, check=True)
    return set(probe.stdout.split())


def docker_baseline() -> dict:
    """Record resource identities before the sole job on this managed build host."""
    probe = subprocess.run(["systemctl", "is-active", "docker.service"], capture_output=True)
    if probe.returncode != 0:
        raise ValueError("Docker must be running and inspectable before admitting a job")
    running = subprocess.run(["docker", "ps", "--quiet"], capture_output=True, text=True,
                             check=True, timeout=20)
    if running.stdout.strip():
        raise ValueError("build host has running containers outside this job; refusing to start")
    return {kind: sorted(docker_ids(kind)) for kind in ("container", "network", "volume")}


def cleanup_docker(job: str, cancelled: bool = False) -> None:
    """Docker work escapes the service cgroup, so cancellation also stops its daemon.

    Pool VMs are dedicated workers with one admitted job and no independent running
    containers. The persisted baseline fences deletion to resources this job created;
    cached images and BuildKit caches deliberately survive ordinary completion.
    """
    path = ROOT / (job + ".docker.json")
    if not path.exists():
        return
    baseline = json.loads(path.read_text())
    if set(baseline) != {"container", "network", "volume"}:
        raise ValueError("missing Docker inventory; cleanup cannot be proven")
    if cancelled:
        subprocess.run(["sudo", "-n", "systemctl", "restart", "docker.service"],
                       check=True, timeout=60, capture_output=True)
    for kind in ("container", "network", "volume"):
        created = sorted(docker_ids(kind) - set(baseline[kind]))
        if created:
            subprocess.run(["docker", kind, "rm", *( ["--force"] if kind == "container" else []),
                            *created], check=True, capture_output=True, timeout=30)


def finish(job: str, receipt: dict) -> dict:
    """Commit the terminal fence before a client may return this host to the pool."""
    log = ROOT / (job + ".log")
    with log.open("ab") as output:
        os.fsync(output.fileno())
    # Callers hold host_guard: persist completion before clearing the durable owner,
    # so a late recovery cannot authorize another host-wide cleanup for this UUID.
    atomic_json(ROOT / (job + ".result.json"), receipt)
    if receipt.get("safeToRelease"):
        atomic_json(ROOT / "owner.json", {"job": None})
    return receipt


@contextlib.contextmanager
def host_guard(job: str):
    """Fence host-wide cleanup against another UUID, including an orphaned service."""
    with (ROOT / "host.lock").open("a+") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError("another job owns this host; cleanup/admission refused") from None
        owner_path = ROOT / "owner.json"
        previous = json.loads(owner_path.read_text()).get("job") if owner_path.exists() else None
        if previous and previous != job:
            unit_name(previous)
            result = ROOT / (previous + ".result.json")
            if not result.exists() or not json.loads(result.read_text()).get("safeToRelease"):
                raise ValueError("previous host owner needs recovery before another job can start")
        yield


def add_image(job: str, source: Path, receipt: dict) -> None:
    record = source / ".remote-build-artifacts/image.json"
    if record.exists():
        image = json.loads(record.read_text())
        if image.get("tag") != "neo-sdk:remote-" + job or not re.fullmatch(r"sha256:[0-9a-f]{64}", image.get("id", "")):
            raise ValueError("image receipt must name this job's image and a complete digest")
        receipt["image"] = image


def process_start(pid: int) -> str:
    """Linux start ticks distinguish our SSH streamer from a later reused PID."""
    return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19]


def cancel(job: str, interrupt_streamer: bool = False) -> dict:
    unit = unit_name(job)
    ROOT.mkdir(parents=True, exist_ok=True)
    # Keep the marker after recovery. A stale transport may still reach run(),
    # but it cannot cross the admission lock and start fresh heavy work.
    marker = ROOT / (job + ".cancelled")
    with marker.open("a") as output:
        output.flush()
        os.fsync(output.fileno())
    sync_directory(ROOT)
    if interrupt_streamer:
        # A dead client's stdout can block the SSH streamer while it holds both
        # admission locks. Stop only the recorded process incarnation, then let
        # normal cancellation fence the independent systemd/Docker work below.
        info = ROOT / (job + ".info.json")
        if info.exists():
            streamer = json.loads(info.read_text()).get("streamer")
            if streamer and streamer["pid"] != os.getpid():
                with contextlib.suppress(FileNotFoundError, ProcessLookupError):
                    if process_start(streamer["pid"]) == streamer["start"]:
                        os.kill(streamer["pid"], signal.SIGTERM)
    deadline = time.monotonic() + 25
    while True:
        subprocess.run(["sudo", "-n", "systemctl", "stop", unit], capture_output=True, timeout=15)
        with (ROOT / (job + ".lock")).open("a+") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                result = ROOT / (job + ".result.json")
                if result.exists():
                    terminal = json.loads(result.read_text())
                    if terminal.get("safeToRelease"):
                        # A second recovery may arrive after another client released A
                        # and started B on this host. Never repeat A's host-wide cleanup.
                        return terminal
                if not active(unit):
                    with host_guard(job):
                        cleanup_docker(job, cancelled=True)
                        receipt = {"safeToRelease": True, "job": job, "exitCode": 130, "cancelled": True}
                        info = ROOT / (job + ".info.json")
                        if info.exists():
                            metadata = json.loads(info.read_text())
                            receipt["sourceRoot"] = metadata["sourceRoot"]
                            receipt["seconds"] = max(0, time.time() - metadata["startedAt"])
                            add_image(job, Path(metadata["sourceRoot"]), receipt)
                        command_result = ROOT / (job + ".command-result.json")
                        if command_result.exists():
                            outcome = json.loads(command_result.read_text())
                            receipt.update(exitCode=outcome["exitCode"], seconds=outcome["seconds"],
                                           cancelled=outcome.get("cancelled", False), recovered=True)
                        return finish(job, receipt)
            except BlockingIOError:
                pass
        if time.monotonic() >= deadline:
            return {"safeToRelease": False, "job": job}
        time.sleep(0.2)


def run(arguments: argparse.Namespace) -> int:
    unit = unit_name(arguments.job)
    source = arguments.source_root.resolve()
    namespace = Path("/work/remote-builds") / arguments.job
    if not source.is_relative_to(namespace) or source == namespace:
        raise ValueError("source must be inside this job's Crabbox workspace")
    cwd = (source / arguments.cwd).resolve()
    if not cwd.is_relative_to(source):
        raise ValueError("working directory must stay inside the uploaded snapshot")
    if not re.fullmatch(r"(?:[a-f0-9]{40}|[a-f0-9]{64})", arguments.revision):
        raise ValueError("invalid source revision")
    command = arguments.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        raise ValueError("missing command")
    ROOT.mkdir(parents=True, exist_ok=True)
    with (ROOT / (arguments.job + ".lock")).open("a+") as lock, host_guard(arguments.job):
        fcntl.flock(lock, fcntl.LOCK_EX)
        if (ROOT / (arguments.job + ".cancelled")).exists():
            raise ValueError("job was cancelled before it started")
        if (ROOT / (arguments.job + ".info.json")).exists() or (ROOT / (arguments.job + ".result.json")).exists():
            raise ValueError("job was already admitted; recover instead of executing it twice")
        if active(unit):
            raise ValueError("job service already exists; recover instead of starting it twice")
        atomic_json(ROOT / (arguments.job + ".info.json"),
                    {"sourceRoot": str(source), "startedAt": time.time(),
                     "streamer": {"pid": os.getpid(), "start": process_start(os.getpid())}})
        log_path = ROOT / (arguments.job + ".log")
        log_path.touch(mode=0o600, exist_ok=True)
        atomic_json(ROOT / (arguments.job + ".docker.json"), docker_baseline())
        started = time.monotonic()
        home = Path.home()
        environment = {
            "HOME": str(home),
            "PATH": f"{home}/.cargo/bin:{home}/.bun/bin:/usr/local/bin:/usr/bin:/bin",
            "CARGO_BUILD_JOBS": str(os.cpu_count() or 1),
            "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0",
            # One admitted job per VM permits dependency-cache reuse across snapshots.
            # Source roots remain distinct, so Cargo sees each worktree's own inputs.
            "CARGO_TARGET_DIR": "/work/remote-builds/cache/target",
            "CARGO_BUILD_BUILD_DIR": "/work/remote-builds/cache/build-private",
            "NEO_SDK_SOURCE_ROOT": str(source),
            "NEO_SDK_SOURCE_REVISION": arguments.revision,
            "NEO_SDK_SOURCE_DIRTY": arguments.dirty,
            "REMOTE_BUILD_JOB": arguments.job,
            "REMOTE_BUILD_ARTIFACTS": str(source / ".remote-build-artifacts"),
            "NEO_NODE_OUTPUT_DIR": str(source / ".remote-build-artifacts/spec"),
            "NEO_SDK_BUILD_JOBS": str(os.cpu_count() or 1),
            "DOCKER_CONFIG": str(ROOT / (arguments.job + ".docker-config")),
        }
        Path(environment["DOCKER_CONFIG"]).mkdir(mode=0o700, exist_ok=True)
        argv = ["sudo", "-n", "systemd-run", "--quiet", "--wait", "--collect",
                "--unit=" + unit, "--uid=" + str(os.getuid()), "--gid=" + str(os.getgid()),
                "--property=WorkingDirectory=" + str(cwd), "--property=KillMode=control-group",
                "--property=TimeoutStopSec=10s",
                "--property=StandardOutput=append:" + str(log_path),
                "--property=StandardError=append:" + str(log_path)]
        argv.extend("--setenv=" + key + "=" + value for key, value in environment.items())
        # SSH death drops the flock, but the systemd service can survive it. A new
        # caller must recover this persisted owner before admitting another UUID.
        atomic_json(ROOT / "owner.json", {"job": arguments.job})
        # No ambient cloud, registry, agent or personal environment is inherited.
        # The service writes to a durable file independently of SSH. A disconnected
        # streamer cannot take its logs with it; recovery can collect the whole file.
        with log_path.open("ab", buffering=0) as output, log_path.open("rb") as stream:
            payload = ["python3", str(Path(__file__).resolve()), "--root", str(ROOT),
                       "payload", "--job", arguments.job, "--", *command]
            process = subprocess.Popen([*argv, "--", *payload], stdout=output, stderr=subprocess.STDOUT)
            while True:
                data = stream.read()
                if data:
                    sys.stdout.buffer.write(data)
                    sys.stdout.buffer.flush()
                if process.poll() is not None:
                    sys.stdout.buffer.write(stream.read())
                    sys.stdout.buffer.flush()
                    break
                time.sleep(0.1)
        cleanup_docker(arguments.job)
        receipt = {"job": arguments.job, "exitCode": process.returncode,
                   "seconds": time.monotonic() - started, "safeToRelease": not active(unit),
                   "sourceRoot": str(source)}
        add_image(arguments.job, source, receipt)
        finish(arguments.job, receipt)
        print("REMOTE_BUILD_RESULT=" + json.dumps(receipt), flush=True)
        return process.returncode


def payload(arguments: argparse.Namespace) -> int:
    """The systemd-owned child records exit status independently of the SSH streamer."""
    job = arguments.job
    unit_name(job)
    owner = json.loads((ROOT / "owner.json").read_text())
    if owner.get("job") != job:
        raise ValueError("payload no longer owns this worker")
    command = arguments.command[1:] if arguments.command[:1] == ["--"] else arguments.command
    if not command:
        raise ValueError("missing payload command")
    started = time.monotonic()
    cancelled = (ROOT / (job + ".cancelled")).exists()
    code = 130 if cancelled else subprocess.run(command).returncode
    if code < 0:
        code = 128 - code
    # A service killed during cancellation has no normal command outcome; recovery
    # records that as cancellation. A completed command keeps its true exit status.
    atomic_json(ROOT / (job + ".command-result.json"),
                {"job": job, "exitCode": code, "seconds": time.monotonic() - started,
                 "cancelled": cancelled})
    return code


def main() -> int:
    global ROOT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    sub = parser.add_subparsers(dest="operation", required=True)
    execute = sub.add_parser("run")
    execute.add_argument("--job", required=True)
    execute.add_argument("--source-root", type=Path, required=True)
    execute.add_argument("--cwd", default=".")
    execute.add_argument("--revision", required=True)
    execute.add_argument("--dirty", choices=["true", "false"], required=True)
    execute.add_argument("command", nargs=argparse.REMAINDER)
    child = sub.add_parser("payload")
    child.add_argument("--job", required=True)
    child.add_argument("command", nargs=argparse.REMAINDER)
    stop = sub.add_parser("cancel")
    stop.add_argument("--job", required=True)
    read = sub.add_parser("result")
    read.add_argument("--job", required=True)
    sub.add_parser("lease-rpc")
    for operation in ("artifacts", "cleanup", "image"):
        extra = sub.add_parser(operation)
        extra.add_argument("--job", required=True)
    arguments = parser.parse_args()
    ROOT = arguments.root
    try:
        if arguments.operation == "lease-rpc":
            # The coordinator's forced-command SSH key can only invoke this narrow
            # cancellation protocol. It cannot upload files or open a general shell.
            if os.environ.get("SSH_ORIGINAL_COMMAND") != "lease-cancel":
                raise ValueError("restricted coordinator command")
            request = json.loads(sys.stdin.read(4097))
            if set(request) != {"job"}:
                raise ValueError("expected a job identifier only")
            print(json.dumps(cancel(request["job"], interrupt_streamer=True)))
            return 0
        if arguments.operation == "run":
            return run(arguments)
        if arguments.operation == "payload":
            return payload(arguments)
        if arguments.operation == "cancel":
            print(json.dumps(cancel(arguments.job)))
        elif arguments.operation == "result":
            unit_name(arguments.job)
            print((ROOT / (arguments.job + ".result.json")).read_text())
        else:
            unit_name(arguments.job)
            receipt = json.loads((ROOT / (arguments.job + ".result.json")).read_text())
            if not receipt["safeToRelease"]:
                raise ValueError("job cleanup has not been confirmed")
            if arguments.operation == "image":
                image = receipt["image"]
                if image["tag"] != "neo-sdk:remote-" + arguments.job:
                    raise ValueError("refusing to export another job's image")
                actual = subprocess.check_output(["docker", "image", "inspect", image["tag"],
                                                  "--format", "{{.Id}}"], text=True).strip()
                if actual != image["id"]:
                    raise ValueError("remote image no longer matches this job's receipt")
                # Stream through SSH; neither the source tree nor the VM needs an
                # additional multi-gigabyte tar file. Check both sides of the pipeline.
                with subprocess.Popen(["docker", "image", "save", image["tag"]], stdout=subprocess.PIPE) as export:
                    compressed = subprocess.run(["gzip", "-1"], stdin=export.stdout)
                    export.stdout.close()
                    code = export.wait()
                if code or compressed.returncode:
                    raise ValueError("image export failed")
                return 0
            namespace = Path("/work/remote-builds") / arguments.job
            if arguments.operation == "cleanup":
                with contextlib.suppress(FileNotFoundError):
                    shutil.rmtree(namespace)
                print(json.dumps({"cleaned": True}))
            else:
                archive = ROOT / (arguments.job + ".artifacts.tar.gz")
                if archive.exists():
                    with archive.open("rb") as saved:
                        shutil.copyfileobj(saved, sys.stdout.buffer)
                    return 0
                # Keep reports/screenshots while omitting the copied public corpus and
                # dependency trees. The independent service log survives SSH death and
                # is available even when execution never reached source admission.
                command = ["tar", "-czf", "-", "--exclude=*/upstream/tests",
                           "--exclude=*/upstream/packages", "--exclude=*/upstream/node_modules"]
                if receipt.get("sourceRoot"):
                    source = Path(receipt["sourceRoot"]).resolve()
                    if not source.is_relative_to(namespace):
                        raise ValueError("artifact source escaped this job's namespace")
                    artifacts = source / ".remote-build-artifacts"
                    if artifacts.exists():
                        if not artifacts.resolve().is_relative_to(namespace):
                            raise ValueError("artifact directory escaped this job's namespace")
                        command.extend(["-C", str(artifacts), "."])
                log = ROOT / (arguments.job + ".log")
                log.touch(mode=0o600, exist_ok=True)
                command.extend(["-C", str(ROOT), log.name, arguments.job + ".result.json"])
                # Freeze the result bundle before acknowledging collection. A later
                # recovery (even from another Mac) can fetch it after source cleanup.
                with (ROOT / (arguments.job + ".artifacts.lock")).open("a+") as archive_lock:
                    fcntl.flock(archive_lock, fcntl.LOCK_EX)
                    if not archive.exists():
                        descriptor, temporary = tempfile.mkstemp(prefix=".artifacts-", dir=ROOT)
                        try:
                            with os.fdopen(descriptor, "wb") as output:
                                subprocess.run(command, stdout=output, check=True)
                                output.flush()
                                os.fsync(output.fileno())
                            os.replace(temporary, archive)
                            sync_directory(ROOT)
                        finally:
                            with contextlib.suppress(FileNotFoundError):
                                os.unlink(temporary)
                with archive.open("rb") as saved:
                    shutil.copyfileobj(saved, sys.stdout.buffer)
        return 0
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        if arguments.operation in {"run", "payload"} and JOB_ID.fullmatch(arguments.job):
            with contextlib.suppress(OSError):
                with (ROOT / (arguments.job + ".log")).open("a") as output:
                    output.write(str(error) + "\n")
        print(json.dumps({"error": str(error)}), file=sys.stderr)
        return 70


if __name__ == "__main__":
    raise SystemExit(main())
