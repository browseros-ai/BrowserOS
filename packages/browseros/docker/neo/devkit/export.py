#!/usr/bin/env python3
"""Copy what a running claw-sdk Neo still holds of its trace into one folder.

`dev.sh export` runs this. It reads the host API directly (bearer token from a
file, never printed), so it works without the gateway, and writes:

  <traces-dir>/export-<YYYYmmdd-HHMMSS>/
    runs.json                 every RunSummary, all pages, newest first
    runs/<runId>.json         each full Run with its operations
    screenshots/<opId>.jpg    each operation's picture, where it has one
    screenshots/index.json    per operation: saved, or missing/expired with Neo's reason
    events.json               the events Neo still holds (its last 1024)
    cli-runs.jsonl            a copy of dev.sh run's own log, joined by requestId

  python3 export.py --host-url http://127.0.0.1:19300 --token-file F --traces-dir D

Standard library only (python3 >= 3.12).
"""

import argparse
import json
import shutil
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Generator
from pathlib import Path

# Neo keeps this many events for resumption (neo-service's event log).
EVENT_RING = 1024
# A replay of the ring arrives at once. These bound the wait when it does not,
# so an export never hangs on a quiet stream.
EVENT_READ_TIMEOUT_S = 5
EVENT_DEADLINE_S = 20


class Host:
    """The host API over TCP, with the bearer header on every request."""

    def __init__(self, url: str, token: str):
        self.url = url.rstrip("/")
        self.token = token

    def open(self, path: str, headers: dict | None = None, timeout: float = 30):
        request = urllib.request.Request(
            self.url + path, headers={"Authorization": f"Bearer {self.token}", **(headers or {})}
        )
        return urllib.request.urlopen(request, timeout=timeout)

    def json(self, path: str):
        with self.open(path) as reply:
            return json.load(reply)


def list_runs(host: Host) -> list[dict]:
    runs, cursor = [], None
    while True:
        query = {"limit": 100, **({"cursor": cursor} if cursor else {})}
        page = host.json(f"/v1/runs?{urllib.parse.urlencode(query)}")
        runs += page["items"]
        cursor = page.get("nextCursor")
        if not cursor:
            return runs


def save_screenshot(host: Host, operation_id: str, folder: Path) -> dict:
    """Saves one operation's JPEG, recording missing or expired images from older rows."""
    try:
        with host.open(f"/v1/operations/{urllib.parse.quote(operation_id)}/screenshot") as reply:
            (folder / f"{operation_id}.jpg").write_bytes(reply.read())
        return {"status": "saved"}
    except urllib.error.HTTPError as error:
        if error.code not in (404, 410):
            raise
        body = json.loads(error.read() or b"{}")
        return {"status": body.get("code", "not_found" if error.code == 404 else "expired"), "message": body.get("message")}


def sse(host: Host, last_event_id: str) -> Generator[tuple[str, str, dict], None, None]:
    """Yields (id, event type, data) from /v1/events, resuming after `last_event_id`.

    SSE makes `id:` optional, but Neo puts `<boot>-<seq>` on every event and
    the pull orders and stops by it, so an event without one is refused here
    rather than checked by every reader.
    """
    reply = host.open("/v1/events", {"Last-Event-ID": last_event_id}, timeout=EVENT_READ_TIMEOUT_S)
    with reply:
        event_id, kind, data = None, "message", []
        for raw in reply:
            line = raw.decode().rstrip("\r\n")
            if not line:
                if data:
                    if event_id is None:
                        raise RuntimeError(f"/v1/events sent a {kind} event without an id")
                    yield event_id, kind, json.loads("\n".join(data))
                event_id, kind, data = None, "message", []
            elif line.startswith(":"):  # keep-alive comment
                continue
            else:
                field, _, value = line.partition(":")
                value = value.removeprefix(" ")
                if field == "id":
                    event_id = value
                elif field == "event":
                    kind = value
                elif field == "data":
                    data.append(value)


def pull_events(host: Host) -> dict:
    """The events Neo still holds, oldest first.

    /v1/events only streams from a point on. An id Neo never sent makes it
    answer `stream.reset` first, whose id is `<boot>-<newest seq>`; resuming
    from `<boot>-<newest - 1024>` then replays the whole ring, and the pull
    stops at the newest seq it saw at the start. Boot ids hold no `-`, so the
    seq is after the last one.
    """
    probe = sse(host, "export")
    reset_id, kind, _ = next(probe)
    probe.close()
    if kind != "stream.reset":
        raise RuntimeError(f"/v1/events began with {kind}, not stream.reset")
    boot, _, seq = reset_id.rpartition("-")
    newest = int(seq)
    if newest == 0:
        return {"boot": boot, "newest": 0, "complete": True, "events": []}
    events: list[dict] = []
    complete = False
    deadline = time.monotonic() + EVENT_DEADLINE_S
    try:
        for event_id, kind, event in sse(host, f"{boot}-{max(0, newest - EVENT_RING)}"):
            if kind == "stream.reset":  # the ring moved on between the two connections
                break
            events.append({"id": event_id, "event": event})
            if int(event_id.rpartition("-")[2]) >= newest:
                complete = True
                break
            if time.monotonic() > deadline:
                break
    except TimeoutError:
        pass
    return {"boot": boot, "newest": newest, "complete": complete, "events": events}


def export(host: Host, traces: Path) -> Path:
    out = traces / f"export-{time.strftime('%Y%m%d-%H%M%S')}"
    (out / "runs").mkdir(parents=True)
    (out / "screenshots").mkdir()

    runs = list_runs(host)
    (out / "runs.json").write_text(json.dumps(runs, indent=2))
    shots = {}
    unfinished = []
    for summary in runs:
        # A run's reply can come before its screenshots are taken; `wait` holds the read until
        # the run is complete (at most 10 s), so a fresh run is not exported without them.
        wait = "" if summary.get("complete") else "?wait=10"
        run = host.json(f"/v1/runs/{urllib.parse.quote(summary['id'])}{wait}")
        if not run.get("complete"):
            unfinished.append(run["id"])
        (out / "runs" / f"{run['id']}.json").write_text(json.dumps(run, indent=2))
        for operation in run.get("operations", []):
            if operation.get("hasScreenshot"):
                shots[operation["id"]] = save_screenshot(host, operation["id"], out / "screenshots")
    (out / "screenshots" / "index.json").write_text(json.dumps(shots, indent=2))

    events = pull_events(host)
    (out / "events.json").write_text(json.dumps(events, indent=2))

    cli_log = traces / "runs.jsonl"
    if cli_log.exists():
        shutil.copyfile(cli_log, out / "cli-runs.jsonl")

    saved = sum(1 for shot in shots.values() if shot["status"] == "saved")
    print(
        f"{len(runs)} runs, {saved} screenshots ({len(shots) - saved} not saved), "
        f"{len(events['events'])} events{'' if events['complete'] else ' (incomplete)'}"
    )
    if unfinished:
        print(f"{len(unfinished)} runs were still running or taking screenshots: {', '.join(unfinished)}")
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description="Export a claw-sdk Neo's runs, screenshots and events.")
    parser.add_argument("--host-url", required=True, help="host API origin, e.g. http://127.0.0.1:19300")
    parser.add_argument("--token-file", required=True, type=Path)
    parser.add_argument("--traces-dir", required=True, type=Path)
    args = parser.parse_args()
    host = Host(args.host_url, args.token_file.read_text().strip())
    try:
        print(export(host, args.traces_dir))
    except urllib.error.HTTPError as error:
        sys.exit(f"host API answered HTTP {error.code} for {error.url}: {error.read().decode(errors='replace')[:300]}")
    except urllib.error.URLError as error:
        sys.exit(f"can't reach the host API at {args.host_url} ({error.reason}); is `dev.sh up --server claw-sdk` running?")


if __name__ == "__main__":
    main()
