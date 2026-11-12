#!/usr/bin/env python3
"""End-to-end check of a running claw-sdk Neo container, from the host.

claw-sdk serves one stateless MCP tool, `run`, which executes JavaScript
against Playwright-style globals (sdk-skeleton D3 and D5). Passing means
/system/health and /system/ready answer 200 on the published MCP port, an
agent outside the container initializes without being handed a session id and
sees exactly that one tool, and one run drives a real tab: it opens the tab,
loads a data: URL, reads the page title and closes the tab. claw-server issues
a session id and lists many tools, so an image that runs the wrong server
fails here as well. This is the claw-sdk counterpart of smoke.py.

  python3 smoke_sdk.py [--mcp-port 9010]

Standard library only, so it runs anywhere python3 does.
"""

import argparse
import json
import urllib.error
import urllib.request

from smoke import PROTOCOL, McpClient, check

TITLE = "neo-sdk-smoke"
# The run returns the title upper-cased, and the code holds it only in lower
# case. So the result can carry EXPECTED only if a tab really loaded the page.
# Error text that echoes the code cannot match it.
EXPECTED = TITLE.upper()
# The script calls only the four placeholder built-ins (sdk-skeleton plan step
# 9), so it passes before sdk-playwright plugs in. It names its tab `tab` so it
# never redeclares the `page` global.
CODE = f"""\
const tab = await context.newPage();
await tab.goto({json.dumps(f"data:text/html,<title>{TITLE}</title>")});
const title = (await tab.title()).toUpperCase();
await tab.close();
return title;
"""
# In milliseconds (sdk-skeleton D5). A hung run fails within a minute with
# claw-sdk's own error instead of running to its 300 s default.
RUN_TIMEOUT_MS = 60_000


def http_status(url: str) -> int:
    try:
        with urllib.request.urlopen(url, timeout=15) as response:
            return response.status
    except urllib.error.HTTPError as error:  # urllib raises for every non-2xx answer
        return error.code


def main() -> None:
    parser = argparse.ArgumentParser(description="End-to-end check of a running claw-sdk Neo container.")
    parser.add_argument("--mcp-port", type=int, default=9010)
    args = parser.parse_args()
    origin = f"http://localhost:{args.mcp_port}"

    for endpoint in ("health", "ready"):
        got = http_status(f"{origin}/system/{endpoint}")
        check(f"/system/{endpoint} answers 200", got == 200, str(got))

    # smoke.py's client sends MCP-Protocol-Version only with a session id, so
    # no request after initialize carries it. rmcp's stateless mode reads that
    # as 2025-03-26 and serves it while claw-sdk leaves
    # stateless_protocol_metadata_required off.
    client = McpClient(f"{origin}/mcp")
    info = client.call("initialize", {
        "protocolVersion": PROTOCOL, "capabilities": {},
        "clientInfo": {"name": "neo-docker-smoke", "version": "1"},
    })
    check("MCP initialize from the host", "serverInfo" in info, info.get("serverInfo", {}).get("name", ""))
    check("stateless: no session id issued", client.session_id is None, client.session_id or "")
    client.notify("notifications/initialized")

    tools = client.tools()
    names = [tool["name"] for tool in tools]
    check("MCP lists exactly the run tool", names == ["run"], ", ".join(names))
    properties = tools[0].get("inputSchema", {}).get("properties", {})
    check("run takes code", "code" in properties, ", ".join(properties))

    # Not client.tool(): it adds claw-server's agentName, which run's input lacks.
    result = client.call("tools/call", {"name": "run", "arguments": {"code": CODE, "timeout": RUN_TIMEOUT_MS}})
    # Where run's result puts the returned value is not pinned yet, so the
    # check looks for it anywhere in the result.
    text = json.dumps(result)
    check("a run opened a tab, read its title and closed it", not result.get("isError") and EXPECTED in text, text[:300])


if __name__ == "__main__":
    main()
