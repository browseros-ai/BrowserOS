"""Run smoke_sdk.py the way release verify does, against a fake stateless claw-sdk."""

import contextlib
import dataclasses
import json
import re
import subprocess
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

SMOKE = Path(__file__).with_name("smoke_sdk.py")
TITLE = re.compile(r"<title>(.*?)</title>")

# How the fake's run answers, given the value a real run would return and the submitted code.
RUN_REPLIES = {
    "text": lambda value, code: {"content": [{"type": "text", "text": value}], "isError": False},
    # run's result shape is not pinned, so the smoke must also find the value here.
    "structured": lambda value, code: {"content": [], "structuredContent": {"value": value}, "isError": False},
    # Carries the right value, so only isError can fail it.
    "error": lambda value, code: {
        "content": [{"type": "text", "text": f"{value}\nneo / browser_disconnected"}], "isError": True,
    },
    # Not an error, and the code holds only the lower-case title.
    "echo": lambda value, code: {"content": [{"type": "text", "text": code}], "isError": False},
}


@dataclasses.dataclass
class Peer:
    """A stateless MCP server that behaves like claw-sdk unless a field says otherwise.

    Its run answers as claw-sdk would for the smoke's script: the title of the data: URL the
    script visits, upper-cased when the script upper-cases it. `runs` collects the code of
    every run it received.
    """

    sse: bool = False
    statuses: dict = dataclasses.field(default_factory=lambda: {"/system/health": 200, "/system/ready": 200})
    session_id: str | None = None
    tool_pages: list = dataclasses.field(default_factory=lambda: [["run"]])
    code_param: str = "code"
    reply: str = "text"  # a RUN_REPLIES key
    runs: list = dataclasses.field(default_factory=list)


@contextlib.contextmanager
def serve(peer: Peer):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            self.send_response(peer.statuses.get(self.path, 404))
            self.end_headers()

        def do_POST(self):
            message = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            if "id" not in message:  # a notification
                self.send_response(202)
                self.end_headers()
                return
            body = json.dumps({"jsonrpc": "2.0", "id": message["id"], **self.answer(message["method"], message["params"])})
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream" if peer.sse else "application/json")
            if peer.session_id:
                self.send_header("mcp-session-id", peer.session_id)
            self.end_headers()
            self.wfile.write((f"data: {body}\n\n" if peer.sse else body).encode())

        def answer(self, method: str, params: dict) -> dict:
            if method == "initialize":
                return {"result": {
                    "protocolVersion": params["protocolVersion"], "capabilities": {"tools": {}},
                    "serverInfo": {"name": "claw-sdk", "version": "0.0.0"},
                }}
            if method == "tools/list":
                index = int(params.get("cursor") or 0)
                page = {"tools": [self.tool(name) for name in peer.tool_pages[index]]}
                if index + 1 < len(peer.tool_pages):
                    page["nextCursor"] = str(index + 1)
                return {"result": page}
            arguments = params.get("arguments", {})
            # run takes { code, timeout? } and nothing else (sdk-skeleton D5).
            if method != "tools/call" or params.get("name") != "run" or set(arguments) - {"code", "timeout"}:
                return {"error": {"code": -32602, "message": f"unexpected {method}: {params}"}}
            code = arguments["code"]
            peer.runs.append(code)
            title = TITLE.search(code).group(1)
            return {"result": RUN_REPLIES[peer.reply](title.upper() if ".toUpperCase()" in code else title, code)}

        @staticmethod
        def tool(name: str) -> dict:
            if name != "run":
                return {"name": name, "inputSchema": {"type": "object"}}
            return {"name": "run", "inputSchema": {
                "type": "object",
                "properties": {peer.code_param: {"type": "string"}, "timeout": {"type": "integer"}},
                "required": [peer.code_param],
            }}

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    # shutdown() waits up to one poll interval, 0.5 s by default, for every test's server.
    thread = threading.Thread(target=httpd.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
    thread.start()
    try:
        yield httpd.server_port
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join()


class SmokeSdkTests(unittest.TestCase):
    def smoke(self, peer: Peer) -> subprocess.CompletedProcess:
        with serve(peer) as port:
            return subprocess.run([sys.executable, str(SMOKE), "--mcp-port", str(port)],
                                  capture_output=True, text=True, timeout=60)

    def fail_line(self, peer: Peer) -> str:
        """Requires the smoke to stop on a FAIL line, not a crash, and returns that line."""
        result = self.smoke(peer)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        line = result.stdout.splitlines()[-1]
        self.assertTrue(line.startswith("FAIL"), result.stdout)
        return line

    def test_passes_against_a_stateless_claw_sdk_over_json_and_sse(self):
        for sse in (False, True):
            for reply in ("text", "structured"):
                with self.subTest(sse=sse, reply=reply):
                    result = self.smoke(Peer(sse=sse, reply=reply))
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    lines = result.stdout.splitlines()
                    self.assertTrue(lines and all(line.startswith("ok") for line in lines), result.stdout)

    def test_fails_until_health_and_ready_answer_200(self):
        for path, status in (("/system/ready", 503), ("/system/health", 404)):
            with self.subTest(path=path):
                peer = Peer()
                peer.statuses[path] = status
                self.assertIn(str(status), self.fail_line(peer))

    def test_fails_when_initialize_issues_a_session_id(self):
        self.assertIn("claw-server-session", self.fail_line(Peer(session_id="claw-server-session")))

    def test_fails_on_any_tool_besides_run_on_any_page(self):
        for pages in ([["run", "tabs"]], [["run"], ["tabs"]]):
            with self.subTest(pages=pages):
                self.assertIn("tabs", self.fail_line(Peer(tool_pages=pages)))

    def test_fails_when_run_takes_no_code(self):
        self.assertIn("script", self.fail_line(Peer(code_param="script")))

    def test_fails_when_the_run_reports_an_error(self):
        self.assertIn("browser_disconnected", self.fail_line(Peer(reply="error")))

    def test_fails_when_the_result_only_echoes_the_code(self):
        peer = Peer(reply="echo")
        line = self.fail_line(peer)
        (code,) = peer.runs
        self.assertIn(TITLE.search(code).group(1), line)


if __name__ == "__main__":
    unittest.main()
