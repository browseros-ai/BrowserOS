#!/usr/bin/env python3
"""Turn queued GitHub Actions jobs into ordinary pool jobs that each run one runner.

This dispatcher runs as a service on the coordinator. It counts the queued jobs of the
allowed workflow that ask for the pool's label. For each one it takes a slot through the
same `pool.py acquire` as `remote.py`, with the same queue, priority, per-worker cap,
lease and hard wall. Only once the slot is granted does it mint a one-job runner
registration. It then starts that runner on the granted machine through the restricted
`guest.py runner-rpc` key:

    queued job ──► acquire (same queue) ──► generate-jitconfig ──► runner-start <uuid>
                                                                     │ guest.py run: systemd unit
    release ◄── result: safeToRelease ◄── runner exits after one job ◄┘ heartbeat every 20 s

No runner exists while no slot is held, so a runner job and a `remote.py` job never
share a machine. If this process dies, its heartbeats stop and the coordinator's reaper
cancels its jobs after the two-minute lease, exactly as for a lost developer client.
"""
from __future__ import annotations

import argparse
import base64
import contextlib
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

from pool import WORKER_LIMIT, request as pool_request

POLL_SECONDS = 15
ACQUIRE_SECONDS = 2
WATCH_SECONDS = 1
# A runner minted for a queued job is normally assigned within seconds. One still idle
# after this long lost its job (the run was cancelled, or another runner took it). It is
# withdrawn, so a pool slot never waits on GitHub indefinitely.
IDLE_SECONDS = 180
RUNNER_PREFIX = "browseros-pool-"
# GitHub jobs are one pool worker: together they hold at most WORKER_LIMIT machines,
# running plus waiting. The rest stay queued on GitHub without holding a slot.
SUBMISSION = {"kind": "run", "worker": hashlib.sha256(b"github-actions").hexdigest()[:32],
              "label": "github-actions", "clientPid": os.getpid(), "clientMachine": "coordinator",
              "protocol": 2}


def log(message: str) -> None:
    print(message, flush=True)


@dataclass(frozen=True)
class Policy:
    """Which GitHub jobs may have a pool runner; also handed to each runner's hook."""
    repository: str
    workflow: str
    branches: tuple[str, ...]
    label: str

    def workflow_refs(self) -> list[str]:
        return [f"{self.repository}/{self.workflow}@refs/heads/{branch}" for branch in self.branches]

    def admits(self, run: dict) -> bool:
        """A dispatched run of the allowed workflow file and branch, in this repository.

        A fork's pull request (event pull_request, another head repository) never
        counts, so no runner is ever minted while only such a job waits.
        """
        return (run.get("event") == "workflow_dispatch" and run.get("path") == self.workflow
                and run.get("head_branch") in self.branches
                and (run.get("repository") or {}).get("full_name") == self.repository
                and (run.get("head_repository") or {}).get("full_name") == self.repository)


def pool_jobs_waiting(runs: list[dict], jobs_of, policy: Policy) -> int:
    """Count queued jobs that ask for the pool label in admitted runs.

    A run's pool job counts only once its other jobs have finished. In pool-run.yml that
    is the GitHub-hosted check of the request, so a request it refuses never takes a
    machine.
    """
    count = 0
    for run in runs:
        if not policy.admits(run):
            continue
        jobs = jobs_of(run["id"])
        pool_jobs = [job for job in jobs if policy.label in job.get("labels", [])]
        if any(job.get("status") != "completed" for job in jobs if job not in pool_jobs):
            continue
        count += sum(1 for job in pool_jobs if job.get("status") == "queued")
    return count


def runner_group_problem(group: dict, repositories: list[str], policy: Policy) -> str | None:
    """Why the organisation's runner group would let other code reach the pool, if it would.

    The group is the GitHub-side boundary: it keeps every other repository and workflow,
    a fork's pull request included, from being given these runners. The dispatcher mints
    no runner unless it reads back exactly that restriction.
    """
    if group.get("visibility") != "selected" or repositories != [policy.repository]:
        return f"runner group admits {group.get('visibility')} repositories {repositories}, not only {policy.repository}"
    if group.get("restricted_to_workflows") is not True \
            or sorted(group.get("selected_workflows") or []) != sorted(policy.workflow_refs()):
        return f"runner group is not restricted to exactly {policy.workflow_refs()}"
    return None


def runners_to_start(demand: int, supply: int, held: int) -> int:
    """New slots to open: unmet demand, bounded by the pool's per-worker cap.

    `supply` counts slots that can still serve a queued job (waiting for a slot, starting,
    or registered without a job). `held` counts every slot the pool still sees as ours,
    including ones winding down, because the pool's cap counts them too.
    """
    return max(0, min(demand - supply, WORKER_LIMIT - held))


class NotFound(Exception):
    pass


class Conflict(Exception):
    pass


class GitHub:
    """REST calls as the organisation's runner App, with a cached installation token."""
    API = "https://api.github.com"

    def __init__(self, app_id: int, installation_id: int, key: Path):
        self.app_id, self.installation_id, self.key = app_id, installation_id, key
        self.token, self.expires = "", 0.0
        self.lock = threading.Lock()

    def app_jwt(self) -> str:
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric import padding, rsa

        def encode(data: bytes) -> str:
            return base64.urlsafe_b64encode(data).rstrip(b"=").decode()
        now = int(time.time())
        unsigned = (encode(json.dumps({"alg": "RS256", "typ": "JWT"}).encode()) + "." +
                    encode(json.dumps({"iat": now - 60, "exp": now + 540, "iss": str(self.app_id)}).encode()))
        key = serialization.load_pem_private_key(self.key.read_bytes(), password=None)
        if not isinstance(key, rsa.RSAPrivateKey):
            raise ValueError("a GitHub App key is an RSA private key")
        return unsigned + "." + encode(key.sign(unsigned.encode(), padding.PKCS1v15(), hashes.SHA256()))

    def installation_token(self) -> str:
        with self.lock:
            if time.time() > self.expires - 300:
                reply = self.send("POST", f"/app/installations/{self.installation_id}/access_tokens",
                                  None, self.app_jwt())
                self.token = reply["token"]
                self.expires = time.time() + 3000  # GitHub issues one-hour tokens.
            return self.token

    def call(self, method: str, path: str, body: dict | None = None) -> dict:
        return self.send(method, path, body, self.installation_token())

    def send(self, method: str, path: str, body: dict | None, bearer: str) -> dict:
        data = None if body is None else json.dumps(body).encode()
        headers = {"Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28",
                   "Authorization": "Bearer " + bearer, "User-Agent": "browseros-pool-runners"}
        if data is not None:
            headers["Content-Type"] = "application/json"
        call = urllib.request.Request(self.API + path, data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(call, timeout=20) as reply:
                payload = reply.read()
        except urllib.error.HTTPError as error:
            if error.code == 404:
                raise NotFound(path) from None
            if error.code == 422:
                raise Conflict(path) from None
            raise RuntimeError(f"GitHub {method} {path}: HTTP {error.code}") from None
        return json.loads(payload) if payload else {}


class Transport:
    """The dispatcher's restricted SSH key: it reaches only `guest.py runner-rpc`."""

    def __init__(self, key: Path, profile: dict):
        self.key, self.endpoints, self.known_hosts = key, profile["hosts"], profile["knownHosts"]

    def args(self, host: dict, command: str) -> list[str]:
        return ["ssh", "-i", str(self.key), "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes",
                "-o", "ForwardAgent=no", "-o", "StrictHostKeyChecking=yes",
                "-o", "UserKnownHostsFile=" + self.known_hosts, "-o", "ConnectTimeout=10",
                "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=4",
                "buildrunner@" + self.endpoints[host["name"]], command]

    def call(self, host: dict, command: str) -> dict:
        for attempt in range(3):
            reply = subprocess.run(self.args(host, command), capture_output=True, text=True, timeout=700)
            if reply.returncode != 255:  # SSH's own failure; the guest's refusals exit 70.
                break
            time.sleep(5 * (attempt + 1))
        if reply.returncode:
            raise RuntimeError(f"{host['name']} {command.split()[0]}: {reply.stderr.strip()[-300:]}")
        return json.loads(reply.stdout)

    def reachable(self, host: dict) -> bool:
        # The forced command refuses an unknown word with exit 70; SSH itself fails with 255.
        reply = subprocess.run(self.args(host, "probe 00000000000000000000000000000000"),
                               capture_output=True, text=True, timeout=40)
        return reply.returncode != 255

    def start(self, host: dict, job: str, request: dict, errors) -> subprocess.Popen:
        process = subprocess.Popen(self.args(host, "runner-start " + job), stdin=subprocess.PIPE,
                                   stdout=subprocess.DEVNULL, stderr=errors, text=True)
        assert process.stdin is not None
        process.stdin.write(json.dumps(request))
        process.stdin.close()
        return process


class Slot(threading.Thread):
    """One pool reservation that hosts one runner for one GitHub job.

    States: acquiring → starting → idle (registered, no job yet) → busy or finishing → done.
    A slot leaves acquiring for done without a host when demand disappears.
    """

    def __init__(self, dispatcher: Dispatcher):
        super().__init__(daemon=True)
        self.dispatcher = dispatcher
        self.job = uuid.uuid4().hex
        self.state = "acquiring"
        self.wanted = True
        self.host: dict | None = None
        self.runner: int | None = None

    def pool(self, operation: str, **extra) -> dict:
        return pool_request(self.dispatcher.state, {"operation": operation, "job": self.job, **extra})

    def run(self) -> None:
        try:
            self.serve()
        except Exception as error:  # A slot's failure must never stop the dispatcher.
            log(f"{self.job}: {type(error).__name__}: {error}")
        finally:
            self.forget_runner()
            self.state = "done"

    def acquire(self) -> dict | None:
        settings = self.dispatcher.settings
        message = {"lease": True, "submission": SUBMISSION, "hosts": settings["hosts"]}
        while self.wanted:
            host = self.pool("acquire", **message)["host"]
            if host and host["name"] not in settings["hosts"]:
                # A coordinator whose pool.py predates host lists ignores ours. Hand the
                # machine straight back, untouched, and stop until an operator upgrades it.
                self.pool("release")
                self.dispatcher.halt(f"the pool granted {host['name']}, outside {settings['hosts']}: "
                                     "install the current pool.py on the coordinator, then restart")
                return None
            if host and self.dispatcher.transport.reachable(host):
                return host
            if host:
                # Nothing was dispatched: cool this host down for every client, as remote.py does.
                self.pool("unavailable")
                log(f"{self.job}: {host['name']} did not answer; trying another host")
                continue
            time.sleep(ACQUIRE_SECONDS)
        self.pool("withdraw")
        return None

    def serve(self) -> None:
        host = self.host = self.acquire()
        if host is None:
            return
        self.state = "starting"
        dispatcher, settings = self.dispatcher, self.dispatcher.settings
        try:
            problem = dispatcher.group_problem()
            if problem:
                raise RuntimeError(problem)
            # A just-in-time runner carries only the labels named here: GitHub adds no default
            # `self-hosted` label, and the workflow asks for both.
            minted = dispatcher.github.call("POST", f"/orgs/{settings['org']}/actions/runners/generate-jitconfig", {
                "name": RUNNER_PREFIX + self.job[:12], "runner_group_id": settings["runnerGroupId"],
                "labels": ["self-hosted", dispatcher.policy.label], "work_folder": "_work"})
        except Exception:
            self.pool("release")  # Nothing reached the host; the slot is free again.
            raise
        self.runner = minted["runner"]["id"]
        log(f"{self.job}: runner {self.runner} on {host['name']}")
        start = {"jitconfig": minted["encoded_jit_config"], "repository": dispatcher.policy.repository,
                 "workflows": dispatcher.policy.workflow_refs()}
        with tempfile.TemporaryFile("w+") as errors, self.lease():
            process = dispatcher.transport.start(host, self.job, start, errors)
            self.state = "idle"
            self.watch(host, process)
            errors.seek(0)
            if process.returncode:
                log(f"{self.job}: runner job exited {process.returncode}: {errors.read().strip()[-300:]}")
        self.finish(host)

    def watch(self, host: dict, process: subprocess.Popen) -> None:
        started = checked = time.monotonic()
        while process.poll() is None:
            time.sleep(WATCH_SECONDS)
            if self.state != "idle" or time.monotonic() - checked < POLL_SECONDS:
                continue
            checked = time.monotonic()
            # Supervision (and so the lease) lasts as long as the runner job does. A GitHub
            # or SSH error here is retried at the next poll: it must never leave a running
            # job to be cancelled by the reaper.
            busy = self.runner_busy()
            if busy:
                self.state = "busy"
            elif busy is False and time.monotonic() - started > IDLE_SECONDS and self.withdraw_runner():
                self.state = "finishing"  # No longer able to serve a queued job.
                try:
                    self.dispatcher.transport.call(host, "cancel " + self.job)
                except (RuntimeError, OSError, subprocess.SubprocessError) as error:
                    # The deleted registration also ends the runner; its exit ends the watch.
                    log(f"{self.job}: cancel failed: {error}")

    def runner_busy(self) -> bool | None:
        """True once GitHub assigned the runner a job; None when GitHub could not say."""
        path = f"/orgs/{self.dispatcher.settings['org']}/actions/runners/{self.runner}"
        try:
            return bool(self.dispatcher.github.call("GET", path).get("busy"))
        except NotFound:
            return True  # GitHub removes a one-job runner once its job is done.
        except Exception as error:
            log(f"{self.job}: runner status unavailable: {error}")
            return None

    def withdraw_runner(self) -> bool:
        """Delete an unused registration first, so GitHub cannot assign it a job meanwhile."""
        try:
            self.forget_runner(strict=True)
        except Conflict:
            self.state = "busy"  # Assigned in the meantime: let it run.
            return False
        except Exception as error:
            log(f"{self.job}: could not withdraw runner {self.runner}: {error}")
            return False
        log(f"{self.job}: runner idle for {IDLE_SECONDS}s; withdrawn")
        return True

    def forget_runner(self, strict: bool = False) -> None:
        if self.runner is None:
            return
        path = f"/orgs/{self.dispatcher.settings['org']}/actions/runners/{self.runner}"
        try:
            self.dispatcher.github.call("DELETE", path)
        except NotFound:
            pass
        except Conflict:
            if strict:
                raise
            return
        except Exception as error:
            if strict:
                raise
            log(f"{self.job}: could not delete runner {self.runner}: {error}")
            return
        self.runner = None

    @contextlib.contextmanager
    def lease(self):
        """Renew the slot's lease while the runner job lives; the reaper owns expiry."""
        stopped = threading.Event()

        def renew():
            while not stopped.wait(20):
                try:
                    self.pool("heartbeat")
                except (OSError, ValueError) as error:
                    log(f"{self.job}: lease renewal failed: {error}")

        thread = threading.Thread(target=renew, daemon=True)
        thread.start()
        try:
            yield
        finally:
            stopped.set()
            thread.join(timeout=1)

    def finish(self, host: dict) -> None:
        """Release only on the guest's own safe receipt, exactly as remote.py does."""
        transport = self.dispatcher.transport
        try:
            receipt = transport.call(host, "result " + self.job)
        except RuntimeError:
            receipt = transport.call(host, "cancel " + self.job)  # Start never recorded one.
        if receipt.get("job") != self.job or receipt.get("safeToRelease") is not True:
            self.pool("unknown")
            log(f"{self.job}: cleanup unproven on {host['name']}; slot retained for recovery")
            return
        self.pool("release")
        log(f"{self.job}: exit {receipt.get('exitCode')}, slot released")
        transport.call(host, "cleanup " + self.job)


class Dispatcher:
    def __init__(self, settings: dict, github, transport, state: Path):
        self.settings, self.github, self.transport, self.state = settings, github, transport, state
        self.policy = Policy(settings["repository"], settings["workflow"],
                             tuple(settings["branches"]), settings["label"])
        self.slots: list[Slot] = []
        self.halted: str | None = None
        self.group = (0.0, "runner group not checked yet")
        self.group_lock = threading.Lock()

    def halt(self, reason: str) -> None:
        if self.halted is None:
            log("halted: " + reason)
        self.halted = reason

    def group_problem(self) -> str | None:
        """Read the runner group's restriction back, at most once a minute."""
        with self.group_lock:
            checked, problem = self.group
            if time.monotonic() - checked > 60 or checked == 0.0:
                org, group = self.settings["org"], self.settings["runnerGroupId"]
                settings = self.github.call("GET", f"/orgs/{org}/actions/runner-groups/{group}")
                listed = self.github.call("GET", f"/orgs/{org}/actions/runner-groups/{group}/repositories?per_page=100")
                problem = runner_group_problem(settings, [repository["full_name"] for repository in
                                                         listed.get("repositories", [])], self.policy)
                if problem:
                    log("not minting runners: " + problem)
                self.group = (time.monotonic(), problem)
            return problem

    def demand(self) -> int:
        repository, workflow = self.policy.repository, Path(self.policy.workflow).name
        runs = []
        for status in ("queued", "in_progress"):
            query = urllib.parse.urlencode({"event": "workflow_dispatch", "status": status, "per_page": 50})
            runs += self.github.call("GET", f"/repos/{repository}/actions/workflows/{workflow}/runs?{query}")["workflow_runs"]

        def jobs_of(run: int) -> list[dict]:
            return self.github.call("GET", f"/repos/{repository}/actions/runs/{run}/jobs?filter=latest&per_page=100")["jobs"]
        return pool_jobs_waiting(runs, jobs_of, self.policy)

    def remove_stale_runners(self) -> None:
        """Registrations from a previous process that never ran a job are useless; delete them."""
        org, group = self.settings["org"], self.settings["runnerGroupId"]
        listed = self.github.call("GET", f"/orgs/{org}/actions/runner-groups/{group}/runners?per_page=100")
        for runner in listed.get("runners", []):
            if runner["name"].startswith(RUNNER_PREFIX) and runner["status"] == "offline" and not runner["busy"]:
                with contextlib.suppress(NotFound, Conflict):
                    self.github.call("DELETE", f"/orgs/{org}/actions/runners/{runner['id']}")
                    log(f"removed stale runner {runner['name']}")

    def tick(self) -> None:
        if self.halted or self.group_problem():
            return
        demand = self.demand()
        self.slots = [slot for slot in self.slots if slot.state != "done"]
        serving = [slot for slot in self.slots if slot.wanted and slot.state in {"acquiring", "starting", "idle"}]
        # More slots waiting than queued jobs left (a run was cancelled): stop the newest
        # waiters. A slot that already holds a machine finishes through its idle limit.
        surplus = len(serving) - demand
        for slot in reversed([slot for slot in serving if slot.state == "acquiring"]):
            if surplus <= 0:
                break
            slot.wanted = False
            surplus -= 1
        supply = sum(slot.wanted and slot.state in {"acquiring", "starting", "idle"} for slot in self.slots)
        for _ in range(runners_to_start(demand, supply, len(self.slots))):
            slot = Slot(self)
            self.slots.append(slot)
            slot.start()

    def serve(self) -> None:
        self.remove_stale_runners()
        log(f"dispatching {self.policy.workflow_refs()} to {self.settings['hosts']}")
        while True:
            try:
                self.tick()
            except Exception as error:  # GitHub or network trouble: try again next poll.
                log(f"poll failed: {type(error).__name__}: {error}")
            time.sleep(POLL_SECONDS)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path("/var/lib/browseros-remote-builds")
    parser.add_argument("--state", type=Path, default=root / "pool.json")
    parser.add_argument("--config", type=Path, default=root / "runners.json")
    parser.add_argument("--reaper", type=Path, default=root / "reaper.json")
    args = parser.parse_args()
    settings = json.loads(args.config.read_text())
    profile = json.loads(args.reaper.read_text())
    github = GitHub(settings["appId"], settings["installationId"], Path(settings["appKey"]))
    Dispatcher(settings, github, Transport(Path(settings["key"]), profile), args.state).serve()


if __name__ == "__main__":
    sys.exit(main())
