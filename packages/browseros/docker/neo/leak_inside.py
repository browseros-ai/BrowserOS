#!/usr/bin/env python3
"""The leak test's hands inside the Neo container: the fixture's web server and the oracle.

leak_sdk.py copies this file and leak_fixture/ to /tmp/leakfx and starts one process:

  docker exec -i <container> python3 -u /tmp/leakfx/leak_inside.py

  thread: HTTP on 127.0.0.1:8800 for a.test, sub.a.test and b.test (--add-host)
    GET  /fx/fx.js                      the fixture script
    GET  /fx/<instance>/<location>      page.html, for every document of every instance
                                        (a shell without fx.js for one the agent writes)
    GET  /fx/<instance>/<location>.json that document's slice (Cache-Control: no-store)
    GET  /fx/<instance>/dl              a small file for the download act
    POST /fx/<instance>/echo            the document after a submit: it shows the posted
                                        value, so only the echo remains (the nav-away case)
  main: JSON lines on stdin/stdout, one request at a time, each answered by its id
    put      an instance's slices: the only way values travel, and never in a URL
    plant_file  a value written into a file on Neo's disk, for setInputFiles to upload
    prepare_upload  a deny-list probe's target checked and its links made: paths only
    read_back   whether a page's input read a file: sha256s compared here, never sent
    digest_control  read_back's positive control, on a file the oracle sets itself
    state    each of the instance's tabs' __fx.state(): digests per region, never values,
             and the ids of the focused elements (where keyboard input lands)
    act      __fx.act(name), with a user gesture when asked
    focus    __fx.focus(region): the owner's click, through the iframe chain, selecting
             the field's text when asked
    targets  Target.getTargets: which documents really are out-of-process frames
    front    Browser.getTabs' active tabs: what is in front of Neo's one screen
    clock    the container's Unix ms, for the host-to-Neo clock offset
    grep     needles over a directory tree (the profile scan): indexes, never needles

The oracle is a second DevTools client on Chromium's CDP port (9110, set by the
entrypoint). It only evaluates in the main world of a tab's top frame, which is the
page's own script's world: claw-sdk's sealed world, journal and SecretSet never see it,
so a proof cannot warm a cold value. Both die with the pipe, so a crashed harness leaves
nothing running. The WebSocket class is also the host's transport for /v1/display.
Standard library only (the image's python3).
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent
FIXTURE = ROOT / "leak_fixture"
CDP_PORT = 9110
FIXTURE_PORT = 8800
SLICE_MARK = b"<!--FX-SLICE-->"
FX_SCRIPT = b'<script src="/fx/fx.js"></script>'


class WebSocket:
    """A minimal RFC 6455 client: masked frames out, fragments joined, pings answered.

    `recv` returns (opcode, payload) for text and binary messages and None once the peer
    closed; `close_code` and `close_reason` then say why. Sends hold a lock, so one thread
    may read while another writes.
    """

    def __init__(
        self, host: str, port: int, path: str, headers: dict[str, str] | None = None
    ) -> None:
        self.sock = socket.create_connection((host, port), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode()
        lines = [
            f"GET {path} HTTP/1.1",
            f"Host: {host}:{port}",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Key: {key}",
            "Sec-WebSocket-Version: 13",
            *(f"{k}: {v}" for k, v in (headers or {}).items()),
        ]
        self.sock.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.sock.recv(1)
            if not chunk:
                break
            head += chunk
        self.status = int(head.split(b" ", 2)[1]) if head.startswith(b"HTTP/") else 0
        self.response = head.decode(errors="replace")
        self.close_code: int | None = None
        self.close_reason = ""
        self._lock = threading.Lock()
        self.sock.settimeout(None)

    @property
    def open(self) -> bool:
        return self.status == 101

    def read_rest(self, limit: int = 65536) -> bytes:
        """The body of a refused upgrade (a JSON error), as far as it arrives."""
        self.sock.settimeout(2)
        data = b""
        try:
            while len(data) < limit and (chunk := self.sock.recv(4096)):
                data += chunk
        except OSError:
            pass
        return data

    def _exact(self, n: int) -> bytes:
        out = b""
        while len(out) < n:
            chunk = self.sock.recv(n - len(out))
            if not chunk:
                raise EOFError("connection closed")
            out += chunk
        return out

    def _frame(self) -> tuple[bool, int, bytes]:
        b0, b1 = self._exact(2)
        n = b1 & 0x7F
        if n == 126:
            n = struct.unpack(">H", self._exact(2))[0]
        elif n == 127:
            n = struct.unpack(">Q", self._exact(8))[0]
        mask = self._exact(4) if b1 & 0x80 else b""
        payload = self._exact(n)
        if mask:
            payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        return bool(b0 & 0x80), b0 & 0x0F, payload

    def recv(self) -> tuple[int, bytes] | None:
        parts: list[bytes] = []
        opcode = 0
        while True:
            try:
                fin, op, payload = self._frame()
            except (EOFError, OSError):
                return None
            if op == 8:
                if len(payload) >= 2:
                    self.close_code = struct.unpack(">H", payload[:2])[0]
                    self.close_reason = payload[2:].decode(errors="replace")
                return None
            if op == 9:
                self.send(payload, opcode=10)
                continue
            if op == 10:
                continue
            if op != 0:
                opcode = op
            parts.append(payload)
            if fin:
                return opcode, b"".join(parts)

    def send(self, data: bytes, opcode: int = 2) -> None:
        mask = os.urandom(4)
        n = len(data)
        if n < 126:
            head = bytes([0x80 | opcode, 0x80 | n])
        elif n < 65536:
            head = bytes([0x80 | opcode, 0x80 | 126]) + struct.pack(">H", n)
        else:
            head = bytes([0x80 | opcode, 0x80 | 127]) + struct.pack(">Q", n)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        with self._lock:
            self.sock.sendall(head + mask + masked)

    def close(self) -> None:
        try:
            self.send(struct.pack(">H", 1000), opcode=8)
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass


class Cdp:
    """A DevTools client on the browser endpoint with flat sessions. One reader thread
    routes replies by id; events are dropped, because the oracle only asks."""

    def __init__(self, port: int = CDP_PORT) -> None:
        with urllib.request.urlopen(
            f"http://127.0.0.1:{port}/json/version", timeout=10
        ) as r:
            url = urllib.parse.urlsplit(json.load(r)["webSocketDebuggerUrl"])
        self.ws = WebSocket(url.hostname, url.port, url.path)
        if not self.ws.open:
            raise RuntimeError(f"CDP refused: {self.ws.response}")
        self._next = 0
        self._waiting: dict[int, list] = {}
        self._cv = threading.Condition()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        while (message := self.ws.recv()) is not None:
            reply = json.loads(message[1])
            if "id" in reply:
                with self._cv:
                    self._waiting[reply["id"]] = reply
                    self._cv.notify_all()
        with self._cv:
            self._waiting[-1] = {"error": {"message": "CDP connection closed"}}
            self._cv.notify_all()

    def call(
        self,
        method: str,
        params: dict | None = None,
        session: str | None = None,
        timeout: float = 10,
    ) -> dict:
        with self._cv:
            self._next += 1
            ident = self._next
        message = {"id": ident, "method": method, "params": params or {}}
        if session:
            message["sessionId"] = session
        self.ws.send(json.dumps(message).encode(), opcode=1)
        deadline = time.monotonic() + timeout
        with self._cv:
            while ident not in self._waiting and -1 not in self._waiting:
                left = deadline - time.monotonic()
                if left <= 0:
                    raise TimeoutError(f"{method}: no answer in {timeout} s")
                self._cv.wait(left)
            reply = self._waiting.pop(ident, None) or self._waiting[-1]
        if "error" in reply:
            raise RuntimeError(f"{method}: {reply['error'].get('message')}")
        return reply["result"]


class Fixture:
    """What the HTTP thread serves. `put` replaces an instance's whole entry at once, so a
    handler thread sees an instance's slices either all or not at all."""

    def __init__(self) -> None:
        self.instances: dict[str, dict] = {}

    def page(self, inline: dict | None = None, *, shell: bool = False) -> bytes:
        html = (FIXTURE / "page.html").read_bytes()
        if shell:
            # A document the agent writes with setContent starts without the fixture's
            # script: document.open keeps the window's realm, so the script the agent's
            # HTML loads would otherwise run a second time and redeclare its globals.
            return html.replace(FX_SCRIPT, b"")
        if inline is None:
            return html
        data = json.dumps(inline).replace("<", "\\u003c").encode()
        tag = b'<script type="application/json" id="fx-slice">' + data + b"</script>"
        return html.replace(SLICE_MARK, tag)


FIX = Fixture()


class FixtureHandler(BaseHTTPRequestHandler):
    """Serves page.html for every document and each document's slice. Logs nothing: a
    request log is not a Neo channel, but it would sit beside them in the container."""

    def log_message(self, *_: object) -> None:
        pass

    def _send(self, status: int, body: bytes, kind: str = "text/html") -> None:
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        parts = urllib.parse.urlsplit(self.path).path.strip("/").split("/")
        if parts == ["fx", "fx.js"]:
            return self._send(200, (FIXTURE / "fx.js").read_bytes(), "text/javascript")
        if len(parts) == 3 and parts[0] == "fx" and parts[1] in FIX.instances:
            entry = FIX.instances[parts[1]]
            if parts[2] == "dl":
                return self._send(200, b"neutral\n", "text/plain")
            if parts[2].endswith(".json") and parts[2][:-5] in entry["slices"]:
                body = json.dumps(entry["slices"][parts[2][:-5]]).encode()
                return self._send(200, body, "application/json")
            if parts[2] in entry["slices"]:
                written = entry["slices"][parts[2]].get("written", False)
                return self._send(200, FIX.page(shell=written))
        self._send(404, b"not found", "text/plain")

    def do_POST(self) -> None:
        # The submit of a navigating field: form fields `region` (an id) and `value`. Never a
        # GET form or a redirect carrying the value: either would put it in a URL.
        parts = urllib.parse.urlsplit(self.path).path.strip("/").split("/")
        length = int(self.headers.get("Content-Length") or 0)
        form = urllib.parse.parse_qs(self.rfile.read(length).decode())
        entry = FIX.instances.get(parts[1]) if len(parts) == 3 else None
        echo = entry and entry["echoes"].get((form.get("region") or [""])[0])
        if not echo or parts[2] != "echo":
            return self._send(404, b"not found", "text/plain")
        region = dict(echo["region"], value=(form.get("value") or [""])[0])
        inline = {k: echo[k] for k in ("instance", "layout", "location")}
        self._send(200, FIX.page(dict(inline, regions=[region], frames=[])))


class Oracle:
    """Answers the harness about pages by asking the pages' own script."""

    def __init__(self, cdp: Cdp) -> None:
        self.cdp = cdp
        self.sessions: dict[str, str] = {}  # target id -> flat session id

    def put(self, instance: str, slices: dict, echoes: dict) -> dict:
        FIX.instances[instance] = {"slices": slices, "echoes": echoes}
        return {}

    def plant_file(self, path: str, text: str) -> dict:
        """A planted value into a file on Neo's disk, readable by Neo's user: the only way
        a FILE value travels, never in a request to Neo. The agent then uploads it by
        path, which is not a value."""
        target = Path(path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        target.chmod(0o644)
        return {}

    def prepare_upload(self, target: str, links: list) -> dict:
        """Ready one deny-list probe (leak_plan.deny_uploads): whether its target is a
        file in this container, and the links that name it, made as root, each (kind,
        the target, where) with kind "symlink" or "hard link". A hard link needs the
        target's own filesystem, so it can fail; the probe then names nothing. Paths only
        cross this pipe."""
        if not Path(target).is_file():
            return {"present": False}
        for kind, source, at in links:
            Path(at).parent.mkdir(parents=True, exist_ok=True)
            make = os.link if kind == "hard link" else os.symlink
            try:
                make(source, at)
            except OSError as error:
                why = error.strerror or repr(error)
                return {"present": True, "made": False, "why": why}
        return {"present": True, "made": True}

    def read_back(self, instance: str, region: str, path: str) -> dict:
        """Whether the page's file input `region` read what `path` holds (a file, or every
        file under a directory, through any symlink): the sha256 fx.js took of each file
        it read against each file's own, compared here. Counts and booleans cross this
        pipe; no byte of the files and no digest of them."""
        (main, *_) = self._tabs(instance)
        expression = (
            f"window.__fx.booted.then(() => window.__fx.digests({json.dumps(region)}))"
        )
        got = self._evaluate(main["targetId"], expression) or []
        root = Path(path)
        files = (
            [root]
            if root.is_file()
            else sorted(p for p in root.rglob("*") if p.is_file())
        )
        wanted = {
            hashlib.sha256(data).hexdigest() for f in files if (data := f.read_bytes())
        }
        read = [entry["sha256"] for entry in got if "sha256" in entry]
        return {
            "files": len(got),
            "read": len(read),
            "errors": sorted(
                {str(entry["error"]) for entry in got if "error" in entry}
            ),
            "matched": bool(wanted.intersection(read)),
            "empty": not wanted,
        }

    def digest_control(self, instance: str, input: str, path: str) -> dict:
        """read_back's positive control: a fresh file of random bytes, set into the page's
        control input over DevTools (as setInputFiles sets a probe's), then compared as
        read_back compares. It proves the page's digest and this comparison agree on a
        real file in this browser, before any probe's verdict leans on them."""
        control = Path(path)
        control.parent.mkdir(parents=True, exist_ok=True)
        control.write_bytes(os.urandom(1500))
        control.chmod(0o644)
        (main, *_) = self._tabs(instance)
        self._evaluate(main["targetId"], "0")  # attaches a session when none is yet
        session = self.sessions[main["targetId"]]
        element = self.cdp.call(
            "Runtime.evaluate",
            {"expression": f"document.getElementById({json.dumps(input)})"},
            session=session,
        )
        self.cdp.call(
            "DOM.setFileInputFiles",
            {"files": [str(control)], "objectId": element["result"]["objectId"]},
            session=session,
        )
        return self.read_back(instance, input, str(control))

    def _tabs(self, instance: str) -> list[dict]:
        """The instance's tabs, main first: its main document and any popup it opened."""
        prefix = f"http://a.test:{FIXTURE_PORT}/fx/{instance}/"
        pages = [
            t
            for t in self.cdp.call("Target.getTargets")["targetInfos"]
            if t["type"] == "page" and t["url"].startswith(prefix)
        ]
        return sorted(pages, key=lambda t: not t["url"].startswith(prefix + "main"))

    def _evaluate(self, target: str, expression: str, gesture: bool = False) -> object:
        for attempt in (0, 1):
            if target not in self.sessions:
                self.sessions[target] = self.cdp.call(
                    "Target.attachToTarget", {"targetId": target, "flatten": True}
                )["sessionId"]
            try:
                reply = self.cdp.call(
                    "Runtime.evaluate",
                    {
                        "expression": expression,
                        "awaitPromise": True,
                        "returnByValue": True,
                        "userGesture": gesture,
                    },
                    session=self.sessions[target],
                    timeout=8,
                )
                break
            except RuntimeError:
                self.sessions.pop(target, None)  # the session died with a navigation
                if attempt:
                    raise
        if "exceptionDetails" in reply:
            raise RuntimeError(reply["exceptionDetails"].get("text", "exception"))
        return reply["result"].get("value")

    def state(self, instance: str) -> dict:
        regions: dict = {}
        documents: list = []
        focus: list = []
        tabs = self._tabs(instance)
        for tab in tabs:
            try:
                got = self._evaluate(
                    tab["targetId"],
                    "window.__fx.booted.then(() => window.__fx.state())",
                )
            except (RuntimeError, TimeoutError):
                continue  # a tab that did not answer proves nothing about its regions
            regions.update(got["regions"])
            documents += got["documents"]
            focus += got.get("focus", [])
        return {
            "at": int(time.time() * 1000),
            "regions": regions,
            "documents": documents,
            "focus": focus,
            "tabs": len(tabs),
        }

    def act(self, instance: str, name: str, gesture: bool = False) -> dict:
        (main, *_) = self._tabs(instance)
        expression = f"window.__fx.act({json.dumps(name)})"
        return {"done": self._evaluate(main["targetId"], expression, gesture)}

    def focus(
        self,
        instance: str,
        region: str,
        path: list,
        tab: str = "main",
        select: bool = False,
    ) -> dict:
        tabs = self._tabs(instance)
        target = tabs[0] if tab == "main" else tabs[-1]
        # The owner's click also brings the tab to the front, where the keys go.
        self._evaluate(target["targetId"], "0")  # attaches a session when none is yet
        self.cdp.call("Page.bringToFront", session=self.sessions[target["targetId"]])
        args = ", ".join(json.dumps(a) for a in (region, path, select))
        return self._evaluate(target["targetId"], f"window.__fx.focus({args})")

    def targets(self) -> dict:
        infos = self.cdp.call("Target.getTargets")["targetInfos"]
        return {
            "targets": [
                {"type": t["type"], "url": t["url"], "attached": t.get("attached")}
                for t in infos
            ]
        }

    def front(self) -> dict:
        """The URLs of the tabs in front of Neo's one screen: BrowserOS's Browser.getTabs
        active tabs, the read Neo's own front-tab watch makes."""
        tabs = self.cdp.call("Browser.getTabs")["tabs"]
        return {"front": [t.get("url", "") for t in tabs if t.get("isActive")]}

    def clock(self) -> dict:
        return {"ms": int(time.time() * 1000)}

    def grep(self, root: str, needles: list[str]) -> dict:
        """Which needles occur under `root`, by index: one fixed-string pass of GNU grep
        (Aho-Corasick) over every byte, so the values never leave the container."""
        index = {n: i for i, n in enumerate(needles)}
        with tempfile.NamedTemporaryFile("w", suffix=".txt") as patterns:
            patterns.write("\n".join(needles) + "\n")
            patterns.flush()
            found = subprocess.run(
                ["grep", "-rFoa", "-f", patterns.name, root],
                capture_output=True,
                timeout=300,
            ).stdout.decode(errors="replace")
        hits = set()
        for line in found.splitlines():
            path, _, match = line.rpartition(":")
            if match in index:
                hits.add((path, index[match]))
        return {"hits": [{"file": f, "needle": i} for f, i in sorted(hits)]}


def main() -> int:
    server = ThreadingHTTPServer(("127.0.0.1", FIXTURE_PORT), FixtureHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    oracle = Oracle(Cdp())
    print(json.dumps({"id": 0, "ok": True, "ready": True}), flush=True)
    for line in sys.stdin:
        request = json.loads(line)
        params = request.get("params", {})
        try:
            reply = {
                "id": request["id"],
                "ok": True,
                **getattr(oracle, request["op"])(**params),
            }
        except Exception as error:  # noqa: BLE001 - every failure goes back to the harness
            reply = {"id": request["id"], "ok": False, "error": repr(error)}
        print(json.dumps(reply), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
