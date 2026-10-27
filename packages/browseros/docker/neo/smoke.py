#!/usr/bin/env python3
"""End-to-end check of a running BrowserOS neo container, from the host.

It talks to the container the way a developer does: through the published
viewer and MCP ports. Passing means the noVNC viewer is served, an agent
outside the container can reach Neo's MCP server (which proves the
allow_remote_in_mcp setting works through Docker's port forwarding), agents get
the hosted tool set (BROWSERCLAW_HOSTED_MODE=1), an agent-opened tab loads
and, in hosted mode, stays out of any tab group (the shipped neo.toml keeps the
explicit grouping default), a new tab page is blank (the entrypoint's
--disable-browseros-features list reached the browser). WebGL1/2 must render a
triangle on the container's software GPU (NEO_SOFTWARE_GRAPHICS reached the
browser), and in hosted mode pages must see the hosted GPU identity, not that
software renderer.

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
    check_graphics(client, int(re.search(r"opened page (\d+)", opened).group(1)), hosted)
    if hosted:
        check_ungrouped(client, int(re.search(r"opened page (\d+)", opened).group(1)))
    check_new_tab_blank(client)


def evaluate(client: McpClient, page: int, code: str) -> str:
    """Runs `code` in the page and returns its one-line string result.

    The value is the one line between the reply's [UNTRUSTED...] markers and its tip.
    """
    reply = client.tool("evaluate", {"page": page, "code": code})
    return next((line for line in reply.splitlines() if line and not line.startswith(("[", "Tip:"))), "")


def check_new_tab_blank(client: McpClient) -> int:
    """The entrypoint's hidden-feature list reached the browser.

    Most of the names in that list only change the browser's frame, which no tool can
    see. NewTabPage is the exception: it sends every new tab page to about:blank, where
    Chromium or the bundled extension would otherwise draw one. A browser older than
    0.51.2.5 ignores the switch, so it fails here instead of shipping the full frame.
    Returns the page, for later checks that need a blank one.
    """
    opened = client.tool("tabs", {"action": "new", "url": "chrome://newtab/"})
    page = int(re.search(r"opened page (\d+)", opened).group(1))
    # The tab keeps chrome://newtab/ as its address, so ask the page where it really is.
    href = evaluate(client, page, "return location.href")
    check("new tab page is blank", href == "about:blank", href)
    return page


# Renderer names for CPU-only drawing: Mesa's llvmpipe and softpipe, Chromium's SwiftShader.
SOFTWARE_RENDERER = re.compile(r"llvmpipe|softpipe|swiftshader", re.IGNORECASE)


def check_graphics(client: McpClient, page: int, hosted: bool) -> None:
    """WebGL draws on the container's software GPU, behind the hosted GPU identity.

    Without a GPU, Chromium draws with Mesa's llvmpipe and refuses every WebGL context
    on it unless NEO_SOFTWARE_GRAPHICS's --ignore-gpu-blocklist reached the browser. A
    context or a renderer name proves little, so both WebGL versions render a triangle
    and read it back. Hosted mode reports a desktop GPU in place of llvmpipe, and some
    Chromium switches turn that identity off (--enable-unsafe-webgpu among them), so a
    launcher that passes one fails here. Keep the workloads tiny so the release smoke
    check remains cheap on ARM VMs.
    """
    reply = client.tool("evaluate", {"page": page, "code": r"""
const result = {};
for (const kind of ['webgl', 'webgl2']) {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 4;
  const gl = canvas.getContext(kind);
  if (!gl) { result[kind] = {context: false}; continue; }
  try {
    const modern = kind === 'webgl2';
    const prefix = modern ? '#version 300 es\n' : '';
    const vertex = prefix + (modern ? 'in' : 'attribute') +
      ' vec2 position; void main() { gl_Position = vec4(position, 0., 1.); }';
    const fragment = prefix + 'precision mediump float; ' +
      (modern ? 'out vec4 color; ' : '') + 'void main() { ' +
      (modern ? 'color' : 'gl_FragColor') + ' = vec4(1., 0.25, 0., 1.); }';
    const program = gl.createProgram();
    for (const [type, source] of [[gl.VERTEX_SHADER, vertex], [gl.FRAGMENT_SHADER, fragment]]) {
      const shader = gl.createShader(type);
      gl.shaderSource(shader, source); gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw Error(gl.getShaderInfoLog(shader));
      gl.attachShader(program, shader); gl.deleteShader(shader);
    }
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw Error(gl.getProgramInfoLog(program));
    gl.useProgram(program);
    const buffer = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1, 3,-1, -1,3]), gl.STATIC_DRAW);
    const position = gl.getAttribLocation(program, 'position');
    gl.enableVertexAttribArray(position); gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    const pixels = new Uint8Array(4);
    gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
    const error = gl.getError();
    const info = gl.getExtension('WEBGL_debug_renderer_info');
    const renderer = gl.getParameter(info ? info.UNMASKED_RENDERER_WEBGL : gl.RENDERER);
    result[kind] = {context: true, pixels: Array.from(pixels), error, renderer};
    gl.deleteBuffer(buffer); gl.deleteProgram(program);
  } catch (error) { result[kind] = {context: true, error: String(error)}; }
  finally { gl.getExtension('WEBGL_lose_context')?.loseContext(); }
}
return result;
"""})
    # MCP wraps the JSON value in its untrusted-content markers and may append a
    # session tip. Decode just the first JSON object, not that presentation text.
    result, _ = json.JSONDecoder().raw_decode(reply[reply.index("{"):])
    for kind in ("webgl", "webgl2"):
        value = result[kind]
        check(f"{kind} renders and reads back", value.get("pixels") == [255, 64, 0, 255] and value.get("error") == 0, json.dumps(value))
    if hosted:
        for kind in ("webgl", "webgl2"):
            renderer = result[kind].get("renderer", "")
            check(f"hosted {kind} reports the GPU identity", not SOFTWARE_RENDERER.search(renderer), renderer)


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
