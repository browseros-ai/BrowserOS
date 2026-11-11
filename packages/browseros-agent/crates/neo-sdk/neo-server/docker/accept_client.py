#!/usr/bin/env python3
"""accept.sh's probes, run inside the containers under test with the image's own python3.

accept.sh copies this file next to the Neo image's smoke.py (whose streamable-HTTP MCP client
it reuses) and runs one command per call, as whichever uid the check needs. Each command prints
one JSON object on stdout; accept.sh decides PASS or FAIL from it.

  run <mcp-url> <task-id> <request-id>   a `run` that drives a real page, with the host's _meta
  describe <mcp-url>                     the `run` tool's description, which lists neo.* methods
  host <socket> <token-file|-> <path>    GET a host API path over the Unix socket
  listeners                              the container's listening TCP sockets, from /proc
  probe <host> <port> http|https         GET /system/ready from another container

Standard library only.
"""

import json
import socket
import ssl
import sys
import urllib.parse

RUN_META = "com.browseros.neo/run"
TITLE = "neo-sdk-accept"
# A data: page in the task's own tab, filled and read back through Playwright locators: the
# shape of call a migrated zuzu mirrord sends. The image under test has no network.
CODE = f"""\
await page.goto('data:text/html,<title>{TITLE}</title><h1 id=h>hello</h1><input aria-label=Name>');
await page.getByLabel('Name').fill('zuzu');
return {{ title: await page.title(), heading: await page.locator('#h').textContent(),
         value: await page.getByLabel('Name').inputValue() }};
"""


def connect(url: str):
    # Imported here so `probe` runs from stdin in a container that has only this file.
    from smoke import PROTOCOL, McpClient

    client = McpClient(url)
    client.call(
        "initialize",
        {
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": {"name": "neo-sdk-accept", "version": "1"},
        },
    )
    client.notify("notifications/initialized")
    return client


def describe(url: str) -> dict:
    tools = connect(url).tools()
    return {"description": next(t.get("description", "") for t in tools if t["name"] == "run")}


def run(url: str, task_id: str, request_id: str) -> dict:
    client = connect(url)
    tools = [tool["name"] for tool in client.tools()]
    meta = {"requestId": request_id, "taskId": task_id, "hostData": {"check": "accept"}}
    result = client.call(
        "tools/call",
        {
            "name": "run",
            "arguments": {"code": CODE, "timeout": 60_000},
            "_meta": {RUN_META: meta},
        },
    )
    text = "\n".join(p.get("text", "") for p in result.get("content", []) if p.get("type") == "text")
    return {
        "tools": tools,
        "isError": bool(result.get("isError")),
        "meta": result.get("_meta", {}).get(RUN_META, {}),
        "value": result.get("structuredContent"),
        "text": text[:400],
    }


def host(socket_path: str, token_file: str, path: str) -> dict:
    token = "" if token_file == "-" else open(token_file).read().strip()
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.settimeout(20)
    try:
        connection.connect(socket_path)
    except OSError as error:
        return {"connect": error.__class__.__name__, "errno": error.errno}
    auth = f"Authorization: Bearer {token}\r\n" if token else ""
    connection.sendall(f"GET {path} HTTP/1.1\r\nHost: neo\r\n{auth}Connection: close\r\n\r\n".encode())
    reply = b""
    while chunk := connection.recv(65536):
        reply += chunk
    connection.close()
    head, _, body = reply.partition(b"\r\n\r\n")
    status = int(head.split(b" ", 2)[1])
    if b"transfer-encoding: chunked" in head.lower():
        body = dechunk(body)
    try:
        parsed = json.loads(body)
    except ValueError:
        parsed = body.decode(errors="replace")[:400]
    return {"connect": "ok", "status": status, "body": parsed}


def dechunk(body: bytes) -> bytes:
    out = b""
    while body:
        size, _, rest = body.partition(b"\r\n")
        length = int(size, 16)
        if length == 0:
            break
        out += rest[:length]
        body = rest[length + 2 :]
    return out


def listeners() -> dict:
    """Listening sockets as "address:port", from /proc (the image has no ss)."""
    found = set()
    for table, width in (("/proc/net/tcp", 8), ("/proc/net/tcp6", 32)):
        try:
            rows = open(table).read().splitlines()[1:]
        except OSError:
            continue
        for row in rows:
            fields = row.split()
            if fields[3] != "0A":  # TCP_LISTEN
                continue
            address, port = fields[1].split(":")
            found.add(f"{decode_address(address, width)}:{int(port, 16)}")
    return {"listening": sorted(found)}


def decode_address(hex_address: str, width: int) -> str:
    raw = bytes.fromhex(hex_address)
    if width == 8:
        return socket.inet_ntop(socket.AF_INET, raw[::-1])
    words = b"".join(raw[i : i + 4][::-1] for i in range(0, 16, 4))
    return socket.inet_ntop(socket.AF_INET6, words)


def probe(host_name: str, port: int, scheme: str) -> dict:
    """What a peer outside the container's loopback gets from the browser's MCP proxy."""
    try:
        raw = socket.create_connection((host_name, port), timeout=10)
    except TimeoutError:
        return {"tcp": "timeout"}
    except OSError as error:
        return {"tcp": "refused", "error": str(error)}
    try:
        if scheme == "https":
            context = ssl.create_default_context()
            context.check_hostname = False
            context.verify_mode = ssl.CERT_NONE
            raw = context.wrap_socket(raw, server_hostname=host_name)
        raw.sendall(f"GET /system/ready HTTP/1.1\r\nHost: {host_name}\r\nConnection: close\r\n\r\n".encode())
        reply = raw.recv(4096)
    except TimeoutError:
        # Neither an answer nor a close: a firewall drop, or a hung listener.
        return {"tcp": "accepted", "http": "timeout"}
    except (OSError, ssl.SSLError) as error:
        return {"tcp": "accepted", "http": "closed", "error": error.__class__.__name__}
    finally:
        raw.close()
    if not reply:
        return {"tcp": "accepted", "http": "closed"}
    return {"tcp": "accepted", "http": reply.split(b"\r\n", 1)[0].decode(errors="replace")}


def main() -> None:
    command, args = sys.argv[1], sys.argv[2:]
    if command == "run":
        out = run(*args)
    elif command == "host":
        out = host(args[0], args[1], urllib.parse.quote(args[2], safe="/?=&"))
    elif command == "describe":
        out = describe(args[0])
    elif command == "listeners":
        out = listeners()
    elif command == "probe":
        out = probe(args[0], int(args[1]), args[2])
    else:
        raise SystemExit(f"unknown command {command}")
    print(json.dumps(out))


if __name__ == "__main__":
    main()
