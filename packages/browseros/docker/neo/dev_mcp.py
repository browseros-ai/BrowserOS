#!/usr/bin/env python3
"""Call a running Neo's MCP tools from the shell, acting as one agent across calls.

dev.sh's `tools` and `call` commands run this, so agents test a server change
through the MCP surface that ships rather than through Playwright or raw CDP.
Neo ties page ownership to the MCP session, so the session id is kept in a file
and every invocation joins the same session; a fresh session per call would see
the pages its previous call opened as another agent's.

  python3 dev_mcp.py --url URL --state DIR tools [name]
  python3 dev_mcp.py --url URL --state DIR call <tool> [json-args]

Standard library only; the transport is smoke.py's client.
"""

import argparse
import base64
import json
import sys
import time
import urllib.error
from pathlib import Path

from smoke import PROTOCOL, McpClient


class Session:
    """An MCP session that outlives the process, stored in <state>/mcp-session.

    The id is reused until Neo stops recognising it (30 idle minutes, or the
    container was recreated), then one new session replaces it transparently.
    """

    def __init__(self, url: str, state: Path):
        self.file = state / "mcp-session"
        self.client = McpClient(url)
        self.client.session_id = self.file.read_text().strip() if self.file.exists() else None

    def request(self, method: str, params: dict) -> dict:
        if not self.client.session_id:
            self._connect()
        try:
            return self.client.call(method, params)
        except urllib.error.HTTPError as error:
            # rmcp answers 404 for an unknown session id; a stale id is the
            # only failure retried, so real request errors still surface.
            if error.code != 404:
                raise
            self._connect()
            return self.client.call(method, params)

    def tools(self) -> list[dict]:
        found, params = [], {}
        while True:
            page = self.request("tools/list", params)
            found += page["tools"]
            if not page.get("nextCursor"):
                return found
            params = {"cursor": page["nextCursor"]}

    def _connect(self) -> None:
        self.client.session_id = None
        self.client.call(
            "initialize",
            {"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": "neo-dev", "version": "1"}},
        )
        self.client.notify("notifications/initialized")
        self.file.write_text(self.client.session_id or "")


def list_tools(session: Session, name: str | None) -> None:
    listed = session.tools()
    if name is None:
        for tool in listed:
            summary = (tool.get("description") or "").strip().splitlines() or [""]
            print(f"{tool['name']:20} {summary[0][:100]}")
        return
    tool = next((tool for tool in listed if tool["name"] == name), None)
    if tool is None:
        sys.exit(f"no tool named {name}; run `dev.sh tools` for the list")
    print(tool.get("description", ""))
    print(json.dumps(tool["inputSchema"], indent=2))


def call_tool(session: Session, state: Path, name: str, arguments: dict) -> int:
    """Prints text parts and saves image parts to files, since agents read
    screenshots from disk; returns the exit status for the tool's isError."""
    result = session.request("tools/call", {"name": name, "arguments": arguments})
    for part in result.get("content", []):
        if part.get("type") == "text":
            print(part["text"])
        elif part.get("type") == "image":
            shots = state / "shots"
            shots.mkdir(exist_ok=True)
            extension = part.get("mimeType", "image/png").split("/")[-1]
            path = shots / f"{time.strftime('%Y%m%d-%H%M%S')}-{name}.{extension}"
            path.write_bytes(base64.b64decode(part["data"]))
            print(f"[image saved: {path}]")
        else:
            print(json.dumps(part))
    return 1 if result.get("isError") else 0


def main() -> None:
    parser = argparse.ArgumentParser(description="Call a running Neo's MCP tools as one agent.")
    parser.add_argument("--url", required=True, help="MCP endpoint, e.g. http://localhost:19010/mcp")
    parser.add_argument("--state", required=True, type=Path, help="folder for the session id and images")
    commands = parser.add_subparsers(dest="command", required=True)
    tools = commands.add_parser("tools", help="list tools, or show one tool's schema")
    tools.add_argument("name", nargs="?")
    call = commands.add_parser("call", help="call one tool")
    call.add_argument("name")
    call.add_argument("arguments", nargs="?", default="{}", help="JSON object of tool arguments")
    args = parser.parse_args()

    args.state.mkdir(parents=True, exist_ok=True)
    session = Session(args.url, args.state)
    # Agents read these errors to correct their next call, so they get one line
    # (unknown tool, schema mismatch, Neo not running) rather than a traceback.
    try:
        if args.command == "tools":
            list_tools(session, args.name)
        else:
            sys.exit(call_tool(session, args.state, args.name, json.loads(args.arguments)))
    except json.JSONDecodeError as error:
        sys.exit(f"arguments are not a JSON object: {error}")
    except urllib.error.HTTPError as error:
        sys.exit(f"Neo answered HTTP {error.code}: {error.read().decode(errors='replace')[:300]}")
    except urllib.error.URLError as error:
        sys.exit(f"can't reach Neo at {args.url} ({getattr(error, 'reason', error)}); is `dev.sh up` running?")
    except RuntimeError as error:
        sys.exit(str(error))


if __name__ == "__main__":
    main()
