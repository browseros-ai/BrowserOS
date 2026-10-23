import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import release


@contextlib.contextmanager
def server(token=200, tags=403, tools=None, sse=False, bad_token=False, remote_error=False):
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
            if self.path.startswith("/token?"):
                self.reply(token, {} if bad_token else {"token": "anonymous-test-token"})
            elif self.path.endswith("/tags/list"):
                self.reply(tags, {"tags": ["bootstrap"]})
            else:
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


class ReleaseTests(unittest.TestCase):
    def tests_that_only_explicit_anonymous_denial_allows_private_push(self):
        for token, tags in [(200, 401), (200, 403), (401, 200), (403, 200)]:
            with self.subTest(token=token, tags=tags), server(token, tags) as (url, requests):
                release.require_private(url)
                self.assertIsNone(requests[0][1])
                if len(requests) > 1:
                    self.assertEqual(requests[1][1], "Bearer anonymous-test-token")

    def tests_that_public_or_inconclusive_registry_responses_block_push(self):
        for token, tags, bad in [(200, 200, False), (500, 403, False),
                                 (200, 404, False), (200, 500, False), (200, 403, True)]:
            with self.subTest(token=token, tags=tags, bad=bad):
                with server(token, tags, bad_token=bad) as (url, _):
                    with self.assertRaises(RuntimeError):
                        release.require_private(url)

    def tests_that_hosted_tool_checks_work_with_json_and_sse_sessions(self):
        for sse in [False, True]:
            for hosted in [False, True]:
                tools = ["tabs", "solve_captcha"] if hosted else ["tabs"]
                with self.subTest(sse=sse, hosted=hosted):
                    with server(tools=tools, sse=sse) as (url, requests):
                        with contextlib.redirect_stdout(io.StringIO()) as output:
                            release.verify(url, hosted)
                        self.assertEqual(json.loads(output.getvalue().splitlines()[1]), sorted(tools))
                        self.assertIn(("tools/list", "test-session"), requests)

    def tests_that_wrong_tool_availability_fails_verification(self):
        for hosted, tools in [(True, ["tabs"]), (False, ["tabs", "solve_captcha"]), (False, [])]:
            with self.subTest(hosted=hosted, tools=tools), server(tools=tools) as (url, _):
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(RuntimeError):
                    release.verify(url, hosted)

    def tests_that_remote_error_bodies_never_reach_public_logs(self):
        with server(remote_error=True) as (url, _):
            result = subprocess.run([sys.executable, str(Path(release.__file__)), "verify", url, "1"],
                                    capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("fake-private-remote-value", result.stdout + result.stderr)
        self.assertEqual(result.stderr.strip(), "Release verification failed")


if __name__ == "__main__":
    unittest.main()
