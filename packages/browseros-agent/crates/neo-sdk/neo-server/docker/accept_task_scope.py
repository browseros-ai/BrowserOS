#!/usr/bin/env python3
"""The default tab scope, on the image: accept.sh's `task-scope` check.

The rule under `tabs.scope: task`, the image's default, which this check reads and never
changes: a run that carries a task sees and acts on only its task family's tabs (the tabs its
family's runs opened, the tasks those runs created included); a run without a task sees every
tab but cannot use a task a host named. This is the shape a host meets when it takes the image
as it ships, so the image proves it, not only the server's own tests.

  accept_task_scope.py <mcp-url> <host-socket> <host-token-file>

With fresh host tasks A and B, a tab A opened, a tab a run without a task opened (held by no
task), and a tab of a task A's run created:

  scope      GET /v1/settings says tabs.scope is task
  opened     A's run opens its tab; its result says tabsScope task
  loose      a run without a task opens a tab; its result says tabsScope browser
  created    A's run creates a task and opens a tab in it
  listed     B's run lists none of the three tabs; its result says tabsScope task
  refused-*  B's run that names each of the three by its id is refused page_not_in_task
  own        A's next run lists and reads its own tab and its created task's tab, not the loose one
  holder     GET /v1/tabs names A, and the created task, as their tabs' taskId
  trace      once B's refused runs are complete, no operation of theirs names a tab, and no tab's
             taskIds lists B; A's tab lists A, so the trace links tabs at all
  task-api   a run without a task asking for A (neo.task) gets task_not_found, tabsScope browser
  untasked   a run without a task still lists A's tab and reads its title
  noopener   A's run opens two popups from its own tab, one with window.open(url, '_blank',
             'noopener') and one through a target=_blank rel=noopener link; A's next run lists and
             reads both; B's run lists neither, and naming either is refused page_not_in_task.
             Without an opener the page can reach, Neo still knows each popup's opener, so it
             joins A's task. The pages come from a server here on 127.0.0.1, as window.open
             cannot open a data: URL.

Prints one JSON object: ok, detail, the first problems. Standard library only.
"""

import http.server
import json
import sys
import threading
import uuid

from accept_client import call_run, connect, host


class Site(http.server.BaseHTTPRequestHandler):
    """`/opener`, with a noopener window.open button and a rel=noopener link, and `/child?<name>`."""

    def do_GET(self):
        path, _, name = self.path.partition("?")
        body = {
            "/opener": "<title>opener</title><button id=open onclick=\"window.open('/child?one', '_blank', "
            "'noopener')\">one</button><a id=link href='/child?two' target=_blank rel=noopener>two</a>",
            "/child": f"<title>child {name}</title><p>{name}</p>",
        }.get(path, "")
        self.send_response(200 if body else 404)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body.encode())))
        self.end_headers()
        self.wfile.write(body.encode())

    def log_message(self, format, *args):  # noqa: A002 (the base class's name)
        del format, args


def noopener(scope: Scope, owner: str, other: str, origin: str) -> None:
    """A's noopener popups are A's: its next run has both, another task's run neither."""
    opened = (
        f"const p = await context.newPage(); await p.goto({json.dumps(origin + '/opener')});\n"
        "const [one] = await Promise.all([p.waitForEvent('popup'), p.click('#open')]);\n"
        "const [two] = await Promise.all([p.waitForEvent('popup'), p.click('#link')]);\n"
        "await one.waitForLoadState(); await two.waitForLoadState();\n"
        "return { one: one.tabId, two: two.tabId };"
    )
    value, error, _, _ = scope.run(owner, opened)
    popups = value if isinstance(value, dict) else {}
    one, two = popups.get("one"), popups.get("two")
    if not isinstance(one, int) or not isinstance(two, int):
        scope.expect("noopener-opened", False, (value, error))
        return
    mine = (
        "const t = (id) => context.pages().find((q) => q.tabId === id);\n"
        f"return {{ one: (await t({one})?.title()) ?? null, two: (await t({two})?.title()) ?? null }};"
    )
    value, error, _, _ = scope.run(owner, mine)
    scope.expect("noopener-own", value == {"one": "child one", "two": "child two"}, (value, error))
    listed, error, _, _ = scope.run(other, f"return context.pages().some((q) => q.tabId === {one} || q.tabId === {two});")
    scope.expect("noopener-listed", listed is False, (listed, error))
    for target in (one, two):
        value, error, _, _ = scope.run(other, named(target))
        scope.expect("noopener-refused", error == "page_not_in_task", (target, value, error))


def page_url(title: str) -> str:
    return f"data:text/html,<title>{title}</title><h1>{title}</h1>"


class Scope:
    """`run` calls over one MCP session, with a task in `_meta` or none, and the host socket."""

    def __init__(self, mcp_url: str, socket_path: str, token_file: str, origin: str):
        self.client = connect(mcp_url)
        self.socket_path, self.token_file = socket_path, token_file
        self.origin = origin
        self.problems: list[str] = []

    def run(self, task: str | None, code: str):
        """The script's value, its error code, the result's tabsScope, and the run's id."""
        reply = call_run(self.client, code, {"taskId": task} if task is not None else {})
        content = reply.get("value") or {}
        error = content.get("error") or {}
        meta = reply.get("meta", {})
        return content.get("value"), error.get("code"), meta.get("tabsScope"), meta.get("id")

    def get(self, path: str) -> dict:
        reply = host(self.socket_path, self.token_file, path)
        return reply.get("body", {}) if reply.get("status") == 200 else {}

    def expect(self, name: str, ok: bool, got) -> bool:
        if not ok:
            self.problems.append(f"{name}: {got}")
        return ok


def lookup(tab: int) -> str:
    return f"const p = context.pages().find((q) => q.tabId === {tab});\n"


def named(tab: int) -> str:
    """The page reference a script's own `page.title()` sends, naming `tab` directly."""
    return f"return await __neo.invoke('page.title', {{ page: {{ '$neo': 'page', tab: {tab} }} }});"


def check(scope: Scope) -> None:
    setting = scope.get("/v1/settings").get("settings", {}).get("tabs", {}).get("scope")
    if not scope.expect("scope", setting == "task", setting):
        return
    owner, other = (f"accept-scope-{name}-{uuid.uuid4().hex[:6]}" for name in ("a", "b"))
    opened = f"await page.goto({json.dumps(page_url('own'))}); return page.tabId;"
    tab, error, tabs_scope, owner_run = scope.run(owner, opened)
    if not isinstance(tab, int) or tabs_scope != "task":
        scope.expect("opened", False, (tab, error, tabs_scope))
        return
    loose: int | None = None
    try:
        value, error, tabs_scope, _ = scope.run(None, f"await page.goto({json.dumps(page_url('loose'))}); return page.tabId;")
        if scope.expect("loose", isinstance(value, int) and tabs_scope == "browser", (value, error, tabs_scope)):
            loose = value
        value, error, _, _ = scope.run(
            owner,
            "const t = await neo.createTask(); const p = await t.newPage(); "
            f"await p.goto({json.dumps(page_url('created'))}); return {{ tab: p.tabId, task: t.id }};",
        )
        created = value if isinstance(value, dict) and isinstance(value.get("tab"), int) else None
        if not scope.expect("created", created is not None, (value, error)) or loose is None or created is None:
            return
        tabs = {"own": tab, "loose": loose, "created": created["tab"]}

        listed, error, tabs_scope, _ = scope.run(other, "return context.pages().map((q) => q.tabId);")
        scope.expect(
            "listed",
            isinstance(listed, list) and tabs_scope == "task" and not set(tabs.values()) & set(listed),
            (listed, error, tabs_scope),
        )
        refused_runs = []
        for name, target in tabs.items():
            value, error, _, run_id = scope.run(other, named(target))
            scope.expect(f"refused-{name}", error == "page_not_in_task", (value, error))
            refused_runs.append(run_id)

        mine = (
            "const t = (id) => context.pages().find((q) => q.tabId === id);\n"
            f"return {{ own: (await t({tab})?.title()) ?? null, created: (await t({tabs['created']})?.title()) ?? null, "
            f"loose: !!t({loose}) }};"
        )
        value, error, _, _ = scope.run(owner, mine)
        scope.expect("own", value == {"own": "own", "created": "created", "loose": False}, (value, error))

        # Each run's record is complete before the trace is read, so a link it would add is there.
        for run_id in [owner_run, *refused_runs]:
            record = scope.get(f"/v1/runs/{run_id}?wait=10")
            if not scope.expect("trace-complete", record.get("complete") is True, (run_id, record.get("complete"))):
                continue
            if run_id != owner_run:
                tabbed = [
                    op.get("id")
                    for op in record.get("operations", [])
                    if op.get("tabId") is not None or op.get("page") is not None
                ]
                scope.expect("trace-op", not tabbed, (run_id, tabbed))
        items = {item.get("tabId"): item for item in scope.get("/v1/tabs").get("items", [])}
        holders = {name: items.get(target, {}).get("taskId") for name, target in tabs.items()}
        scope.expect("holder", holders == {"own": owner, "loose": None, "created": created["task"]}, holders)
        task_ids = {name: items.get(target, {}).get("taskIds", []) for name, target in tabs.items()}
        scope.expect("trace-own", owner in task_ids["own"], task_ids)
        scope.expect("trace", all(other not in ids for ids in task_ids.values()), task_ids)

        value, error, tabs_scope, _ = scope.run(None, f"return (await neo.task({json.dumps(owner)})).id;")
        scope.expect(
            "task-api", error == "task_not_found" and tabs_scope == "browser", (value, error, tabs_scope)
        )
        value, error, _, _ = scope.run(None, lookup(tab) + "return p ? await p.title() : null;")
        scope.expect("untasked", value == "own", (value, error))
        noopener(scope, owner, other, scope.origin)
    finally:
        scope.run(owner, "for (const q of context.pages()) await q.close(); return 1;")
        if loose is not None:
            scope.run(None, lookup(loose) + "if (p) await p.close(); return 1;")


def main() -> None:
    mcp_url, socket_path, token_file = sys.argv[1:4]
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Site)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        scope = Scope(mcp_url, socket_path, token_file, f"http://127.0.0.1:{server.server_address[1]}")
        check(scope)
    finally:
        server.shutdown()
    ok = not scope.problems
    detail = (
        "under tabs.scope task: another task's tab, a tab no task holds and a created task's tab "
        "are unlisted and refused page_not_in_task by another task's run, with no trace link; "
        "the owner keeps its own and its created task's tabs and its noopener popups; a run "
        "without a task gets task_not_found for a host's task and still reads the tab"
        if ok
        else "task scope wrong"
    )
    print(json.dumps({"ok": ok, "detail": detail, "problems": scope.problems[:8]}))
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
