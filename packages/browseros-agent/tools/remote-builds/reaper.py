#!/usr/bin/env python3
"""Cancel abandoned or over-budget jobs independently of every Mac client.

A systemd timer runs this module. Expiry first fences renewal under the pool lock;
only the guest's durable terminal receipt permits reuse. A restricted internal SSH
key can cancel UUIDs but cannot execute arbitrary commands on the build workers.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess

from pool import request


def sweep(state: Path, profile: dict) -> None:
    for host in request(state, {"operation": "expired"})["hosts"]:
        job = host["job"]["id"]
        reason = host["job"].get("expiryReason", "expired lease")
        endpoint = profile["hosts"][host["name"]]
        arguments = ["ssh", "-i", profile["key"], "-o", "BatchMode=yes",
                     "-o", "IdentitiesOnly=yes", "-o", "ForwardAgent=no",
                     "-o", "StrictHostKeyChecking=yes", "-o", "UserKnownHostsFile=" + profile["knownHosts"],
                     "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=10",
                     "-o", "ServerAliveCountMax=2", "buildrunner@" + endpoint, "lease-cancel"]
        try:
            reply = subprocess.run(arguments, input=json.dumps({"job": job}), text=True,
                                   capture_output=True, timeout=90, check=True)
            receipt = json.loads(reply.stdout)
            if receipt.get("job") != job or receipt.get("safeToRelease") is not True:
                raise ValueError("guest cleanup remains uncertain")
            # release matches the UUID again while holding the pool lock. A delayed
            # result for A can never release B's later reservation on the same host.
            request(state, {"operation": "release", "job": job})
            print(f"{host['name']}: {reason} {job} stopped and released", flush=True)
        except (OSError, ValueError, subprocess.SubprocessError):
            print(f"{host['name']}: {reason} {job} retained pending cleanup", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, default=Path("/var/lib/browseros-remote-builds/pool.json"))
    parser.add_argument("--config", type=Path, default=Path("/var/lib/browseros-remote-builds/reaper.json"))
    args = parser.parse_args()
    sweep(args.state, json.loads(args.config.read_text()))


if __name__ == "__main__":
    main()
