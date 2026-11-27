#!/usr/bin/env python3
"""Boot a packaged server in isolation and exercise health and MCP initialization."""

import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


def main() -> None:
    artifact = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="claw-linux-smoke-") as directory:
        root = Path(directory)
        config = root / "config.json"
        config.write_text(
            json.dumps(
                {
                    "ports": {"server": 19200, "cdp": 19300},
                    "directories": {"resources": str(artifact / "resources")},
                    "flags": {"devMode": True},
                }
            )
        )
        env = dict(os.environ, BROWSERCLAW_DIR=str(root / "state"))
        binary = artifact / "resources/bin/browseros-claw-server"
        with (root / "server.log").open("w+") as log:
            server = subprocess.Popen(
                [str(binary), "--config", str(config)], env=env, stdout=log, stderr=log
            )
            try:
                deadline = time.monotonic() + 30
                while True:
                    if server.poll() is not None:
                        raise RuntimeError(
                            f"Server exited with code {server.returncode}"
                        )
                    try:
                        with urllib.request.urlopen(
                            "http://127.0.0.1:19200/system/health", timeout=1
                        ) as response:
                            assert response.status == 200
                        break
                    except (urllib.error.URLError, TimeoutError):
                        if time.monotonic() >= deadline:
                            raise RuntimeError(
                                "Server did not become healthy within 30 seconds"
                            ) from None
                        time.sleep(0.1)
                payload = {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-03-26",
                        "capabilities": {},
                        "clientInfo": {"name": "linux-release-smoke", "version": "1"},
                    },
                }
                request = urllib.request.Request(
                    "http://127.0.0.1:19200/mcp",
                    data=json.dumps(payload).encode(),
                    headers={
                        "Content-Type": "application/json",
                        "Accept": "application/json, text/event-stream",
                    },
                )
                with urllib.request.urlopen(request, timeout=10) as response:
                    body = response.read().decode()
                    if response.headers.get_content_type() == "text/event-stream":
                        body = "\n".join(
                            line[5:].lstrip()
                            for line in body.splitlines()
                            if line.startswith("data:")
                        )
                    reply = json.loads(body)
                    assert reply["id"] == 1 and "serverInfo" in reply["result"], reply
                print("Packaged server booted; health and MCP initialize passed")
            except BaseException:
                log.seek(0)
                sys.stderr.write(log.read())
                raise
            finally:
                server.terminate()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == "__main__":
    main()
