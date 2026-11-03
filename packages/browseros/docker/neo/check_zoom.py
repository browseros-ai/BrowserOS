#!/usr/bin/env python3
"""Check inside a running Neo container that the viewer cannot zoom Neo's pages.

noVNC sends a two-finger pinch as Ctrl + mouse wheel, and keyboard zoom arrives
as Ctrl +/-/0. This sends both to the VNC server the way the viewer does and
reads the page's zoom over the loopback CDP endpoint. Ctrl + 0 only changes a
page that is not at 100%, so it is sent to a zoomed-out mobile preview, the
DevTools emulation setViewport uses. Keys without Ctrl must still type and a
plain mouse wheel must still scroll, which shows the input reached the page.
The release workflow streams this script to the container:

  docker exec -i <container> python3 - < check_zoom.py

Standard library only, like neo-quit.py: the image has no VNC or WebSocket client.
"""

import base64
import contextlib
import json
import os
import socket
import struct
import sys
import time
import urllib.parse
import urllib.request

CDP = "http://127.0.0.1:9110"
VNC = ("127.0.0.1", 5900)
# Long enough to scroll; the field takes the typing check's keys.
PAGE = (
    "data:text/html,<title>zoom check</title>"
    "<body style='height:5000px'><input id='typed'></body>"
)

# X keysyms, and the VNC pointer button bits of the wheel (buttons 4 and 5).
CONTROL = 0xFFE3
EQUAL, MINUS, ZERO = 0x3D, 0x2D, 0x30
WHEEL_UP, WHEEL_DOWN = 0x08, 0x10


class Vnc:
    """The input half of an RFB 3.8 client: keys and the pointer."""

    def __init__(self) -> None:
        self.sock = socket.create_connection(VNC, timeout=5)
        if not self._read(12).startswith(b"RFB 003."):
            raise RuntimeError("not a VNC server")
        self.sock.sendall(b"RFB 003.008\n")
        if 1 not in self._read(self._read(1)[0]):
            raise RuntimeError("VNC server wants a password")
        self.sock.sendall(b"\x01")
        if struct.unpack(">I", self._read(4))[0] != 0:
            raise RuntimeError("VNC security handshake failed")
        # Shared, so a connected viewer stays connected.
        self.sock.sendall(b"\x01")
        width, height = struct.unpack(">HH", self._read(4))
        self._read(16)
        self._read(struct.unpack(">I", self._read(4))[0])
        # Below the tab strip and toolbar, over the page.
        self.center = (width // 2, height // 2)

    def _read(self, size: int) -> bytes:
        data = b""
        while len(data) < size:
            chunk = self.sock.recv(size - len(data))
            if not chunk:
                raise RuntimeError("VNC server closed the connection")
            data += chunk
        return data

    def key(self, keysym: int, down: bool) -> None:
        self.sock.sendall(struct.pack(">BBxxI", 4, down, keysym))

    def tap(self, keysym: int, control: bool = False) -> None:
        if control:
            self.key(CONTROL, True)
        self.key(keysym, True)
        self.key(keysym, False)
        if control:
            self.key(CONTROL, False)

    def pointer(self, buttons: int = 0) -> None:
        self.sock.sendall(struct.pack(">BBHH", 5, buttons, *self.center))

    def wheel(self, button: int, clicks: int, control: bool = False) -> None:
        """Wheel clicks over the page; with control, noVNC's pinch gesture."""
        self.pointer()
        if control:
            self.key(CONTROL, True)
        for _ in range(clicks):
            self.pointer(button)
            self.pointer()
        if control:
            self.key(CONTROL, False)

    def close(self) -> None:
        self.sock.close()


def cdp(path: str, method: str = "GET") -> dict:
    request = urllib.request.Request(f"{CDP}{path}", method=method)
    with urllib.request.urlopen(request, timeout=5) as response:
        body = response.read()
    return json.loads(body) if body.strip().startswith(b"{") else {}


class Page:
    """One DevTools session on the check page, over a minimal WebSocket client.

    It stays open for the whole check, since closing it would also end its
    device emulation.
    """

    def __init__(self, target: dict) -> None:
        url = urllib.parse.urlparse(target["webSocketDebuggerUrl"])
        self.sock = socket.create_connection((url.hostname, url.port), timeout=5)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            (
                f"GET {url.path} HTTP/1.1\r\nHost: {url.netloc}\r\n"
                "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            ).encode()
        )
        self.reader = self.sock.makefile("rb")
        if b" 101 " not in self.reader.readline():
            raise RuntimeError("CDP refused the WebSocket upgrade")
        while self.reader.readline() not in (b"\r\n", b""):
            pass
        self.last_id = 0

    def call(self, method: str, params: dict | None = None) -> dict:
        self.last_id += 1
        payload = json.dumps(
            {"id": self.last_id, "method": method, "params": params or {}}
        ).encode()
        # Clients mask every frame (RFC 6455).
        mask = os.urandom(4)
        if len(payload) < 126:
            header = bytes([0x81, 0x80 | len(payload)])
        else:
            header = bytes([0x81, 0x80 | 126]) + struct.pack(">H", len(payload))
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self.sock.sendall(header + mask + masked)
        while True:
            first, second = self.reader.read(2)
            size = second & 0x7F
            if size == 126:
                size = struct.unpack(">H", self.reader.read(2))[0]
            elif size == 127:
                size = struct.unpack(">Q", self.reader.read(8))[0]
            message = self.reader.read(size)
            if first & 0x0F != 0x1 or (reply := json.loads(message)).get("id") != self.last_id:
                continue
            if "error" in reply:
                raise RuntimeError(f"{method} failed: {message[:200]!r}")
            return reply["result"]

    def evaluate(self, expression: str):
        result = self.call("Runtime.evaluate", {"expression": expression, "returnByValue": True})
        if "exceptionDetails" in result:
            raise RuntimeError(f"evaluate failed: {expression}")
        return result["result"].get("value")

    def state(self) -> dict:
        """Browser zoom shows in devicePixelRatio, pinch zoom in the visual viewport."""
        return self.evaluate(
            "({zoom: devicePixelRatio, pinch: visualViewport.scale, scrollY,"
            " typed: document.getElementById('typed').value})"
        )

    def close(self) -> None:
        self.sock.close()


def report(label: str, passed: bool, detail: str) -> bool:
    print(f"{'ok  ' if passed else 'FAIL'} {label} ({detail})")
    return passed


def run(vnc: Vnc, page: Page, target_id: str) -> bool:
    cdp(f"/json/activate/{target_id}")
    deadline = time.monotonic() + 10
    while page.evaluate("document.readyState") != "complete":
        if time.monotonic() > deadline:
            raise RuntimeError("the check page did not load")
        time.sleep(0.2)
    page.evaluate("document.getElementById('typed').focus()")
    # Lets the activated tab take keyboard focus before input arrives.
    time.sleep(1)

    passed = True
    before = page.state()
    for label, send in (
        ("a viewer's pinch (Ctrl + wheel) does not zoom", lambda: vnc.wheel(WHEEL_UP, 3, control=True)),
        ("Ctrl + = does not zoom", lambda: vnc.tap(EQUAL, control=True)),
        ("Ctrl + - does not zoom", lambda: vnc.tap(MINUS, control=True)),
    ):
        send()
        time.sleep(1)
        after = page.state()
        detail = f"zoom {before['zoom']} -> {after['zoom']}, pinch {before['pinch']} -> {after['pinch']}"
        unchanged = (after["zoom"], after["pinch"]) == (before["zoom"], before["pinch"])
        passed &= report(label, unchanged, detail)
        before = after

    for keysym in (EQUAL, MINUS, ZERO):
        vnc.tap(keysym)
    vnc.wheel(WHEEL_DOWN, 3)
    time.sleep(1)
    after = page.state()
    passed &= report("keys without Ctrl still type", after["typed"] == "=-0", f"typed {after['typed']!r}")
    passed &= report(
        "a plain wheel still scrolls",
        after["scrollY"] > before["scrollY"],
        f"scrollY {before['scrollY']} -> {after['scrollY']}",
    )

    # Without a viewport tag, a phone-sized preview lays the page out 980 px
    # wide and shows it zoomed out to fit, which Chromium's Ctrl + 0 undoes.
    page.call(
        "Emulation.setDeviceMetricsOverride",
        {"width": 400, "height": 906, "deviceScaleFactor": 2, "mobile": True},
    )
    deadline = time.monotonic() + 5
    while (before := page.state())["pinch"] >= 1 and time.monotonic() < deadline:
        time.sleep(0.2)
    vnc.tap(ZERO, control=True)
    time.sleep(1)
    after = page.state()
    page.call("Emulation.clearDeviceMetricsOverride")
    passed &= report(
        "Ctrl + 0 keeps a zoomed-out mobile preview",
        before["pinch"] < 1 and after["pinch"] == before["pinch"],
        f"pinch {before['pinch']} -> {after['pinch']}",
    )
    return passed


def main() -> int:
    # Chromium opens pages only for a PUT to /json/new.
    target = cdp("/json/new?" + urllib.parse.quote(PAGE, safe=""), method="PUT")
    with contextlib.ExitStack() as cleanup:
        cleanup.callback(cdp, f"/json/close/{target['id']}")
        page = Page(target)
        cleanup.callback(page.close)
        vnc = Vnc()
        cleanup.callback(vnc.close)
        return 0 if run(vnc, page, target["id"]) else 1


if __name__ == "__main__":
    sys.exit(main())
