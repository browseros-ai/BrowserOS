#!/usr/bin/env python3
"""Which tab a task's next run starts on, on the image: accept.sh's `task-page` check.

The rule: a run's built-in `page` is its task's own tab (one the task opened or was given), the
one its own script last acted on. Neo's own operations never move it (the frame refresh that
ends every `page.waitForTimeout`, its listeners, the event pump), and it is never another task's
tab or the browser's New Tab, even one the script reached through `context.pages()`. A task with
no open tab gets a new one.

  accept_task_page.py <mcp-url> <host-socket> <host-token-file> [reps] [shapes]

Every shape runs `reps` times (default 20), each repetition in fresh tasks, beside a neighbour
task's tab and whatever else the browser has open (its New Tab). The shapes are the first host's
(it found the bug on the published image): a later run's `page` was another task's tab whenever
the previous run's last call was a wait.

  wait-middle    goto; a run that only waits; the next run reads `page`
  ends-on-wait   goto, a click and a read, then a wait as the script's last call
  wait-loop      a wait loop that the run's deadline ends (a card entry that never saves)
  other-tab      the script itself acts on every other tab through `context.pages()`. Only the
                 `browser` tab scope lets a task's run reach another task's tab, so this shape
                 sets `tabs.scope` to `browser` over the host socket for its own repetitions
                 and puts the value it found back after
  interleaved    two tasks: each goes to its page, each waits, each reads
  closed         a run without a task (the owner's or a host's own script, which sees every tab)
                 closes the task's tab after a wait: its next run opens a new tab of its own

Prints one JSON object: ok, how many repetitions of each shape went wrong, the first problems.
Standard library only.
"""

import json
import sys
import uuid

from accept_client import RUN_META, connect, host

WAIT = "await page.waitForTimeout(300); return 1;"
READ = "return { tab: page.tabId, task: page.taskId };"


def url(title: str) -> str:
    return f"data:text/html,<title>{title}</title><h1>{title}</h1>"


class Runs:
    """`run` calls over one MCP session, each naming its task in `_meta` as a host does, or none
    for the owner's or a host's own script; and the host socket, for the tab scope."""

    def __init__(self, mcp_url: str, socket_path: str, token_file: str):
        self.client = connect(mcp_url)
        self.socket_path, self.token_file = socket_path, token_file

    def run(self, task: str | None, code: str, timeout: int = 60_000):
        result = self.client.call(
            "tools/call",
            {
                "name": "run",
                "arguments": {"code": code, "timeout": timeout},
                "_meta": {RUN_META: {"taskId": task} if task is not None else {}},
            },
        )
        structured = result.get("structuredContent") or {}
        return structured.get("value"), structured.get("error")

    def goto(self, task: str) -> int | None:
        value, _ = self.run(task, f"await page.goto({json.dumps(url(task))}); return page.tabId;")
        return value

    def page(self, task: str) -> dict:
        value, error = self.run(task, READ)
        return value if isinstance(value, dict) else {"error": error}

    def tab_scope(self, scope: str | None = None) -> str | None:
        """`tabs.scope` as the server has it, after setting it to `scope` when one is given."""
        if scope is None:
            reply = host(self.socket_path, self.token_file, "/v1/settings")
        else:
            body = json.dumps({"tabs": {"scope": scope}})
            reply = host(self.socket_path, self.token_file, "/v1/settings", "PATCH", body)
        settings = reply.get("body", {}).get("settings", {}) if reply.get("status") == 200 else {}
        return settings.get("tabs", {}).get("scope")

    def close(self, task: str) -> None:
        """Closes the tabs `task` owns, never `page` itself: where `page` is wrong it is a
        neighbour's tab."""
        self.run(
            task,
            f"for (const q of context.pages()) if (q.taskId === {json.dumps(task)}) "
            "await q.close(); return 1;",
        )


def task(shape: str, i: int) -> str:
    return f"accept-tp-{shape}-{i}-{uuid.uuid4().hex[:6]}"


def waited(runs: Runs, name: str, problems: list, label: str) -> bool:
    """Runs the wait in `name`'s task; a wait that failed proves nothing about the next page."""
    value, error = runs.run(name, WAIT)
    if value == 1:
        return True
    problems.append(f"{label}: the wait failed: {error}")
    return False


def own_page(runs: Runs, name: str, opened, problems: list, label: str) -> bool:
    """Whether `name`'s next run starts on `opened`, its own tab."""
    got = runs.page(name)
    if opened is not None and got.get("tab") == opened and got.get("task") == name:
        return True
    problems.append(f"{label}: opened {opened}, next page {got}")
    return False


def wait_middle(runs: Runs, i: int, problems: list) -> bool:
    name = task("wait-middle", i)
    opened = runs.goto(name)
    if not waited(runs, name, problems, f"wait-middle {i}"):
        return False
    ok = own_page(runs, name, opened, problems, f"wait-middle {i}")
    runs.close(name)
    return ok


def ends_on_wait(runs: Runs, i: int, problems: list) -> bool:
    name = task("ends-on-wait", i)
    opened, _ = runs.run(
        name,
        f"await page.goto({json.dumps(url(name))}); await page.click('h1'); "
        "await page.title(); await page.waitForTimeout(300); return page.tabId;",
    )
    ok = own_page(runs, name, opened, problems, f"ends-on-wait {i}")
    runs.close(name)
    return ok


def wait_loop(runs: Runs, i: int, problems: list) -> bool:
    name = task("wait-loop", i)
    opened = runs.goto(name)
    _, error = runs.run(
        name, "await page.click('h1'); for (;;) await page.waitForTimeout(200);", timeout=1500
    )
    if "timeout" not in json.dumps(error).lower():
        problems.append(f"wait-loop {i}: not ended by its deadline: {error}")
        return False
    ok = own_page(runs, name, opened, problems, f"wait-loop {i}")
    runs.close(name)
    return ok


def other_tab(runs: Runs, i: int, problems: list) -> bool:
    name = task("other-tab", i)
    opened = runs.goto(name)
    acted, error = runs.run(
        name,
        "let n = 0; for (const q of context.pages()) if (q !== page) { await q.title(); n++; } "
        "return n;",
    )
    if not isinstance(acted, int) or acted < 1:
        problems.append(f"other-tab {i}: acted on no other tab: {acted} {error}")
        return False
    ok = own_page(runs, name, opened, problems, f"other-tab {i}")
    runs.close(name)
    return ok


def interleaved(runs: Runs, i: int, problems: list) -> bool:
    first, second = task("interleaved-a", i), task("interleaved-b", i)
    opened = {first: runs.goto(first), second: runs.goto(second)}
    if not all([waited(runs, name, problems, f"interleaved {i}") for name in opened]):
        return False
    ok = all([own_page(runs, name, opened[name], problems, f"interleaved {i}") for name in opened])
    runs.close(first)
    runs.close(second)
    return ok


def closed(runs: Runs, i: int, problems: list) -> bool:
    name = task("closed", i)
    opened = runs.goto(name)
    if not waited(runs, name, problems, f"closed {i}"):
        return False
    # Closed from outside the task, as the owner or a host's own script closes a tab: a run
    # without a task, which sees every tab under either tab scope.
    shut, error = runs.run(
        None,
        f"const q = context.pages().find((p) => p.tabId === {json.dumps(opened)}); "
        "if (!q) return 0; await q.close(); return 1;",
    )
    if shut != 1:
        problems.append(f"closed {i}: the task's tab {opened} was not closed: {shut} {error}")
        return False
    value, error = runs.run(
        name,
        "const before = page.tabId; const existing = context.pages().map((q) => q.tabId); "
        f"await page.goto({json.dumps(url(name))}); "
        "return { before, existing, tab: page.tabId, task: page.taskId };",
    )
    value = value if isinstance(value, dict) else {"error": error}
    fresh = value.get("tab")
    ok = (
        value.get("before") is None
        and fresh is not None
        and fresh != opened
        and fresh not in (value.get("existing") or [])
        and value.get("task") == name
    )
    if not ok:
        problems.append(f"closed {i}: closed {opened}, next run {value}")
    runs.close(name)
    return ok


SHAPES = {
    "wait-middle": wait_middle,
    "ends-on-wait": ends_on_wait,
    "wait-loop": wait_loop,
    "other-tab": other_tab,
    "interleaved": interleaved,
    "closed": closed,
}


def shape_runs(runs: Runs, shape: str, reps: int, problems: list) -> int:
    """How many of `shape`'s repetitions went wrong. `other-tab` runs under the `browser` tab
    scope, set for its repetitions alone and then put back. Every later check assumes the
    server's own scope, so a scope that cannot be read first, cannot be set, or is not put back
    fails every repetition, whatever they found, and its problem is listed first."""
    check = SHAPES[shape]
    if shape != "other-tab":
        return sum(0 if check(runs, i, problems) else 1 for i in range(1, reps + 1))
    before = runs.tab_scope()
    if before is None:
        problems.insert(0, "other-tab: tabs.scope could not be read, so it was left alone")
        return reps
    if runs.tab_scope("browser") != "browser":
        restored = runs.tab_scope(before) == before
        problems.insert(0, f"other-tab: tabs.scope did not become browser (was {before}, put back: {restored})")
        return reps
    wrong = reps
    try:
        wrong = sum(0 if check(runs, i, problems) else 1 for i in range(1, reps + 1))
    finally:
        restored = runs.tab_scope(before) == before
    if not restored:
        problems.insert(0, f"other-tab: tabs.scope was not put back to {before}")
        return reps
    return wrong


def accept(runs: Runs, reps: int, shapes: list[str]) -> dict:
    """The check's result, which accept.sh reads: `ok` is false when any repetition of any shape
    went wrong, including every repetition of a shape whose tab scope could not be read, set or
    put back."""
    # A neighbour task's tab stays open throughout: the tab a wrong `page` most often was.
    neighbour = task("neighbour", 0)
    runs.goto(neighbour)
    problems: list = []
    wrong = {shape: shape_runs(runs, shape, reps, problems) for shape in shapes}
    runs.close(neighbour)
    counts = ", ".join(f"{shape} {wrong[shape]}/{reps}" for shape in shapes)
    return {
        "ok": not any(wrong.values()),
        "wrong": wrong,
        "reps": reps,
        "detail": f"next run's page not the task's own tab: {counts}",
        "problems": problems[:12],
    }


def main() -> None:
    mcp_url, socket_path, token_file = sys.argv[1:4]
    reps = int(sys.argv[4]) if len(sys.argv) > 4 else 20
    shapes = sys.argv[5].split(",") if len(sys.argv) > 5 else list(SHAPES)
    print(json.dumps(accept(Runs(mcp_url, socket_path, token_file), reps, shapes)))


if __name__ == "__main__":
    main()
