#!/usr/bin/env python3
"""End-to-end check of a running BrowserOS neo container, from the host.

It talks to the container the way a developer does: through the published
viewer and MCP ports. Passing means the noVNC viewer is served, an agent
outside the container can reach Neo's MCP server (which proves the
allow_remote_in_mcp setting works through Docker's port forwarding), agents get
the hosted tool set (BROWSERCLAW_HOSTED_MODE=1), an agent-opened tab loads
and, in hosted mode, stays out of any tab group (the shipped neo.toml keeps the
explicit grouping default), and a new tab page is blank (the entrypoint's
--disable-browseros-features list reached the browser).

  python3 smoke.py [--vnc-port 6080] [--mcp-port 9010]

Standard library only, so it runs anywhere python3 does.
"""

import argparse
import json
import re
import sys
import time
import urllib.request

PROTOCOL = "2025-06-18"
HUMAN_HELP_TOOLS = {"request_human_help", "await_human_help"}
# Server tools a hosted claw-server does not offer.
HOSTED_HIDDEN_TOOLS = HUMAN_HELP_TOOLS | {"name_session", "save_skill", "mark_skill_run"}


class McpClient:
    """Minimal streamable-HTTP MCP client for Neo's rmcp server.

    Sessions are stateful (the mcp-session-id header from initialize must ride
    on every later request) and replies may arrive as SSE frames, each stream
    primed with an empty data frame that must be skipped.
    """

    def __init__(self, url: str):
        self.url = url
        self.session_id = None
        self.next_id = 0

    def _post(self, body: dict):
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        }
        if self.session_id:
            headers["mcp-session-id"] = self.session_id
            headers["mcp-protocol-version"] = PROTOCOL
        request = urllib.request.Request(self.url, json.dumps(body).encode(), headers)
        response = urllib.request.urlopen(request, timeout=120)
        self.session_id = response.headers.get("mcp-session-id") or self.session_id
        return response

    def notify(self, method: str) -> None:
        self._post({"jsonrpc": "2.0", "method": method, "params": {}}).read()

    def call(self, method: str, params: dict | None = None) -> dict:
        self.next_id += 1
        response = self._post(
            {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params or {}}
        )
        if "event-stream" not in response.headers.get("content-type", ""):
            return self._result(method, json.loads(response.read()))
        data: list[str] = []
        for raw in response:
            line = raw.decode().rstrip("\r\n")
            if line.startswith("data:"):
                data.append(line[5:].strip())
            elif not line and data:
                text, data = "\n".join(data).strip(), []
                if text and (message := json.loads(text)).get("id") == self.next_id:
                    return self._result(method, message)
        raise RuntimeError(f"{method}: stream ended without a reply")

    @staticmethod
    def _result(method: str, message: dict) -> dict:
        if "error" in message:
            raise RuntimeError(f"{method}: {message['error']}")
        return message["result"]

    def tool(self, name: str, arguments: dict) -> str:
        result = self.call(
            "tools/call",
            {"name": name, "arguments": {**arguments, "agentName": "neo-docker-smoke"}},
        )
        text = "\n".join(
            part.get("text", "")
            for part in result.get("content", [])
            if part.get("type") == "text"
        )
        if result.get("isError"):
            raise RuntimeError(f"{name}: {text[:300]}")
        return text

    def tools(self) -> list[dict]:
        """Read every page so release checks also cover tools beyond page one."""
        tools = []
        params = {}
        while True:
            page = self.call("tools/list", params)
            tools.extend(page["tools"])
            if not page.get("nextCursor"):
                return tools
            params = {"cursor": page["nextCursor"]}


def accepts_task_name(client: McpClient) -> bool:
    """Whether this claw-server knows the taskName argument, in either mode.

    A server older than hosted-mode support rejects it as an unknown field. That is
    what separates "the bundled server predates hosted mode" from "hosted mode
    is off". The latter can also mean the bundled server predates the current flag.
    """
    try:
        client.tool("tabs", {"action": "list", "taskName": "docker smoke"})
    except RuntimeError as error:
        if "taskName" not in str(error):
            raise
        return False
    return True


def check(label: str, condition: bool, detail: str = "") -> None:
    print(f"{'ok  ' if condition else 'FAIL'} {label}{f' ({detail})' if detail else ''}")
    if not condition:
        sys.exit(1)


def main() -> None:
    parser = argparse.ArgumentParser(description="End-to-end check of a running BrowserOS neo container.")
    parser.add_argument("--vnc-port", type=int, default=6080)
    parser.add_argument("--mcp-port", type=int, default=9010)
    parser.add_argument("--url", default="https://example.com")
    args = parser.parse_args()

    viewer = f"http://localhost:{args.vnc_port}"
    with urllib.request.urlopen(f"{viewer}/", timeout=10) as response:
        check("viewer index served", response.status == 200 and b"vnc.html" in response.read())
    with urllib.request.urlopen(f"{viewer}/vnc.html", timeout=10) as response:
        check("noVNC client served", response.status == 200)

    client = McpClient(f"http://localhost:{args.mcp_port}/mcp")
    info = client.call(
        "initialize",
        {
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": {"name": "neo-docker-smoke", "version": "1"},
        },
    )
    check("MCP initialize from the host", bool(client.session_id), info.get("serverInfo", {}).get("name", ""))
    client.notify("notifications/initialized")

    listed = client.tools()
    tools = {tool["name"] for tool in listed}
    check("MCP lists the tabs tool", "tabs" in tools, f"{len(tools)} tools")
    check("human help disabled", not tools & HUMAN_HELP_TOOLS)
    tabs = next(tool for tool in listed if tool["name"] == "tabs")
    hosted = "taskName" in tabs["inputSchema"].get("properties", {})
    if hosted:
        hidden = tools & HOSTED_HIDDEN_TOOLS
        check("hosted mode hides server-only tools", not hidden, ", ".join(sorted(hidden)))
    else:
        check(
            "claw-server is not in desktop mode",
            not accepts_task_name(client),
            "it knows taskName, but hosted mode is off: check BROWSERCLAW_HOSTED_MODE and the bundled server version",
        )
        print("skip hosted mode: this Neo's claw-server predates hosted-mode support")

    new_tab = {"action": "new", "url": args.url}
    if hosted:
        new_tab["taskName"] = "docker smoke"
    opened = client.tool("tabs", new_tab)
    check("agent opened a tab", "opened page" in opened, opened.splitlines()[0] if opened else "")
    if hosted:
        check_ungrouped(client, int(re.search(r"opened page (\d+)", opened).group(1)))
    check_new_tab_blank(client)


def check_new_tab_blank(client: McpClient) -> None:
    """The entrypoint's hidden-feature list reached the browser.

    Most of the names in that list only change the browser's frame, which no tool can
    see. NewTabPage is the exception: it sends every new tab page to about:blank, where
    Chromium or the bundled extension would otherwise draw one. A browser older than
    0.51.2.5 ignores the switch, so it fails here instead of shipping the full frame.
    """
    opened = client.tool("tabs", {"action": "new", "url": "chrome://newtab/"})
    page = int(re.search(r"opened page (\d+)", opened).group(1))
    # The tab keeps chrome://newtab/ as its address, so ask the page where it really is.
    # The value is the one line between the reply's [UNTRUSTED...] markers and its tip.
    reply = client.tool("evaluate", {"page": page, "code": "return location.href"})
    href = next((line for line in reply.splitlines() if line and not line.startswith(("[", "Tip:"))), "")
    check("new tab page is blank", href == "about:blank", href)


def check_ungrouped(client: McpClient, page: int) -> None:
    """Hosted mode's explicit tab grouping: the server never groups a page itself.

    The listing's text shows ` [group <id>]` on every grouped page, and it is what an agent
    reads. Automatic grouping runs after the tool call returns, so one listing could pass
    before the group lands; listing for a few seconds catches a group that arrives late.
    """
    line = ""
    for _ in range(6):
        listing = client.tool("tabs", {"action": "list"})
        line = next((row for row in listing.splitlines() if row.startswith(f"[{page}] ")), "")
        if not line or "[group " in line:
            break
        time.sleep(0.5)
    check("hosted tabs stay ungrouped", bool(line) and "[group " not in line, line or f"page {page} not listed")

if __name__ == "__main__":
    main()
