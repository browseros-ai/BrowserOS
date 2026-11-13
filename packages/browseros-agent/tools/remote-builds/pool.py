#!/usr/bin/env python3
"""Shared first-free VM slots reached over SSH, without a daemon or cloud credentials.

The sidecar flock protects a small JSON file on the coordinator VM. New clients renew
a short lease; expiry fences admission until the independent reaper proves job cleanup.
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
    if operation not in {"status", "acquire", "release", "unknown", "unavailable", "heartbeat", "expired"}:
        raise ValueError("unknown pool operation")
    job = message.get("job", "")
    if operation not in {"status", "expired"} and not JOB_ID.fullmatch(job):
        raise ValueError("job must be a 32-character hexadecimal identifier")
    with path.with_suffix(".lock").open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        state = json.loads(path.read_text())
        hosts = state["hosts"]
        if (not hosts or len({host["name"] for host in hosts}) != len(hosts)
                or len({host["host"] for host in hosts}) != len(hosts)):
            raise ValueError("pool hosts must have unique names and canonical endpoints")
        assigned = next((host for host in hosts if (host.get("job") or {}).get("id") == job), None)
        if operation == "status":
            return {"hosts": hosts}
        if operation == "expired":
            expired = []
            for host in hosts:
                reservation = host.get("job") or {}
                deadline = reservation.get("clientUntil")
                if deadline is not None and deadline <= time.time():
                    # Claim expiry under the same lock as heartbeat. A delayed
                    # client cannot renew after cancellation has been authorized.
                    reservation["state"] = "reaping"
                    expired.append(host)
            if expired:
                write_state(path, state)
            return {"hosts": expired}
        if operation == "acquire":
            # Retrying after a lost SSH reply must not consume another slot.
            if assigned is not None:
                if assigned["job"]["state"] != "reserved":
                    raise ValueError("job outcome is unknown; recover it before reuse")
                return {"host": assigned}
            free = next((host for host in hosts if not host.get("job")
                         and not host.get("maintenance")
                         and host.get("unavailableUntil", 0) <= time.time()), None)
            if free is None:
                return {"host": None}
            free["job"] = {"id": job, "started": time.time(), "state": "reserved"}
            if message.get("lease"):
                free["job"]["clientUntil"] = time.time() + 120
            write_state(path, state)
            return {"host": free}
        if assigned is None:
            return {"released": False}
        if operation == "heartbeat":
            reservation = assigned["job"]
            if reservation["state"] != "reserved":
                raise ValueError("job lease no longer renewable; cleanup required")
            if "clientUntil" not in reservation:
                raise ValueError("legacy reservation has no renewable lease")
            reservation["clientUntil"] = time.time() + 120
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
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
