#!/usr/bin/env python3
"""Whether every page a script reaches lists its whole frame tree, on the image: accept.sh's
`frames` check.

The rule: in every run, `page.frames()` of every page the script can reach (its built-in `page`
and every page of `context.pages()`) is the page's real frame tree, cross-site children
included, from the script's first statement on, without the script touching the page first. A
frame added later shows once the script has read the page's frames and then waited.

  accept_frames.py <mcp-url> [reps]

The page is served here on 127.0.0.1, with a same-origin child and a child from `localhost`: a
different site, so Chromium puts it in its own process, as a payment or a sign-in frame is. Each
shape runs `reps` times (default 10), each repetition in fresh tasks:

  observer       a run without a task reads a task's tab (the first host's read of the agent's
                 tab: under the default task scope a host's own reads send no task, since a run
                 that carries one sees only its task family's tabs), with no step before the
                 walk, and after a title
  own-later      a later run of the task that opened the tab takes it from context.pages()
  added          a cross-site frame the page adds 1.5 s after it loads, seen by a run that read
                 the tab's frames and then waited

Prints one JSON object: ok, how many repetitions of each shape went wrong, the first problems.
Standard library only.
"""

import http.server
import json
import sys
import threading
import uuid

from accept_client import RUN_META, connect

PAGES = {
    "/outer": "<title>outer</title><p>outer</p>"
    "<iframe id=cross src='http://localhost:{port}/inner'></iframe>"
    "<iframe id=same src='/same'></iframe>",
    "/inner": "<title>inner</title><input aria-label='Digit 1'>",
    "/same": "<title>same</title><p>same-origin child</p>",
    "/late": "<title>late</title><p>late</p><script>setTimeout(() => {{"
    " const f = document.createElement('iframe'); f.src = 'http://localhost:{port}/inner';"
    " document.body.append(f) }}, 1500)</script>",
}

WALK = """const urls = p.frames().map((f) => f.url());
return {{ same: p === page, cross: urls.some((u) => u.startsWith('http://localhost:{port}/')),
  sameSite: urls.some((u) => u.endsWith('/same')), frames: urls.length }};"""


class Site(http.server.BaseHTTPRequestHandler):
    """The page and its children, from one server: `localhost` names another site than 127.0.0.1."""

    def do_GET(self):
        page = PAGES.get(self.path.split("?")[0])
        body = (page or "").format(port=self.server.server_port).encode()  # type: ignore[attr-defined]
        self.send_response(200 if page else 404)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):  # noqa: A002 (the base class's name)
        del format, args


class Runs:
    """`run` calls over one MCP session, each naming its task in `_meta` as a host does, or none
    for a host's own read."""

    def __init__(self, mcp_url: str):
        self.client = connect(mcp_url)

    def run(self, task: str | None, code: str):
        result = self.client.call(
            "tools/call",
            {
                "name": "run",
                "arguments": {"code": code, "timeout": 60_000},
                "_meta": {RUN_META: {"taskId": task} if task is not None else {}},
            },
        )
        structured = result.get("structuredContent") or {}
        return structured.get("value"), structured.get("error")


def fresh(shape: str) -> str:
    return f"accept-frames-{shape}-{uuid.uuid4().hex[:6]}"


def lookup(tab) -> str:
    return f"const p = context.pages().find((q) => q.tabId === {json.dumps(tab)});\n"


def main() -> None:
    mcp_url = sys.argv[1]
    reps = int(sys.argv[2]) if len(sys.argv) > 2 else 10
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Site)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]
    origin = f"http://127.0.0.1:{port}"
    walk = WALK.format(port=port)
    runs = Runs(mcp_url)
    wrong = {"observer": 0, "observer-after-title": 0, "own-later": 0, "added": 0}
    problems: list[str] = []

    def check(shape: str, value, error, want) -> None:
        if not (isinstance(value, dict) and all(value.get(key) == expected for key, expected in want.items())):
            wrong[shape] += 1
            if len(problems) < 8:
                problems.append(f"{shape}: {value if value is not None else error}")

    try:
        for _ in range(reps):
            agent = fresh("agent")
            tab, _ = runs.run(agent, f"await page.goto({json.dumps(origin + '/outer')}); return page.tabId;")
            full = {"cross": True, "sameSite": True, "frames": 3}
            value, error = runs.run(None, lookup(tab) + walk)
            check("observer", value, error, {**full, "same": False})
            value, error = runs.run(None, lookup(tab) + "await p.title();\n" + walk)
            check("observer-after-title", value, error, {**full, "same": False})
            value, error = runs.run(agent, lookup(tab) + walk)
            check("own-later", value, error, full)
            late = fresh("late")
            late_tab, _ = runs.run(late, f"await page.goto({json.dumps(origin + '/late')}); return page.tabId;")
            value, error = runs.run(
                None,
                lookup(late_tab) + "p.frames();\nawait page.waitForTimeout(2500);\n" + walk,
            )
            check("added", value, error, {"cross": True, "frames": 2})
            for name in (agent, late):
                runs.run(name, f"for (const q of context.pages()) if (q.taskId === {json.dumps(name)}) await q.close(); return 1;")
    finally:
        server.shutdown()
    ok = not any(wrong.values())
    detail = f"{reps} times each: " + ", ".join(f"{shape} {reps - count}/{reps}" for shape, count in wrong.items())
    print(json.dumps({"ok": ok, "detail": detail, "wrong": wrong, "problems": problems}))
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
