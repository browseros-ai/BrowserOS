#!/usr/bin/env python3
"""Call a running Neo's MCP tools from the shell, acting as one agent across calls.

dev.sh's `tools`, `call` and `run` commands run this, so agents test a server
change through the MCP surface that ships rather than through Playwright or raw
CDP. claw-server ties page ownership to the MCP session, so the session id is
kept in a file and every invocation joins the same session; a fresh session per
call would see the pages its previous call opened as another agent's. claw-sdk
is stateless: it issues no id, ignores a stored one, and ties pages to the
run's taskId instead.

  python3 dev_mcp.py --url URL --state DIR tools [name]
  python3 dev_mcp.py --url URL --state DIR call <tool> [json-args]
  python3 dev_mcp.py --url URL --state DIR run [CODE | - | -f FILE] [--task ID]
      [--timeout MS] [--request-id ID] [--json] --traces-dir DIR
      [--host-url URL --token-file FILE]

`run` is claw-sdk's tool with its host ids in `_meta`. Each call is appended to
<traces-dir>/runs.jsonl, which the dev kit's trace UI joins to Neo's own trace
by requestId.

Standard library only; the transport is smoke.py's client.
"""

import argparse
import base64
import fcntl
import json
import secrets
import sys
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

from smoke import PROTOCOL, McpClient

# The `_meta` key claw-sdk reads the host's ids from, and returns the run's
# id, status and effects under.
RUN_META = "com.browseros.neo/run"


class Session:
    """An MCP session that outlives the process, stored in <state>/mcp-session.

    The id is reused until Neo stops recognising it (30 idle minutes, or the
    container was recreated), then one new session replaces it transparently.
    """

    def __init__(self, url: str, state: Path):
        self.file = state / "mcp-session"
        self.lock = state / "mcp-session.lock"
        self.client = McpClient(url)
        self.client.session_id = self._stored()
        # rmcp routes each reply by JSON-RPC id within the session, so two
        # overlapping invocations both counting from 1 would swap or lose
        # replies. A random starting id keeps parallel calls apart without a
        # lock that would queue a quick call behind a long `run`.
        self.client.next_id = secrets.randbits(48)

    def request(self, method: str, params: dict) -> dict:
        if not self.client.session_id:
            self._connect(stale=None)
        try:
            return self.client.call(method, params)
        except urllib.error.HTTPError as error:
            # rmcp answers 404 for an unknown session id; a stale id is the
            # only failure retried, so real request errors still surface.
            if error.code != 404:
                raise
            self._connect(stale=self.client.session_id)
            return self.client.call(method, params)

    def tools(self) -> list[dict]:
        found, params = [], {}
        while True:
            page = self.request("tools/list", params)
            found += page["tools"]
            if not page.get("nextCursor"):
                return found
            params = {"cursor": page["nextCursor"]}

    def _stored(self) -> str | None:
        return (self.file.read_text().strip() or None) if self.file.exists() else None

    def _connect(self, stale: str | None) -> None:
        """Adopts or creates the shared session. Parallel first calls would
        otherwise each create one and split page ownership between them, so
        creation is serialised and a session another call just stored wins.
        Only this step is locked; tool calls themselves still run in parallel."""
        with open(self.lock, "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            stored = self._stored()
            if stored and stored != stale:
                self.client.session_id = stored
                return
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
    """Calls one tool and prints its result; returns the exit status for the
    tool's isError."""
    result = session.request("tools/call", {"name": name, "arguments": arguments})
    print_content(result, state, name)
    return 1 if result.get("isError") else 0


def print_content(result: dict, state: Path, name: str) -> None:
    """Prints text parts and saves image parts to files, since agents read
    screenshots from disk."""
    for part in result.get("content", []):
        if part.get("type") == "text":
            print(part["text"])
        elif part.get("type") == "image":
            shots = state / "shots"
            shots.mkdir(exist_ok=True)
            extension = part.get("mimeType", "image/png").split("/")[-1]
            # Unique per image: a printed path must keep showing what that
            # call captured, even across calls in the same second.
            path = shots / f"{time.strftime('%Y%m%d-%H%M%S')}-{name}-{secrets.token_hex(3)}.{extension}"
            path.write_bytes(base64.b64decode(part["data"]))
            print(f"[image saved: {path}]")
        else:
            print(json.dumps(part))


def run_code(session: Session, args: argparse.Namespace, code: str) -> int:
    """One claw-sdk `run`, recorded whatever happens to it.

    The record goes to runs.jsonl even when no reply came back, so the trace
    UI can show a call Neo never answered next to the runs Neo recorded. Its
    requestId is the join key with Neo's trace (`Run.requestId`); the run id
    lives only in the reply's `_meta`, so the record never stores it twice.
    """
    meta = {"requestId": args.request_id or f"dev-{uuid.uuid4().hex}"}
    if args.task:
        meta["taskId"] = args.task
    arguments = {"code": code}
    if args.timeout is not None:
        arguments["timeout"] = args.timeout
    response, transport_error = None, None
    started = epoch_ms()
    try:
        response = session.request("tools/call", {"name": "run", "arguments": arguments, "_meta": {RUN_META: meta}})
    except (OSError, RuntimeError, ValueError) as error:
        transport_error = transport_message(error, session.client.url)
    ended = epoch_ms()
    record = {
        "requestId": meta["requestId"],
        "taskId": args.task,
        "code": code,
        "timeoutMs": args.timeout,
        "timing": {"startedAt": started, "endedAt": ended, "durationMs": ended - started},
        "response": response,
        "transportError": transport_error,
    }
    append_record(args.traces_dir / "runs.jsonl", record)
    if args.json:
        print(json.dumps(record, indent=2))
    elif response is None:
        print(f"run - transport_error {ended - started}ms: {transport_error}", file=sys.stderr)
    else:
        print_content(response, args.state, "run")
        print(status_line(response, ended - started, run_tabs(args, response)))
    return 1 if response is None or response.get("isError") else 0


def status_line(response: dict, duration_ms: int, tabs: list | None) -> str:
    """`run <id> <status> effects=<e> <ms>ms`, then the tabs and the error's
    owner/code when known: one line a shell script can split on spaces."""
    run = response.get("_meta", {}).get(RUN_META, {})
    line = f"run {run.get('id', '-')} {run.get('status', '-')} effects={run.get('effects', '-')} {duration_ms}ms"
    if tabs is not None:
        line += f" tabs=[{','.join(map(str, tabs))}]"
    if error := run.get("error"):
        line += f" error={error.get('owner')}/{error.get('code')}"
    return line


def run_tabs(args: argparse.Namespace, response: dict) -> list | None:
    """The tabs the run's operations targeted, from Neo's trace over the host
    API. Best effort: the status line leaves them out when the API is not
    there (claw-server, or no token). `wait` holds the read until the run is
    `complete`, because the reply can come first: a run refused by a lock read
    `tabs: []` straight after its reply and its tab a moment later."""
    run_id = response.get("_meta", {}).get(RUN_META, {}).get("id")
    if not (args.host_url and args.token_file and run_id):
        return None
    try:
        token = args.token_file.read_text().strip()
        request = urllib.request.Request(
            f"{args.host_url}/v1/runs/{run_id}?wait=10", headers={"Authorization": f"Bearer {token}"}
        )
        with urllib.request.urlopen(request, timeout=15) as reply:
            return json.load(reply).get("tabs")
    except (OSError, ValueError):
        return None


def append_record(path: Path, record: dict) -> None:
    """Appends one whole line under an exclusive lock. Several CLIs may run at
    once, and a reader skips a torn line, so without the lock a run could
    vanish from the log."""
    path.parent.mkdir(parents=True, exist_ok=True)
    line = json.dumps(record, separators=(",", ":")) + "\n"
    with open(path, "a") as log:
        fcntl.flock(log, fcntl.LOCK_EX)
        log.write(line)


def read_code(parser: argparse.ArgumentParser, args: argparse.Namespace) -> str:
    if (args.code is None) == (args.file is None):
        parser.error("give the script once: as CODE, as - (stdin), or with -f FILE")
    if args.file is not None:
        try:
            return args.file.read_text()
        except OSError as error:
            parser.error(f"can't read {args.file}: {error.strerror}")
    return sys.stdin.read() if args.code == "-" else args.code


def epoch_ms() -> int:
    return time.time_ns() // 1_000_000


def transport_message(error: Exception, url: str) -> str:
    """One line an agent can act on, rather than a traceback."""
    if isinstance(error, urllib.error.HTTPError):
        return f"Neo answered HTTP {error.code}: {error.read().decode(errors='replace')[:300]}"
    if isinstance(error, urllib.error.URLError):
        return f"can't reach Neo at {url} ({getattr(error, 'reason', error)}); is `dev.sh up` running?"
    return str(error) or repr(error)


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
    run = commands.add_parser("run", help="send one script to claw-sdk's run tool and log the call")
    run.add_argument("code", nargs="?", help="the script, or - to read it from stdin")
    run.add_argument("-f", "--file", type=Path, help="read the script from this file")
    run.add_argument("--task", help="host task id (taskId): a task's runs share its last tab")
    run.add_argument("--timeout", type=int, metavar="MS", help="run timeout in ms (Neo's default: 300000)")
    run.add_argument("--request-id", help="host request id (requestId); default dev-<uuid4 hex>")
    run.add_argument("--json", action="store_true", help="print the whole logged record instead")
    run.add_argument("--traces-dir", type=Path, required=True, help="folder of runs.jsonl")
    run.add_argument("--host-url", help="host API origin, to read the run's tabs")
    run.add_argument("--token-file", type=Path, help="the host API's bearer token")
    args = parser.parse_args()

    args.state.mkdir(parents=True, exist_ok=True)
    session = Session(args.url, args.state)
    # Agents read these errors to correct their next call, so they get one line
    # (unknown tool, schema mismatch, Neo not running) rather than a traceback.
    try:
        if args.command == "tools":
            list_tools(session, args.name)
        elif args.command == "call":
            sys.exit(call_tool(session, args.state, args.name, json.loads(args.arguments)))
        else:
            sys.exit(run_code(session, args, read_code(run, args)))
    except json.JSONDecodeError as error:
        sys.exit(f"arguments are not a JSON object: {error}")
    except (urllib.error.URLError, RuntimeError) as error:
        sys.exit(transport_message(error, args.url))


if __name__ == "__main__":
    main()
