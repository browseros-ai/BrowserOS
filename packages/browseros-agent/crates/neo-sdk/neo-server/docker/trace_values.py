#!/usr/bin/env python3
"""Image acceptance for trace.values, run as root in accept.sh's own container.

The on control and off run use different canaries, so the raw-file scan can inspect the
shared acceptance database without mistaking another run's retained values for a leak.
An external SQLite write lock creates the real reply-before-end-record crash interval.
Expiry is accelerated only by changing this probe's existing sidecars before a restart.
"""

import http.client
import http.server
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import subprocess
import sys
import threading
import time
import urllib.parse
import uuid

from accept_client import FilesPage, STATEMENT, call_run, connect, host, host_raw


KINDS = (
    "script", "argument", "operation-result", "result", "note", "error",
    "run-error", "console", "host-data", "blob",
)
TITLE = "Trace values acceptance"
BLOB_SIZE = 300 * 1024
TRACE_DIR = Path("/data/browserclaw/claw-sdk")


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class Events:
    """Read live SSE before admitting either run. HTTPResponse handles chunk framing while
    one reader thread collects complete JSON events, including their hydrated records."""

    def __init__(self, socket_path, token_file, task):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(120)
        self.socket.connect(socket_path)
        token = Path(token_file).read_text().strip()
        path = "/v1/events?" + urllib.parse.urlencode({"taskId": task})
        self.socket.sendall(
            (
                f"GET {path} HTTP/1.1\r\nHost: neo\r\nAuthorization: Bearer {token}\r\n"
                "Accept: text/event-stream\r\nConnection: close\r\n\r\n"
            ).encode()
        )
        self.response = http.client.HTTPResponse(self.socket)
        self.response.begin()
        require(self.response.status == 200, f"events answered {self.response.status}")
        self.condition = threading.Condition()
        self.events = []
        self.error = None
        self.stopped = False
        self.thread = threading.Thread(target=self.read, daemon=True)
        self.thread.start()

    def read(self):
        data = []
        try:
            while line := self.response.readline():
                line = line.decode().rstrip("\r\n")
                if line.startswith("data:"):
                    data.append(line[5:].strip())
                elif not line and data:
                    event = json.loads("\n".join(data))
                    data = []
                    with self.condition:
                        self.events.append(event)
                        self.condition.notify_all()
        except (OSError, ValueError) as error:
            if not self.stopped:
                self.error = str(error)
        finally:
            with self.condition:
                self.condition.notify_all()

    def wait(self, run_ids):
        deadline = time.monotonic() + 30
        with self.condition:
            while True:
                ended = {
                    event.get("run", {}).get("id") for event in self.events
                    if event.get("type") == "run.ended"
                }
                if set(run_ids) <= ended:
                    return list(self.events)
                require(self.error is None, f"event stream failed: {self.error}")
                remaining = deadline - time.monotonic()
                require(remaining > 0, "event stream missed a canary run's end")
                self.condition.wait(remaining)

    def close(self):
        self.stopped = True
        try:
            self.socket.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.socket.close()
        self.thread.join(timeout=2)
        self.response.close()


class CountingFilesPage(FilesPage):
    """The external download counter proves request replay never executes the script again,
    including after the Neo server restarts and loses its in-memory reply."""

    def do_GET(self):
        if self.path == "/statement":
            self.server.downloads += 1
        super().do_GET()


def script(markers, origin):
    return f"""// {markers['script']}
const markers = {json.dumps(markers)};
await page.goto({json.dumps(origin + '/')});
const [download] = await Promise.all([
  page.waitForEvent('download'), page.locator('#get').click()
]);
await download.path();
await page.setContent(`<title>{TITLE}</title><input id="${{markers.note}}">
  <div id="read">${{markers['operation-result']}}</div>
  <div id="${{markers.error}}">not an editable element</div>`);
await page.locator('#' + markers.note).fill(markers.argument);
const read = await page.locator('#read').textContent();
console.log(markers.console);
let failure;
try {{
  await page.locator('#' + markers.error).fill(markers.argument, {{ timeout: 1500 }});
}} catch (error) {{
  failure = {{ owner: error.owner, code: error.code, message: error.message }};
}}
await page.title();
const bytes = new Uint8Array({BLOB_SIZE});
bytes.set(Buffer.from(markers.blob));
return {{ result: markers.result, read, failure, bytes }};
"""


def api(socket_path, token_file, path, method="GET", body=None):
    reply = host(socket_path, token_file, path, method, "" if body is None else json.dumps(body))
    require(reply.get("status") == 200, f"{method} {path} answered {reply.get('status', reply.get('connect'))}")
    return reply["body"]


def binary(socket_path, token_file, path, expected_status=200):
    reply = host_raw(socket_path, token_file, path)
    require(isinstance(reply, tuple), f"could not connect for {path}")
    status, _, body = reply
    require(status == expected_status, f"{path} answered {status}, expected {expected_status}")
    return body


def run_case(client, socket_path, token_file, tag, mode, origin, server):
    markers = {kind: f"{tag}-{mode}-{kind}" for kind in KINDS}
    task, request = f"{tag}-{mode}", f"{tag}-{mode}-main"
    meta = {"taskId": task, "requestId": request, "hostData": {"canary": markers["host-data"]}}
    code = script(markers, origin)
    before = server.downloads
    reply = call_run(client, code, meta)
    require(reply["meta"].get("status") == "ok", f"{mode} canary script failed")
    envelope = reply.get("value") or {}
    value = envelope.get("value") or {}
    require(value.get("result") == markers["result"], f"{mode} reply lost its result")
    require(value.get("read") == markers["operation-result"], f"{mode} reply lost its operation result")
    failure = value.get("failure") or {}
    require(markers["error"] in failure.get("message", ""), f"{mode} found-element fill did not produce the error canary")
    require(any(markers["console"] in line.get("text", "") for line in envelope.get("logs", [])), f"{mode} reply lost its console line")
    marker = value.get("bytes") or {}
    require(marker.get("$neo") == "blob" and marker.get("size") == BLOB_SIZE, f"{mode} Uint8Array was not externalized as a blob")
    run_id = reply["meta"].get("id")
    require(run_id, f"{mode} reply has no run id")
    blob_path = f"/v1/runs/{run_id}/blobs/{marker['id']}"
    blob = binary(socket_path, token_file, blob_path)
    require(len(blob) == BLOB_SIZE and blob.startswith(markers["blob"].encode()), f"{mode} blob bytes differ from the reply")
    repeated = call_run(client, code, meta)
    require(repeated["meta"] == reply["meta"] and repeated["value"] == reply["value"], f"{mode} immediate repeat changed its reply")
    require(server.downloads == before + 1, f"{mode} repeat executed the download again")

    failed_meta = {**meta, "requestId": f"{tag}-{mode}-error"}
    failed = call_run(client, f"throw new Error({json.dumps(markers['run-error'])});", failed_meta)
    require(failed["meta"].get("status") == "error", f"{mode} throwing script did not fail")
    failed_id = failed["meta"].get("id")
    require(failed_id, f"{mode} failed reply has no run id")
    record = api(socket_path, token_file, f"/v1/runs/{run_id}?wait=10")
    failed_record = api(socket_path, token_file, f"/v1/runs/{failed_id}?wait=10")
    return {
        "markers": markers, "task": task, "request": request, "meta": meta, "code": code,
        "reply": reply, "record": record, "failed_reply": failed, "failed_record": failed_record,
        "run": run_id, "failed_run": failed_id, "blob": blob_path,
    }


def check_metadata(case, origin):
    record = case["record"]
    require(record.get("requestId") == case["request"] and record.get("taskId") == case["task"], "off record lost host identity")
    require(record.get("status") == "ok" and record.get("complete"), "off record lost its outcome or completion")
    require(record.get("tabs") and record.get("operationCount", 0) > 0, "off record lost tabs or operation count")
    require(isinstance(record.get("startedAt"), int) and isinstance(record.get("endedAt"), int), "off record lost its times")
    operations = record.get("operations") or []
    require(any(op.get("site") for op in operations), "off operations lost script call sites")
    require(any(op.get("tabId") in record["tabs"] for op in operations), "off operations lost their target tabs")
    require(any((op.get("page") or {}).get("url", "").startswith(origin) for op in operations), "off operations lost the page address")
    require(any((op.get("page") or {}).get("title") == TITLE for op in operations), "off operations lost the page title")
    for op in operations:
        for key in ("id", "runId", "name", "kind", "effect", "startedAt", "endedAt", "outcome", "delivery"):
            require(key in op, f"off operation lost {key}")
        if op.get("error"):
            require(op["error"].get("owner") and op["error"].get("code"), "off operation lost error ownership or code")


def check_stripped(record, label, marker=True):
    if marker:
        require(record.get("values") == "off", f"{label} has no values=off marker")
    for key in ("code", "args", "result", "notes", "hostData"):
        require(key not in record, f"{label} exposes {key}")
    if record.get("error"):
        require(record["error"].get("message") == "", f"{label} exposes an error message")


def check_reads(socket_path, token_file, case, events):
    records = [case["record"], case["failed_record"]]
    for record in records:
        check_stripped(record, "run detail")
        for op in record.get("operations", []):
            check_stripped(op, "run operation")
    query = urllib.parse.urlencode({"taskId": case["task"], "limit": 100})
    listed = api(socket_path, token_file, "/v1/runs?" + query)
    require({row["id"] for row in listed["items"]} == {case["run"], case["failed_run"]}, "off listing lost or duplicated a run")
    for record in listed["items"]:
        check_stripped(record, "run list")
    query = urllib.parse.urlencode({"taskId": case["task"], "tail": 1000})
    tail = api(socket_path, token_file, "/v1/trace?" + query)
    require(tail.get("operations"), "off trace tail lost its operations")
    for record in tail["runs"] + tail["operations"]:
        check_stripped(record, "trace tail")
    ended_ops = [event["operation"] for event in events if event.get("type") == "operation.ended"]
    require(ended_ops, "off event stream had no operation.ended records")
    for op in ended_ops:
        check_stripped(op, "operation.ended event")
    for event in events:
        if event.get("type") == "run.ended":
            check_stripped(event["run"], "run.ended event")
    encoded = json.dumps([records, listed, tail, events]).encode()
    for kind, canary in case["markers"].items():
        require(canary.encode() not in encoded, f"off HTTP or events exposed the {kind} canary")


def scan(paths, markers):
    hits = {kind: [] for kind in markers}
    for path in paths:
        contents = path.read_bytes()
        for kind, canary in markers.items():
            if canary.encode() in contents:
                hits[kind].append(str(path))
    return hits


def database_files():
    database = TRACE_DIR / "trace.db"
    require(database.is_file(), "trace.db is missing, so the raw scan cannot prove absence")
    wal = TRACE_DIR / "trace.db-wal"
    return [database] + ([wal] if wal.exists() else [])


def check_control(case):
    record, markers = case["record"], case["markers"]
    operations = record.get("operations") or []
    require("values" not in record and all("values" not in op for op in operations), "default on records gained an omission marker")
    require(markers["script"] in record.get("code", ""), "on control has no stored script canary")
    require(markers["host-data"] in json.dumps(record.get("hostData")), "on control has no stored host data canary")
    require(markers["result"] in json.dumps(record.get("result")), "on control has no stored run result canary")
    require(any(markers["argument"] in json.dumps(op.get("args")) for op in operations), "on control has no stored argument canary")
    require(any(op.get("result") == markers["operation-result"] for op in operations), "on control has no stored operation result canary")
    require(all(not op.get("notes") for op in operations), "on must not begin storing call-log notes")
    logs = TRACE_DIR.parent / "logs"
    logged = b"".join(path.read_bytes() for path in logs.rglob("*") if path.is_file())
    for kind in ("note", "argument"):
        require(markers[kind].encode() in logged, f"on control has no debug call-log {kind} canary")
    require(any(markers["error"] in (op.get("error") or {}).get("message", "") for op in operations), "on control has no stored operation error canary")
    require(markers["run-error"] in (case["failed_record"].get("error") or {}).get("message", ""), "on control has no stored run error canary")
    database = sqlite3.connect(f"file:{TRACE_DIR / 'trace.db'}?mode=ro", uri=True)
    try:
        row = database.execute("SELECT replay FROM runs WHERE id = ?", (case["run"],)).fetchone()
        require(row is not None and row[0] is not None, "on control has no persisted replay")
        stored = json.loads(row[0])
        require(any(markers["console"] in line.get("text", "") for line in stored.get("logs", [])), "on replay has no stored console canary")
    finally:
        database.close()
    found = scan(database_files(), markers)
    for kind in KINDS:
        if kind != "blob":
            require(found[kind], f"raw database scanner did not detect the on {kind} canary")
    blobs = list((TRACE_DIR / "outputs" / case["run"] / "blobs").glob("*"))
    require(scan(blobs, {"blob": markers["blob"]})["blob"], "raw output scanner did not detect the on blob canary")


def server_identity():
    server = subprocess.check_output(["neo-server-info", "path"], text=True).strip().encode()
    matches = []
    for path in Path("/proc").glob("[0-9]*/cmdline"):
        try:
            if path.read_bytes().split(b"\0", 1)[0] == server:
                pid = int(path.parent.name)
                started = (path.parent / "stat").read_text().rsplit(")", 1)[1].split()[19]
                matches.append((pid, started))
        except (OSError, ValueError, IndexError):
            continue
    require(len(matches) == 1, f"expected exactly one own-container Neo server, found {len(matches)}")
    return matches[0]


def restart_server(socket_path, token_file, expected_mode="off", after_kill=None):
    require(Path("/.dockerenv").exists(), "server restart is restricted to the acceptance container")
    old = server_identity()
    # Recheck both argv and process start time immediately before acting. A recycled PID or
    # a server that already exited is a failed precondition, never a reason to kill another PID.
    require(server_identity() == old, "the recorded server changed before SIGKILL")
    os.kill(old[0], signal.SIGKILL)
    if after_kill is not None:
        after_kill()
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        try:
            new = server_identity()
            if new != old:
                settings = api(socket_path, token_file, "/v1/settings")
                require(settings["settings"]["trace"]["values"] == expected_mode, "trace.values did not survive the server restart")
                return [old[0], new[0]]
        except (AssertionError, OSError, ValueError):
            pass
        time.sleep(0.25)
    raise AssertionError(f"server {old[0]} did not relaunch with the saved {expected_mode} setting")


def crash_before_end(url, socket_path, token_file, tag, mode):
    """Keep SQLite's writer blocked from the committed start through reply delivery, then
    kill the real server while the lock is held. The caller's transaction never edits a row."""
    task = f"{tag}-crash-{mode}"
    request = f"{task}-main"
    canary = f"{task}-blob"
    code = f"""await new Promise(resolve => setTimeout(resolve, 5000));
const bytes = new Uint8Array({BLOB_SIZE});
bytes.set(Buffer.from({json.dumps(canary)}));
return bytes;
"""
    client = connect(url)
    finished = threading.Event()
    outcome = {}

    def run():
        try:
            outcome["reply"] = call_run(client, code, {"taskId": task, "requestId": request})
        except Exception as error:
            outcome["error"] = error
        finally:
            finished.set()

    database = sqlite3.connect(
        f"file:{TRACE_DIR / 'trace.db'}?mode=rw", uri=True,
        timeout=0.25, isolation_level=None,
    )
    thread = threading.Thread(target=run, daemon=True)
    try:
        thread.start()
        deadline = time.monotonic() + 15
        select = 'SELECT id, status, ended_at, "values" FROM runs WHERE request_id = ? AND task_id = ?'
        row = None
        while time.monotonic() < deadline:
            rows = database.execute(select, (request, task)).fetchall()
            require(len(rows) <= 1, f"{mode} crash request has duplicate start rows")
            if rows:
                row = rows[0]
                break
            require(not finished.is_set(), f"{mode} crash call finished before its start row was observed")
            time.sleep(0.02)
        require(row is not None, f"{mode} crash run's start did not commit")
        require(row[1:] == ("running", None, None), f"{mode} crash run already ended before the write lock")
        require(not finished.is_set(), f"{mode} crash call finished before its write lock")

        # WAL readers remain usable under this reserved write lock. The run can return its
        # bytes while the trace writer cannot commit the end that carries its recording marker.
        database.execute("BEGIN IMMEDIATE")
        require(not finished.is_set(), f"{mode} crash call finished while acquiring the write lock")
        require(database.execute(select, (request, task)).fetchone() == row, f"{mode} start changed before the write lock")
        require(finished.wait(20), f"{mode} crash run did not deliver its reply under the write lock")
        if "error" in outcome:
            raise outcome["error"]
        reply = outcome["reply"]
        require(reply["meta"].get("status") == "ok", f"{mode} crash run did not return successfully")
        run_id = reply["meta"].get("id")
        require(run_id == row[0], f"{mode} crash reply names another run")
        marker = (reply.get("value") or {}).get("value") or {}
        require(marker.get("$neo") == "blob" and marker.get("size") == BLOB_SIZE, f"{mode} crash run did not externalize its Uint8Array")
        blob_path = f"/v1/runs/{run_id}/blobs/{marker['id']}"
        body = binary(socket_path, token_file, blob_path)
        require(len(body) == BLOB_SIZE and body.startswith(canary.encode()), f"{mode} crash blob was not delivered before the trace end")
        require(database.execute(select, (request, task)).fetchone() == row, f"{mode} crash end reached SQLite before SIGKILL")
        require(database.in_transaction, f"{mode} crash test lost its external write lock")
        expiry = TRACE_DIR / "outputs" / run_id / "blob-expiry"
        until = expiry.read_text() if mode == "off" else None
        if mode == "off":
            require(int(until.strip()) > int(time.time() * 1000), "off crash blob has no future durable deadline")
        else:
            require(not expiry.exists(), "on crash blob unexpectedly has an expiry sidecar")

        # SIGKILL happens before the callback releases the external transaction. Startup
        # recovery can acquire SQLite immediately afterwards; no product fault hook is needed.
        pids = restart_server(socket_path, token_file, expected_mode=mode, after_kill=database.rollback)
        recovered = api(socket_path, token_file, f"/v1/runs/{run_id}")
        require(recovered.get("status") == "interrupted", f"{mode} crash run did not recover as interrupted")
        require("values" not in recovered, f"{mode} interrupted row gained an end-time values marker")
        require("endedAt" not in recovered, f"{mode} interrupted row unexpectedly has a stored end time")
        check_stripped(recovered, f"{mode} interrupted run", marker=False)
        body = binary(socket_path, token_file, blob_path)
        require(len(body) == BLOB_SIZE and body.startswith(canary.encode()), f"{mode} interrupted run lost its unexpired blob")
        if mode == "off":
            require(expiry.read_text() == until, "off crash blob's deadline did not survive restart")
        else:
            require(not expiry.exists(), "on interrupted blob gained an expiry sidecar")
        return {"run": run_id, "blob": blob_path, "canary": canary, "expiry": expiry, "pids": pids}
    finally:
        if database.in_transaction:
            database.rollback()
        database.close()
        thread.join(timeout=1)


def probe(url, socket_path, token_file):
    result = {"ok": False, "problems": [], "serverPids": []}
    saved_trace = None
    events = None
    server = None
    try:
        settings = api(socket_path, token_file, "/v1/settings")
        saved_trace = settings["settings"]["trace"]
        require(saved_trace.get("values") == "on", "acceptance must start with the default on recording policy")
        api(socket_path, token_file, "/v1/settings", "PATCH", {"trace": {"screenshots": "off"}})
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), CountingFilesPage)
        server.downloads = 0
        threading.Thread(target=server.serve_forever, daemon=True).start()
        origin = f"http://127.0.0.1:{server.server_address[1]}"
        tag = "trace-values-" + uuid.uuid4().hex
        client = connect(url)
        on = run_case(client, socket_path, token_file, tag, "on", origin, server)
        check_control(on)
        on_crash = crash_before_end(url, socket_path, token_file, tag, "on")
        result["serverPids"].append(on_crash["pids"])
        client = connect(url)
        api(socket_path, token_file, "/v1/settings", "PATCH", {"trace": {"values": "off"}})
        events = Events(socket_path, token_file, f"{tag}-off")
        before = int(time.time() * 1000)
        off = run_case(client, socket_path, token_file, tag, "off", origin, server)
        after = int(time.time() * 1000)
        captured = events.wait([off["run"], off["failed_run"]])
        events.close()
        events = None
        check_metadata(off, origin)
        check_reads(socket_path, token_file, off, captured)
        error = off["failed_record"].get("error") or {}
        expected = (off["failed_reply"].get("value") or {}).get("error") or {}
        for key in ("owner", "code", "retryable"):
            require(key in error and error[key] == expected.get(key), f"off run error lost {key}")
        for kind, paths in scan(database_files(), off["markers"]).items():
            require(not paths, f"off {kind} canary reached the database or WAL")

        expiry = TRACE_DIR / "outputs" / off["run"] / "blob-expiry"
        until = int(expiry.read_text().strip())
        require(before + 600_000 <= until <= after + 600_000, "off blob sidecar does not grant its ten-minute window")
        require(not (TRACE_DIR / "outputs" / on["run"] / "blob-expiry").exists(), "an on blob unexpectedly gained an expiry")
        off_crash = crash_before_end(url, socket_path, token_file, tag, "off")
        result["serverPids"].append(off_crash["pids"])
        require(binary(socket_path, token_file, off["blob"]).startswith(off["markers"]["blob"].encode()), "unexpired off blob did not survive restart")
        count = server.downloads
        duplicate = call_run(connect(url), off["code"], off["meta"])
        require(((duplicate.get("value") or {}).get("error") or {}).get("code") == "duplicate_request", "off request replayed a persisted value after restart")
        require(server.downloads == count, "off request ran again after restart")

        # Only this probe's existing deadlines change. The interrupted run has no values
        # marker or end time in SQLite, so its cleanup must follow the durable sidecar alone.
        expiry.write_text("0")
        off_crash["expiry"].write_text("0")
        result["serverPids"].append(restart_server(socket_path, token_file))
        binary(socket_path, token_file, off["blob"], 404)
        binary(socket_path, token_file, off_crash["blob"], 404)
        require(not expiry.exists() and not expiry.with_name("blobs").exists(), "startup did not remove expired blobs and their sidecar")
        require(not off_crash["expiry"].exists() and not off_crash["expiry"].with_name("blobs").exists(), "startup did not remove the interrupted off run's blobs and sidecar")
        require(binary(socket_path, token_file, on["blob"]).startswith(on["markers"]["blob"].encode()), "expiry cleanup removed the on control's blob")
        require(binary(socket_path, token_file, on_crash["blob"]).startswith(on_crash["canary"].encode()), "off expiry cleanup removed the interrupted on run's blob")
        require(not on_crash["expiry"].exists(), "interrupted on blob gained an expiry during cleanup")
        require(binary(socket_path, token_file, f"/v1/runs/{off['run']}/downloads/0") == STATEMENT, "expiry cleanup removed or changed the download")
        recorded = api(socket_path, token_file, f"/v1/runs/{off['run']}")
        require(recorded.get("downloads") and recorded["downloads"][0].get("state") == "completed", "off download disappeared from the run record")
        output_files = [path for path in (TRACE_DIR / "outputs").rglob("*") if path.is_file()]
        logs = TRACE_DIR.parent / "logs"
        require(logs.is_dir(), "server log folder is missing, so its raw scan cannot prove absence")
        log_files = [path for path in logs.rglob("*") if path.is_file()]
        require(log_files, "no server log file was available for the canary scan")
        off_markers = {**off["markers"], "crash-blob": off_crash["canary"]}
        for kind, paths in scan(database_files() + output_files + log_files, off_markers).items():
            require(not paths, f"off {kind} canary remains in raw database, WAL, outputs or server logs")
        result.update({
            "runs": {"on": on["run"], "off": off["run"], "crashOn": on_crash["run"], "crashOff": off_crash["run"]},
            "detail": "on control detected; off HTTP/events/raw files omit values; real crashes before trace end retain unexpired blobs; off startup expiry removes normal/interrupted blobs and preserves on blobs and downloads",
            "acceleratedExpiry": True,
        })
    except Exception as error:
        result["problems"].append(f"{type(error).__name__}: {error}")
    finally:
        if events is not None:
            events.close()
        if server is not None:
            server.shutdown()
            server.server_close()
        if saved_trace is not None:
            try:
                api(socket_path, token_file, "/v1/settings", "PATCH", {"trace": saved_trace})
                result["settingsRestored"] = True
            except Exception as error:
                result["problems"].append(f"could not restore trace settings: {error}")
        result["ok"] = not result["problems"]
    return result


if __name__ == "__main__":
    print(json.dumps(probe(*sys.argv[1:])))
