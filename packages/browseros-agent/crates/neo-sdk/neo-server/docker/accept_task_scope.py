#!/usr/bin/env python3
"""The default tab scope, on the image: accept.sh's `task-scope` check.

The rule under `tabs.scope: task`, the image's default, which this check reads and never
changes: a run that carries a task sees and acts on only its task family's tabs; a run without a
task sees every tab but cannot use a task a host named. This is the shape a host meets when it
takes the image as it ships, so the image proves it, not only the server's own tests.

  accept_task_scope.py <mcp-url> <host-socket> <host-token-file>

In fresh tasks A and B:

  scope      GET /v1/settings says tabs.scope is task
  opened     A's run opens a tab; its result says tabsScope task
  listed     B's run does not list A's tab in context.pages(); its result says tabsScope task
  refused    B's run that names A's tab by its id is refused page_not_in_task
  holder     GET /v1/tabs still names A as that tab's taskId
  task-api   a run without a task asking for A (neo.task) gets task_not_found, tabsScope browser
  untasked   a run without a task still lists A's tab and reads its title
  own        A's later run still lists and reads its own tab

Prints one JSON object: ok, detail, the first problems. Standard library only.
"""

import json
import sys
import uuid

from accept_client import call_run, connect, host

TITLE = "scoped"
URL = f"data:text/html,<title>{TITLE}</title><h1>{TITLE}</h1>"


class Scope:
    """`run` calls over one MCP session, with a task in `_meta` or none, and the host socket."""

    def __init__(self, mcp_url: str, socket_path: str, token_file: str):
        self.client = connect(mcp_url)
        self.socket_path, self.token_file = socket_path, token_file
        self.problems: list[str] = []

    def run(self, task: str | None, code: str):
        """The script's value, its error code, and the result's tabsScope."""
        reply = call_run(self.client, code, {"taskId": task} if task is not None else {})
        content = reply.get("value") or {}
        error = content.get("error") or {}
        return content.get("value"), error.get("code"), reply.get("meta", {}).get("tabsScope")

    def get(self, path: str) -> dict:
        reply = host(self.socket_path, self.token_file, path)
        return reply.get("body", {}) if reply.get("status") == 200 else {}

    def expect(self, name: str, ok: bool, got) -> bool:
        if not ok:
            self.problems.append(f"{name}: {got}")
        return ok


def lookup(tab: int) -> str:
    return f"const p = context.pages().find((q) => q.tabId === {tab});\n"


def check(scope: Scope) -> None:
    setting = scope.get("/v1/settings").get("settings", {}).get("tabs", {}).get("scope")
    if not scope.expect("scope", setting == "task", setting):
        return
    owner, other = (f"accept-scope-{name}-{uuid.uuid4().hex[:6]}" for name in ("a", "b"))
    tab, error, tabs_scope = scope.run(owner, f"await page.goto({json.dumps(URL)}); return page.tabId;")
    if not isinstance(tab, int) or tabs_scope != "task":
        scope.expect("opened", False, (tab, error, tabs_scope))
        return
    try:
        listed, error, tabs_scope = scope.run(other, f"return context.pages().some((q) => q.tabId === {tab});")
        scope.expect("listed", listed is False and tabs_scope == "task", (listed, error, tabs_scope))
        # The page reference a script's own `page.title()` sends, naming A's tab directly.
        named = f"return await __neo.invoke('page.title', {{ page: {{ '$neo': 'page', tab: {tab} }} }});"
        value, error, _ = scope.run(other, named)
        scope.expect("refused", error == "page_not_in_task", (value, error))
        items = scope.get("/v1/tabs").get("items", [])
        holder = next((item.get("taskId") for item in items if item.get("tabId") == tab), None)
        scope.expect("holder", holder == owner, holder)
        value, error, tabs_scope = scope.run(None, f"return (await neo.task({json.dumps(owner)})).id;")
        scope.expect(
            "task-api", error == "task_not_found" and tabs_scope == "browser", (value, error, tabs_scope)
        )
        value, error, _ = scope.run(None, lookup(tab) + "return p ? await p.title() : null;")
        scope.expect("untasked", value == TITLE, (value, error))
        value, error, _ = scope.run(owner, lookup(tab) + "return p ? { title: await p.title(), task: p.taskId } : null;")
        scope.expect("own", value == {"title": TITLE, "task": owner}, (value, error))
    finally:
        scope.run(owner, "for (const q of context.pages()) await q.close(); return 1;")


def main() -> None:
    mcp_url, socket_path, token_file = sys.argv[1:4]
    scope = Scope(mcp_url, socket_path, token_file)
    check(scope)
    ok = not scope.problems
    detail = (
        "under tabs.scope task: another task's tab unlisted and refused page_not_in_task, its "
        "holder kept; a run without a task gets task_not_found for a host's task and still reads "
        "the tab; the owner task still has it"
        if ok
        else "task scope wrong"
    )
    print(json.dumps({"ok": ok, "detail": detail, "problems": scope.problems[:8]}))
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
