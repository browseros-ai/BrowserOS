#!/usr/bin/env python3
"""Prove zuzu's S20 audit contract inside a local SDK image as the host uid.

Run beside accept_client.py and smoke.py with Python's standard library. The caller owns the
container, its memory cap, isolated network, host-token mount, and cleanup. This probe drives
real fills through MCP and compares one paged summary response with the stored full records.
"""

import http.server
import json
import threading
import uuid

from accept_client import call_run, connect, host_raw


SOCKET = "/run/neo/host.sock"
TOKEN = "/run/neo-recipes/host-token"
CANARIES = ["s20-secret-7731", "s20-code-5521", "s20-result-8842", "s20-error-3390"]


class Form(http.server.BaseHTTPRequestHandler):
    """Local fixture with stable audit metadata and no external network dependency."""

    def do_GET(self):
        body = b'<title>S20 form</title><input type=password aria-label=Password><h1>Audit</h1>'
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


def get(path):
    status, headers, raw = host_raw(SOCKET, TOKEN, path)
    assert status == 200, (path, status, raw[:300])
    return json.loads(raw), raw


def check_fields(value):
    if isinstance(value, dict):
        forbidden = {"codeWithheld", "result", "resultTruncated", "args", "argsTruncated",
                     "notes", "hostData", "downloads", "unknown", "site", "delivery", "screenshot"}
        assert not forbidden.intersection(value), forbidden.intersection(value)
        if "message" in value:
            assert value["message"] == "", value
        for item in value.values():
            check_fields(item)
    elif isinstance(value, list):
        for item in value:
            check_fields(item)


def probe(origin):
    task = "s20-audit-" + uuid.uuid4().hex[:10]
    other_task = task + "-other"
    client = connect("http://127.0.0.1:9010/mcp")
    runs = []

    def run(task_id, code, error=False):
        reply = call_run(client, code, {"taskId": task_id, "hostData": {"probe": CANARIES[0]}})
        assert reply["isError"] == error, reply
        run_id = reply["meta"]["id"]
        detail, raw = get(f"/v1/runs/{run_id}?wait=60")
        assert detail["complete"], detail
        runs.append((task_id, detail, raw))
        return detail

    run(task, f"""
// {CANARIES[1]}
await page.goto('{origin}/form');
await page.getByLabel('Password').fill('{CANARIES[0]}');
const second = await context.newPage();
await second.goto('{origin}/second');
await second.title();
return '{CANARIES[2]}';
""")
    run(task, f"throw new Error('{CANARIES[3]}');", error=True)
    other = run(other_task, f"const third = await context.newPage(); await third.goto('{origin}/other');")

    summary, raw = get(f"/v1/runs?taskId={task}&limit=20&fields=summary&internal=false")
    for canary in CANARIES:
        assert canary.encode() not in raw, canary
    check_fields(summary)
    expected = {detail["id"]: detail for owner, detail, _ in runs if owner == task}
    assert [item["id"] for item in summary["items"]] == list(reversed(expected)), summary
    assert summary.get("nextCursor") is None, summary
    other_tabs = set(other["tabs"])
    operations = []
    for item in summary["items"]:
        full = expected[item["id"]]
        for key in ["id", "status", "startedAt", "endedAt", "complete"]:
            assert item.get(key) == full.get(key), (key, item, full)
        assert not other_tabs.intersection(item["tabs"]), (other_tabs, item)
        full_ops = {op["id"]: op for op in full["operations"] if not op["internal"]}
        assert [op["id"] for op in item["operations"]] == list(full_ops), item
        for op in item["operations"]:
            assert not op["name"].startswith("events."), op
            baseline = full_ops[op["id"]]
            for key in ["id", "name", "tabId", "startedAt", "durationMs", "outcome", "hasScreenshot", "parentId"]:
                assert op.get(key) == baseline.get(key), (key, op, baseline)
            for key in ["url", "title"]:
                assert op.get("page", {}).get(key) == baseline.get("page", {}).get(key), (key, op, baseline)
            assert op.get("tabId") not in other_tabs, op
            operations.append(op)

    # Comparison calls are proof only. The consumer gets every run and step in the one call above.
    full_bytes = b"".join(raw for owner, _, raw in runs if owner == task)
    for canary in CANARIES:
        assert canary.encode() in full_bytes, f"fixture failed to store {canary}"
    for owner, detail, before in runs:
        _, after = get(f'/v1/runs/{detail["id"]}')
        assert after == before, "summary changed a full read"
    shot = next(op for op in operations if op["hasScreenshot"])
    status, headers, jpeg = host_raw(SOCKET, TOKEN, f'/v1/operations/{shot["id"]}/screenshot')
    assert status == 200 and headers["content-type"] == "image/jpeg" and jpeg.startswith(b"\xff\xd8"), (status, headers)
    page, _ = get(f"/v1/runs?taskId={task}&limit=1&fields=summary&internal=false")
    assert len(page["items"]) == 1 and page.get("nextCursor"), page
    older, _ = get(f'/v1/runs?taskId={task}&limit=1&fields=summary&internal=false&cursor={page["nextCursor"]}')
    assert older["items"][0]["id"] == next(iter(expected)) and not older.get("nextCursor"), older
    listener_run = run(task + "-listener", f"""
await page.goto('{origin}/listeners');
page.on('dialog', () => {{}});
await page.title();
""")
    listeners = [op for op in listener_run["operations"] if op["name"] == "events.listen" and op.get("site")]
    assert listeners and all(not op["internal"] for op in listeners), listener_run
    filtered, _ = get(f'/v1/runs/{listener_run["id"]}?fields=summary&internal=false')
    assert {op["id"] for op in listeners}.issubset({op["id"] for op in filtered["operations"]}), filtered
    assert all(not op["internal"] for op in filtered["operations"]), filtered
    return {"verdict": "PASS", "task": task, "runs": len(expected), "operations": len(operations),
            "canaries_absent": len(CANARIES), "screenshot_bytes": len(jpeg),
            "summary_requests_per_audit": 1, "pagination": "nextCursor", "full_reads_unchanged": True,
            "script_listener_preserved": True}


if __name__ == "__main__":
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Form)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        print(json.dumps(probe(f"http://127.0.0.1:{server.server_port}"), indent=2))
    finally:
        server.shutdown()
        server.server_close()
