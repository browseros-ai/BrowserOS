"""Exercise the release verifier through a stateful JSON/SSE MCP peer."""

import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import captcha_smoke


@contextlib.contextmanager
def server(tools=None, sse=False, remote_error=False, paginated=False):
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def reply(self, status, data, stream=False):
            self.send_response(status)
            self.send_header("Content-Type", "text/event-stream" if stream else "application/json")
            self.send_header("mcp-session-id", "test-session")
            self.end_headers()
            body = json.dumps(data)
            self.wfile.write((f"data: \n\ndata: {body}\n\n" if stream else body).encode())

        def do_GET(self):
            requests.append((self.path, self.headers.get("Authorization")))
            self.reply(200, {"status": "ok"})

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            requests.append((body["method"], self.headers.get("mcp-session-id")))
            if remote_error:
                self.reply(200, {"id": body.get("id"), "error": {"message": "fake-private-remote-value"}})
                return
            result = {"tools": [{"name": name} for name in tools or []]}
            if body["method"] == "initialize":
                result = {"serverInfo": {"name": "test"}}
            elif paginated and not body.get("params", {}).get("cursor"):
                result = {"tools": [], "nextCursor": "page-two"}
            self.reply(200, {"id": body.get("id"), "result": result}, sse)

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{httpd.server_port}", requests
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join()


class CaptchaSmokeTests(unittest.TestCase):
    def tests_that_hosted_tool_checks_work_with_json_and_sse_sessions(self):
        for sse in [False, True]:
            for hosted in [False, True]:
                for keyed in [False, True]:
                    tools = ["tabs", "solve_captcha"] if hosted and keyed else ["tabs"]
                    with self.subTest(sse=sse, hosted=hosted, keyed=keyed):
                        with server(tools=tools, sse=sse) as (url, requests):
                            with contextlib.redirect_stdout(io.StringIO()) as output:
                                captcha_smoke.verify(url, hosted, keyed)
                            self.assertEqual(json.loads(output.getvalue().splitlines()[1]), sorted(tools))
                            self.assertIn(("tools/list", "test-session"), requests)

    def tests_that_captcha_can_be_on_a_later_mcp_page(self):
        with server(tools=["tabs", "solve_captcha"], paginated=True) as (url, _):
            with contextlib.redirect_stdout(io.StringIO()):
                captcha_smoke.verify(url, True)

    def tests_that_wrong_tool_availability_fails_verification(self):
        for hosted, tools in [(True, ["tabs"]), (False, ["tabs", "solve_captcha"]), (False, [])]:
            with self.subTest(hosted=hosted, tools=tools), server(tools=tools) as (url, _):
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(RuntimeError):
                    captcha_smoke.verify(url, hosted)

    def tests_that_remote_error_bodies_never_reach_public_logs(self):
        with server(remote_error=True) as (url, _):
            result = subprocess.run([sys.executable, str(Path(captcha_smoke.__file__)), url, "1", "1"],
                                    capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("fake-private-remote-value", result.stdout + result.stderr)
        self.assertEqual(result.stderr.strip(), "Docker CAPTCHA verification failed")


if __name__ == "__main__":
    unittest.main()
