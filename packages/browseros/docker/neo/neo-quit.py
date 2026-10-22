#!/usr/bin/env python3
"""Ask Neo to quit cleanly over its loopback CDP endpoint (Browser.close).

Chromium's SIGTERM path on Linux exits without recording a clean shutdown, so
the next start shows "Restore pages? Chromium didn't shut down correctly" and
the last cookie writes can be lost. Browser.close is the same path as quitting
from the menu. The entrypoint runs this first and falls back to SIGTERM.

Even after a clean quit, Neo on Linux still records the session as crashed, so
the entrypoint also passes --hide-crash-restore-bubble; this script is about
data, that switch about the prompt.

  neo-quit <cdp-port>

Standard library only: the image has python3 for websockify but no WebSocket
client.
"""

import base64
import json
import os
import socket
import sys
import urllib.request
from urllib.parse import urlparse


def main() -> int:
    port = int(sys.argv[1])
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=3) as response:
        target = urlparse(json.load(response)["webSocketDebuggerUrl"])

    conn = socket.create_connection((target.hostname, target.port), timeout=5)
    key = base64.b64encode(os.urandom(16)).decode()
    conn.sendall(
        (
            f"GET {target.path} HTTP/1.1\r\nHost: {target.netloc}\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        ).encode()
    )
    if b" 101 " not in conn.recv(4096).split(b"\r\n", 1)[0]:
        return 1

    # One masked text frame, as RFC 6455 requires of clients. The payload is
    # shorter than 126 bytes, so the length fits in the second header byte.
    payload = json.dumps({"id": 1, "method": "Browser.close"}).encode()
    mask = os.urandom(4)
    frame = bytes([0x81, 0x80 | len(payload)]) + mask
    frame += bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
    conn.sendall(frame)
    conn.close()
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, KeyError, ValueError, IndexError):
        sys.exit(1)
