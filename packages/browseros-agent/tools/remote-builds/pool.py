#!/usr/bin/env python3
"""Prioritized first-free VM slots over SSH, without a daemon or cloud credentials.

The sidecar flock protects a small JSON file on the coordinator VM. New clients renew
a short lease; a separate frozen wall deadline also bounds live hung clients. Either
expiry fences admission until the independent reaper proves job and Docker cleanup.
"""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import time

JOB_ID = re.compile(r"[a-f0-9]{32}\Z")
# The owner requires this admission limit even in a freshly initialized hosts file.
WORKER_LIMIT = 2
KINDS = {"gate", "image", "viewer-image", "spec", "detector", "run"}
PRIORITY = {"gate": 0, "image": 1, "viewer-image": 1, "spec": 2, "detector": 2, "run": 2}
# Measured gates took about 5 minutes, amd64 images 8, viewer images 16,
# and a full spec job 13. These generous budgets include sync, exclude queue
# wait, and cannot be renewed by a healthy but hung client.
WALL_SECONDS = {"gate": 20 * 60, "image": 45 * 60, "viewer-image": 45 * 60,
                "spec": 45 * 60, "detector": 15 * 60, "run": 30 * 60}


def wall_seconds(state: dict, kind: str) -> int:
    """Only the coordinator's operator-controlled policy can change finite limits."""
    seconds = state.get("policy", {}).get("wallSeconds", {}).get(kind, WALL_SECONDS[kind])
    if type(seconds) is not int or not 1 <= seconds <= 24 * 60 * 60:
        raise ValueError("wallSeconds must be an integer from 1 through 86400")
    return seconds


def wall_deadline(state: dict, reservation: dict) -> float:
    # Older durable reservations have a start time but no frozen wall field.
    # Derive it from that original start, never from a renewal or upgrade time.
    # Administrators must drain before upgrading: an old over-budget job can
    # immediately become eligible for fenced cancellation under the new policy.
    if "wallUntil" not in reservation:
        reservation["wallUntil"] = reservation["started"] + wall_seconds(state, reservation.get("kind", "run"))
    return reservation["wallUntil"]


def fence_wall(state: dict, reservation: dict, now: float) -> None:
    """Fence a late heartbeat/acquire even before the next independent timer tick."""
    if wall_deadline(state, reservation) <= now:
        reservation.update(state="reaping", expiryReason="hard wall limit")
        raise ValueError("job reached its hard wall limit; cleanup required")


def queue_order(row: dict) -> tuple:
    return PRIORITY[row["kind"]], row["submitted"], row["id"]


def submission(message: dict) -> dict | None:
    """Keep job identity/display metadata separate from SSH and cloud credentials."""
    value = message.get("submission")
    if value is None:
        return None  # Already running clients remain protocol-compatible during rollout.
    if (value.get("kind") not in KINDS or not JOB_ID.fullmatch(value.get("worker", ""))
            or not isinstance(value.get("label"), str) or len(value["label"]) > 120):
        raise ValueError("invalid worker/kind metadata")
    return {**{name: value[name] for name in ("kind", "worker", "label", "clientPid", "clientMachine")},
            "protocol": value.get("protocol", 0)}


def worker_jobs(state: dict, worker: str) -> list[tuple[dict, str]]:
    """Count compute reservations and waiting requests under the same pool lock."""
    running = [(host["job"], "running on " + host["name"]) for host in state["hosts"]
               if (host.get("job") or {}).get("worker") == worker]
    waiting = [(row, "waiting") for row in sorted(state.get("waiting", []), key=lambda row: row["submitted"])
               if row.get("worker") == worker]
    return running + waiting


def refusal(label: str, jobs: list[tuple[dict, str]]) -> str:
    present = "; ".join(f"{row.get('kind', 'run')} {row['id'][:8]} ({where})" for row, where in jobs)
    return f"{label} already has two jobs in the pool: {present}. Wait for one to finish before submitting another."


def enforce_worker_cap(state: dict, now: float) -> None:
    """Grandfather active work, but refuse excess waiters deterministically on upgrade.

    Keep refusals briefly so an acquire reply lost in transit cannot turn a rejected
    third request into an accepted request when another job finishes during retry.
    """
    state["rejected"] = {job: value for job, value in state.get("rejected", {}).items() if value["until"] > now}
    maximum = WORKER_LIMIT
    for worker in {row["worker"] for row in state.get("waiting", [])}:
        jobs = worker_jobs(state, worker)
        keep = jobs[:maximum]
        for row, where in jobs[maximum:]:
            if where == "waiting":
                state["rejected"][row["id"]] = {"reason": refusal(row["label"], keep), "until": now + 3600}
    state["waiting"] = [row for row in state["waiting"] if row["id"] not in state["rejected"]]


def write_state(path: Path, state: dict) -> None:
    """Replace only a complete state file while keeping the flock inode stable."""
    descriptor, name = tempfile.mkstemp(prefix=".pool-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as output:
            json.dump(state, output)
            output.flush()
            os.fsync(output.fileno())
        os.replace(name, path)
        # Persist the rename too: losing the latest busy record after a coordinator
        # reboot could otherwise assign a still-running VM to a second client.
        parent = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        with contextlib.suppress(FileNotFoundError):
            os.unlink(name)


def request(path: Path, message: dict) -> dict:
    operation = message.get("operation")
    if operation not in {"status", "queue", "acquire", "release", "unknown", "unavailable", "heartbeat", "expired", "withdraw", "annotate"}:
        raise ValueError("unknown pool operation")
    job = message.get("job", "")
    if operation not in {"status", "queue", "expired"} and not JOB_ID.fullmatch(job):
        raise ValueError("job must be a 32-character hexadecimal identifier")
    with path.with_suffix(".lock").open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        state = json.loads(path.read_text())
        hosts = state["hosts"]
        if (not hosts or len({host["name"] for host in hosts}) != len(hosts)
                or len({host["host"] for host in hosts}) != len(hosts)):
            raise ValueError("pool hosts must have unique names and canonical endpoints")
        assigned = next((host for host in hosts if (host.get("job") or {}).get("id") == job), None)
        now = time.time()
        waiting = [row for row in state.get("waiting", []) if row["until"] > now]
        state["waiting"] = waiting
        enforce_worker_cap(state, now)
        waiting = state["waiting"]
        if operation in {"status", "queue"}:
            write_state(path, state)
            return {"hosts": hosts, "waiting": sorted(waiting, key=queue_order)}
        if operation == "withdraw":
            state["waiting"] = [row for row in waiting if row["id"] != job]
            write_state(path, state)
            # A lost acquire reply may already own a reservation. Withdrawal only
            # removes waiters; the independent lease reaper fences uncertain slots.
            return {"withdrawn": True}
        if operation == "annotate":
            if assigned is not None:
                assigned["job"].update(submission(message) or {})
            queued = next((row for row in waiting if row["id"] == job), None)
            if queued is not None:
                queued.update(submission(message) or {})
            write_state(path, state)
            return {"annotated": assigned is not None or queued is not None}
        if operation == "expired":
            expired = []
            for host in hosts:
                reservation = host.get("job") or {}
                if not reservation:
                    continue
                deadline = reservation.get("clientUntil")
                reason = ("hard wall limit" if wall_deadline(state, reservation) <= now else
                          "client lease" if deadline is not None and deadline <= now else None)
                if reason:
                    # Claim expiry under the same lock as heartbeat. A delayed
                    # client cannot renew after cancellation has been authorized.
                    reservation["state"] = "reaping"
                    reservation["expiryReason"] = reason
                    expired.append(host)
            write_state(path, state)  # Persist derived legacy wall deadlines too.
            return {"hosts": expired}
        if operation == "acquire":
            metadata = submission(message)
            # Retrying after a lost SSH reply must not consume another slot.
            if assigned is not None:
                try:
                    fence_wall(state, assigned["job"], now)
                except ValueError:
                    write_state(path, state)
                    raise
                if assigned["job"]["state"] != "reserved":
                    raise ValueError("job outcome is unknown; recover it before reuse")
                write_state(path, state)
                return {"host": assigned}
            if job in state["rejected"]:
                write_state(path, state)
                raise ValueError(state["rejected"][job]["reason"])
            if metadata is None or metadata.get("protocol", 0) < 2:
                state["waiting"] = [row for row in waiting if row["id"] != job]
                write_state(path, state)
                raise ValueError("This waiting client predates queue identity. No check ran: rerun the same shared launcher command to use the two-job limit.")
            if metadata is not None:
                queued = next((row for row in waiting if row["id"] == job), None)
                if queued is None:
                    existing = worker_jobs(state, metadata["worker"])
                    if len(existing) >= WORKER_LIMIT:
                        reason = refusal(metadata["label"], existing[:2])
                        state["rejected"][job] = {"reason": reason, "until": now + 3600}
                        write_state(path, state)
                        raise ValueError(reason)
                    queued = {"id": job, "submitted": now, **metadata}
                    waiting.append(queued)
                queued["until"] = now + 10
                metadata = {**{name: queued[name] for name in ("kind", "worker", "label", "clientPid", "clientMachine")},
                            "protocol": queued.get("protocol", 0)}
            free = next((host for host in hosts if not host.get("job")
                         and not host.get("maintenance")
                         and host.get("unavailableUntil", 0) <= time.time()), None)
            if free is None:
                write_state(path, state)
                return {"host": None}
            if waiting:
                # Only the highest-priority live waiter can claim a free host.
                # We do not reserve on somebody else's behalf: their next poll
                # claims it, or their ten-second expiry removes the dead client.
                # FIFO within each class avoids racing clients jumping the queue.
                if min(waiting, key=queue_order)["id"] != job:
                    write_state(path, state)
                    return {"host": None}
            state["waiting"] = [row for row in waiting if row["id"] != job]
            free["job"] = {"id": job, "started": now, "state": "reserved", **(metadata or {}),
                           "wallUntil": now + wall_seconds(state, metadata["kind"])}
            if message.get("lease"):
                free["job"]["clientUntil"] = min(now + 120, free["job"]["wallUntil"])
            write_state(path, state)
            return {"host": free}
        if assigned is None:
            return {"released": False}
        if operation == "heartbeat":
            reservation = assigned["job"]
            try:
                fence_wall(state, reservation, now)
            except ValueError:
                write_state(path, state)
                raise
            if reservation["state"] != "reserved":
                raise ValueError("job lease no longer renewable; cleanup required")
            if "clientUntil" not in reservation:
                raise ValueError("legacy reservation has no renewable lease")
            reservation["clientUntil"] = min(now + 120, reservation["wallUntil"])
            write_state(path, state)
            return {"renewed": True}
        if operation == "unavailable":
            # This operation is only for the client's pre-dispatch SSH check. No
            # source or command has reached this reservation, so it can be retried
            # elsewhere without weakening the terminal-result fence for real jobs.
            if assigned["job"]["state"] != "reserved":
                raise ValueError("cannot quarantine an uncertain running job")
            assigned["unavailableUntil"] = time.time() + 120
            assigned["job"] = None
            write_state(path, state)
            return {"quarantined": True}
        if operation == "release":
            assigned["job"] = None
            write_state(path, state)
            return {"released": True}
        assigned["job"]["state"] = "unknown"
        write_state(path, state)
        return {"unknown": True}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, default=Path("/var/lib/browseros-remote-builds/pool.json"))
    arguments = parser.parse_args()
    try:
        message = json.loads(sys.stdin.read(65537))
        print(json.dumps(request(arguments.state, message)))
        return 0
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({"error": str(error)}))
        print(str(error), file=sys.stderr)  # Pre-upgrade clients read SSH stderr on nonzero exit.
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
