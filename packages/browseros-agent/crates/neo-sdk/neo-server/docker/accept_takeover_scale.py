#!/usr/bin/env python3
"""Prove logical phone sizing, physical RFB sizing, and agent capture on the packaged image.

The fixture publishes its own observations because agent evaluate calls remain writes during
a takeover. A timer also publishes while an agent tab is in the background, without animation
frames. Only the host's RFB connection sends input. A separate CDP connection reads window
bounds, never attaches to the page or changes the SDK's emulation session. Standard library only.
"""

import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import signal
import socket
import struct
import sys
import threading
import time
import urllib.parse
import urllib.request
import zlib

from accept_client import call_run, connect, host, host_raw

PHONE = {"width": 366, "height": 700}
ORIGINAL = {"width": 517, "height": 433}
COLORS = ((255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 0), (32, 48, 64))
FIXTURE = """<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Takeover scale proof</title><style>
html,body{margin:0;overflow:hidden;width:100%;height:100%;background:rgb(32,48,64);touch-action:none}
button{position:fixed;width:32px;height:32px;padding:0;border:0;touch-action:none}
#tl{left:8px;top:8px;background:red}#tr{right:8px;top:8px;background:lime}
#bl{left:8px;bottom:8px;background:blue}#br{right:8px;bottom:8px;background:yellow}
#entry{position:fixed;left:48px;top:80px;width:128px;height:40px;box-sizing:border-box;
 margin:0;border:0;padding:0;font-size:16px;background:white}
iframe{position:fixed;right:64px;bottom:64px;width:100px;height:60px;border:0}
</style><button id="tl" aria-label="Top left"></button><button id="tr" aria-label="Top right"></button>
<button id="bl" aria-label="Bottom left"></button><button id="br" aria-label="Bottom right"></button>
<input id="entry" aria-label="Entry"><iframe src="http://localhost:__PORT__/frame"></iframe>
<output id="proof" hidden></output><script>
const events=[]; let frameReady=false,eventSeq=0;
const record=(kind,e,frame=false)=>{events.push({seq:++eventSeq,kind,frame,target:e.target.id,x:e.clientX,y:e.clientY,
  buttons:e.buttons,trusted:e.isTrusted});if(events.length>80)events.shift()};
for(const kind of ['click','pointerdown','pointermove','pointerup'])addEventListener(kind,e=>{
  if(kind!=='pointermove'||e.buttons)record(kind,e);
  // Pointer capture keeps the drag observable when it crosses the framed input.
  if(kind==='pointerdown'&&e.target.tagName==='BUTTON')e.target.setPointerCapture(e.pointerId);
},true);
addEventListener('message',e=>{if(e.source!==document.querySelector('iframe').contentWindow)return;
  if(e.data.ready)frameReady=true;else if(e.data.event){events.push({...e.data.event,seq:++eventSeq});if(events.length>80)events.shift()}});
const media=queries=>Object.fromEntries(queries.map(query=>[query,matchMedia(query).matches]));
const publish=()=>{const v=visualViewport;document.getElementById('proof').textContent=JSON.stringify({
  metrics:{logical:{screen:[screen.width,screen.height],inner:[innerWidth,innerHeight],
    visualViewport:{width:v.width,height:v.height,scale:v.scale,offsetLeft:v.offsetLeft,offsetTop:v.offsetTop,
      pageLeft:v.pageLeft,pageTop:v.pageTop},dpr:devicePixelRatio,
    media:media(['(max-width: 600px)','(device-width: 366px)','(resolution: 2dppx)',
      '(pointer: coarse)','(hover: none)','(orientation: portrait)']),
    touch:{points:navigator.maxTouchPoints,ontouchstart:'ontouchstart' in window},
    identity:{userAgent:navigator.userAgent,platform:navigator.platform,mobile:navigator.userAgentData?.mobile}},
    outer:[outerWidth,outerHeight],available:[screen.availWidth,screen.availHeight,screen.availLeft,screen.availTop],
    position:[screenX,screenY]},frameReady,eventSeq,events});};
addEventListener('neo-proof-refresh',publish);setInterval(publish,100);publish();
</script>"""
FRAME = """<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<style>html,body{margin:0;overflow:hidden}input{width:100vw;height:100vh;box-sizing:border-box;
margin:0;padding:0;border:0;background:magenta;font-size:16px;touch-action:none}</style>
<input id="card" aria-label="Card code"><script>
addEventListener('click',e=>parent.postMessage({event:{kind:'click',frame:true,target:e.target.id,
x:e.clientX,y:e.clientY,trusted:e.isTrusted}},'*'));parent.postMessage({ready:true},'*');
</script>"""


class Site(http.server.BaseHTTPRequestHandler):
    """One loopback server serves two sites so the framed input has a separate renderer."""

    def do_GET(self):
        body = (FRAME if self.path == "/frame" else FIXTURE.replace("__PORT__", str(self.server.server_port))).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


class WebSocket:
    """A bounded RFC 6455 client shared by host RFB and read-only browser observations.

    HTTP reads can already contain the first frame. Keep those bytes and frame fragments so
    socket packet boundaries cannot change the RFB stream or a CDP JSON reply.
    """

    def __init__(self, connection, authority, path, extra_headers=""):
        self.connection, self.buffer = connection, bytearray()
        connection.settimeout(10)
        key = base64.b64encode(os.urandom(16)).decode()
        connection.sendall((f"GET {path} HTTP/1.1\r\nHost: {authority}\r\nUpgrade: websocket\r\n"
                            f"Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n"
                            f"{extra_headers}\r\n").encode())
        try:
            while b"\r\n\r\n" not in self.buffer:
                chunk = connection.recv(4096)
                assert chunk and len(self.buffer) < 16384, "WebSocket upgrade did not finish"
                self.buffer.extend(chunk)
            head, _, tail = self.buffer.partition(b"\r\n\r\n")
            self.buffer = bytearray(tail)
            lines = head.decode().split("\r\n")
            assert " 101 " in lines[0], f"WebSocket upgrade: {lines[0]}"
            headers = {name.lower(): value.strip() for name, value in (line.split(":", 1) for line in lines[1:])}
            expected = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
            assert headers.get("sec-websocket-accept") == expected, "WebSocket accept differs"
        except Exception:
            connection.close()
            raise

    def read(self, count, deadline):
        while len(self.buffer) < count:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("WebSocket read deadline")
            self.connection.settimeout(remaining)
            chunk = self.connection.recv(min(65536, count - len(self.buffer)))
            assert chunk, "WebSocket closed without a close frame"
            self.buffer.extend(chunk)
        result = bytes(self.buffer[:count])
        del self.buffer[:count]
        return result

    def send(self, opcode, payload):
        mask = os.urandom(4)
        size = len(payload)
        length = bytes([0x80 | size]) if size < 126 else b"\xfe" + struct.pack("!H", size) if size < 65536 else b"\xff" + struct.pack("!Q", size)
        self.connection.sendall(bytes([0x80 | opcode]) + length + mask + bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload)))

    def receive(self, deadline):
        while True:
            first, second = self.read(2, deadline)
            assert not second & 0x80, "server WebSocket frames must be unmasked"
            size = second & 0x7f
            if size in (126, 127):
                size = struct.unpack("!H" if size == 126 else "!Q", self.read(2 if size == 126 else 8, deadline))[0]
            assert size <= 8 * 1024 * 1024, "WebSocket frame exceeded the proof's bound"
            payload = self.read(size, deadline)
            opcode = first & 0x0f
            if opcode == 9:
                self.send(10, payload)
            elif opcode != 10:
                return opcode, payload, bool(first & 0x80)

    def close(self):
        self.connection.close()


class Rfb:
    """A host display whose actual ServerInit and layout messages measure Xvnc's geometry."""

    def __init__(self, socket_path, token_file, path):
        connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        connection.settimeout(10)
        connection.connect(socket_path)
        token = Path(token_file).read_text().strip()
        self.websocket = WebSocket(connection, "neo", path, f"Authorization: Bearer {token}\r\n")
        self.buffer, self.screen_entry, self.current_layout = bytearray(), None, None
        try:
            assert self.read(12) == b"RFB 003.008\n", "display must speak RFB 3.8"
            self.send(b"RFB 003.008\n")
            types = self.read(self.read(1)[0])
            assert 1 in types, f"RFB security types: {list(types)}"
            self.send(b"\x01")
            assert self.read(4) == b"\0\0\0\0", "RFB security refused"
            self.send(b"\x01")
            initial = self.read(24)
            width, height = struct.unpack("!HH", initial[:4])
            self.bytes_per_pixel = initial[4] // 8
            length = struct.unpack("!I", initial[20:24])[0]
            assert self.bytes_per_pixel in (2, 3, 4) and length <= 4096, "RFB ServerInit is bounded"
            self.initial = {"width": width, "height": height, "bitsPerPixel": initial[4],
                            "desktop": self.read(length).decode(errors="replace")}
            self.send(struct.pack("!BBHii", 2, 0, 2, 0, -308))
        except Exception:
            self.close()
            raise

    def send(self, payload):
        self.websocket.send(2, payload)

    def read(self, count, deadline=None):
        deadline = deadline or time.monotonic() + 10
        while len(self.buffer) < count:
            opcode, payload, _ = self.websocket.receive(deadline)
            assert opcode in (0, 2), f"RFB received WebSocket opcode {opcode}: {payload[:80]!r}"
            self.buffer.extend(payload)
        result = bytes(self.buffer[:count])
        del self.buffer[:count]
        return result

    def layout(self, width, height):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            # A fresh nonincremental update also confirms an unchanged layout. Xvnc need not
            # repeat ExtendedDesktopSize until the size changes, so waiting for it would hang.
            self.send(struct.pack("!BBHHHH", 3, 0, 0, 0, 1, 1))
            kind = self.read(1, deadline)[0]
            if kind == 2:
                continue
            if kind == 3:
                length = struct.unpack("!I", self.read(7, deadline)[3:])[0]
                assert length <= 1024 * 1024, "RFB clipboard is bounded"
                self.read(length, deadline)
                continue
            assert kind == 0, f"unexpected RFB server message {kind}"
            count = struct.unpack("!H", self.read(3, deadline)[1:])[0]
            for _ in range(count):
                x, y, w, h, encoding = struct.unpack("!HHHHi", self.read(12, deadline))
                if encoding == 0:
                    assert w * h * self.bytes_per_pixel <= 8 * 1024 * 1024, "RFB raw rectangle is bounded"
                    self.read(w * h * self.bytes_per_pixel, deadline)
                else:
                    assert encoding == -308, f"unexpected RFB encoding {encoding}"
                    screens = self.read(4, deadline)[0]
                    entries = self.read(screens * 16, deadline)
                    if screens:
                        self.screen_entry = (struct.unpack("!I", entries[:4])[0], struct.unpack("!I", entries[12:16])[0])
                    assert x != 1 or y == 0, f"RFB refused resize with status {y}"
                    self.current_layout = {"width": w, "height": h, "reason": x, "status": y}
            if self.current_layout and (self.current_layout["width"], self.current_layout["height"]) == (width, height):
                return {**self.current_layout, "confirmedByUpdate": True}
        raise TimeoutError(f"RFB layout did not become {width}x{height}")

    def resize(self, width, height):
        assert self.screen_entry, "read a layout before resizing"
        identity, flags = self.screen_entry
        self.send(struct.pack("!BBHHBBIHHHHI", 251, 0, width, height, 1, 0, identity, 0, 0, width, height, flags))
        return self.layout(width, height)

    def pointer(self, x, y, buttons=0):
        self.send(struct.pack("!BBHH", 5, buttons, round(x), round(y)))

    def click(self, x, y):
        self.pointer(x, y)
        self.pointer(x, y, 1)
        self.pointer(x, y)

    def released(self):
        deadline = time.monotonic() + 10
        while True:
            opcode, payload, _ = self.websocket.receive(deadline)
            if opcode == 8:
                code = struct.unpack("!H", payload[:2])[0]
                assert code == 4002, f"takeover display close code {code}"
                return {"code": code, "reason": payload[2:].decode()}

    def close(self):
        self.websocket.close()


class Window:
    """Read the native window independently of page metrics, with no page session or writes."""

    def __init__(self):
        with urllib.request.urlopen("http://127.0.0.1:9110/json/version", timeout=5) as response:
            self.debugger_url = json.load(response)["webSocketDebuggerUrl"]
            target = urllib.parse.urlparse(self.debugger_url)
        self.websocket = WebSocket(socket.create_connection((target.hostname, target.port), timeout=5), target.netloc, target.path)
        self.serial = 0

    def command(self, method, params=None):
        """Keep one browser connection across a server restart to distinguish it from a relaunch."""
        self.serial += 1
        self.websocket.send(1, json.dumps({"id": self.serial, "method": method, "params": params or {}}).encode())
        deadline, message = time.monotonic() + 10, bytearray()
        while True:
            opcode, payload, final = self.websocket.receive(deadline)
            assert opcode in (0, 1), f"CDP WebSocket opcode {opcode}"
            message.extend(payload)
            if final:
                result, message = json.loads(message), bytearray()
                if result.get("id") == self.serial:
                    assert "error" not in result, result
                    return result["result"]

    def bounds(self, target):
        return self.command("Browser.getWindowForTarget", {"targetId": target})

    def close(self):
        self.websocket.close()


def samples(width, height):
    return [(24, 24), (width - 24, 24), (24, height - 24), (width - 24, height - 24), (width // 2, height // 2)]


def png_pixels(data, density):
    """Decode Chromium's 8-bit RGB/RGBA PNG rows so colored corners prove the captured area.

    Inspect the encoded artifact itself. Dimensions alone also pass for a blank, cropped, or
    incorrectly enlarged screenshot. All five PNG row filters are reversed before sampling.
    """
    assert data.startswith(b"\x89PNG\r\n\x1a\n"), "PNG signature"
    position, compressed, dimensions = 8, bytearray(), None
    while position < len(data):
        length, kind = struct.unpack("!I4s", data[position:position + 8])
        body = data[position + 8:position + 8 + length]
        crc = struct.unpack("!I", data[position + 8 + length:position + 12 + length])[0]
        assert zlib.crc32(kind + body) == crc, f"PNG {kind!r} CRC"
        if kind == b"IHDR":
            width, height, depth, color, compression, filtering, interlace = struct.unpack("!IIBBBBB", body)
            assert depth == 8 and color in (2, 6) and (compression, filtering, interlace) == (0, 0, 0), "PNG must be noninterlaced 8-bit RGB/RGBA"
            dimensions = width, height, 3 if color == 2 else 4
        elif kind == b"IDAT":
            compressed.extend(body)
        position += length + 12
        if kind == b"IEND":
            break
    assert dimensions, "PNG IHDR"
    width, height, channels = dimensions
    assert 0 < width <= 4096 and 0 < height <= 4096, "PNG dimensions are bounded"
    stride = width * channels
    raw = zlib.decompress(compressed)
    assert len(raw) == height * (stride + 1), "PNG decompressed row count"
    points = [(round(x * density), round(y * density)) for x, y in samples(round(width / density), round(height / density))]
    pixels, previous = {}, bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        method, row = raw[start], bytearray(raw[start + 1:start + 1 + stride])
        assert method <= 4, f"PNG row filter {method}"
        for x in range(stride):
            left, above, upper_left = row[x - channels] if x >= channels else 0, previous[x], previous[x - channels] if x >= channels else 0
            if method == 1:
                prediction = left
            elif method == 2:
                prediction = above
            elif method == 3:
                prediction = (left + above) // 2
            elif method == 4:
                estimate = left + above - upper_left
                distances = (abs(estimate - left), abs(estimate - above), abs(estimate - upper_left))
                prediction = (left, above, upper_left)[distances.index(min(distances))]
            else:
                prediction = 0
            row[x] = (row[x] + prediction) & 255
        for index, (x, target_y) in enumerate(points):
            if y == target_y:
                pixels[index] = list(row[x * channels:x * channels + 3])
        previous = row
    return {"width": width, "height": height, "pixels": [pixels[index] for index in range(len(points))]}


def jpeg_size(data):
    assert data[:2] == b"\xff\xd8", "JPEG signature"
    position = 2
    while position < len(data):
        assert data[position] == 255, "JPEG segment marker"
        while data[position] == 255:
            position += 1
        marker, position = data[position], position + 1
        if marker in (0xd8, 0xd9) or 0xd0 <= marker <= 0xd7:
            continue
        length = struct.unpack("!H", data[position:position + 2])[0]
        assert length >= 2 and position + length <= len(data), "JPEG segment bounds"
        if marker in (0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf):
            height, width = struct.unpack("!HH", data[position + 3:position + 7])
            return {"width": width, "height": height}
        assert marker != 0xda, "JPEG scan before dimensions"
        position += length
    raise AssertionError("JPEG dimensions missing")


def until(read, predicate, message, seconds=15):
    deadline, last = time.monotonic() + seconds, None
    while True:
        last = read()
        if predicate(last):
            return last
        remaining = deadline - time.monotonic()
        assert remaining > 0, f"{message}: {last}"
        time.sleep(min(0.1, remaining))


def fresh_events(value, after):
    return [event for event in value["events"] if event["seq"] > after]


def check(url, socket_path, token_file, artifacts):
    evidence = {"ok": False, "problems": [], "cases": {}, "expected": {"phone": PHONE, "agent": ORIGINAL}}
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Site)
    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.1}, daemon=True)
    thread.start()
    client, window, spectator, control, takeover, tab = None, None, None, None, None, None
    task = f"accept-takeover-scale-{os.getpid()}"
    artifacts = Path(artifacts)
    artifacts.mkdir(parents=True, exist_ok=True)

    def request(method, path, body=None):
        reply = host(socket_path, token_file, path, method, "" if body is None else json.dumps(body))
        assert reply.get("status") == 200, f"{method} {path}: {reply}"
        return reply["body"]

    def run(code):
        reply = call_run(client, code, {"taskId": task, "hostData": {"check": "takeover-scale"}})
        assert not reply["isError"] and reply["meta"].get("status") == "ok", reply
        return (reply.get("value") or {}).get("value"), reply["meta"]["id"]

    def on_page(code):
        return run(f"const p = context.pages().find(p => p.tabId === {json.dumps(tab)});\n" + code)

    def facts():
        return on_page("const text = await p.locator('#proof').textContent(); return text ? JSON.parse(text) : null;")[0]

    def at_size(width, height, dpr):
        def matches(value):
            logical = value.get("metrics", {}).get("logical", {}) if value else {}
            visual = logical.get("visualViewport", {})
            return (value and value.get("frameReady") and logical.get("screen") == [width, height]
                    and logical.get("inner") == [width, height] and logical.get("dpr") == dpr
                    and visual.get("width") == width and visual.get("height") == height and visual.get("scale") == 1)
        return until(facts, matches, f"page did not reach {width}x{height} DPR {dpr}")

    def fitted(fit_id, width, height, scale):
        physical = {"width": width * scale, "height": height * scale}
        fit = until(lambda: request("GET", f"/v1/fits/{fit_id}"),
                    lambda item: item.get("screen") == physical and item.get("fit") == "fitted"
                    and item.get("keyboard", {}).get("width") == physical["width"]
                    and item.get("keyboard", {}).get("height") == physical["height"]
                    and len(item.get("keyboard", {}).get("boxes", [])) == 1,
                    "fit did not publish physical keyboard geometry")
        assert fit["mode"] == "mobile", fit
        if scale == 2:
            assert fit.get("viewer") == {"width": width, "height": height, "scale": 2}, fit
        else:
            assert "viewer" not in fit, "unscaled Fit must retain its response shape"
        expected = [48 * scale, 80 * scale, 128 * scale, 40 * scale]
        assert all(abs(actual - wanted) <= 1 for actual, wanted in zip(fit["keyboard"]["boxes"][0], expected)), fit
        return fit

    def screenshots(label, width, height, density):
        result = {"images": {}}
        evidence.setdefault("captures", {})[label] = result
        values, run_id = on_page("""return {
          device: await p.screenshot({type:'png',timeout:15000}),
          css: await p.screenshot({type:'png',scale:'css',timeout:15000}),
          jpeg: await p.screenshot({type:'jpeg',quality:90,timeout:15000})};""")
        result["run"] = run_id
        problems = []
        for name, marker in values.items():
            if marker.get("$neo") == "bytes":
                data = base64.b64decode(marker["base64"], validate=True)
            else:
                assert marker.get("$neo") == "blob", marker
                raw = host_raw(socket_path, token_file, f"/v1/runs/{run_id}/blobs/{marker['id']}")
                assert isinstance(raw, tuple) and raw[0] == 200 and len(raw[2]) == marker["size"], str(raw)[:200]
                data = raw[2]
            filename = f"{label}-{name}.{'jpg' if name == 'jpeg' else 'png'}"
            (artifacts / filename).write_bytes(data)
            info = {"file": filename, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
            result["images"][name] = info
            try:
                # Capture density follows page DPR. Preview scale must never multiply it again.
                image_density = 1 if name == "css" else density
                info.update(jpeg_size(data) if name == "jpeg" else png_pixels(data, image_density))
                assert (info["width"], info["height"]) == (width * image_density, height * image_density), f"{label} {name}: {info}"
                if name != "jpeg":
                    assert info["pixels"] == [list(color) for color in COLORS], f"{label} {name} fixture pixels: {info}"
            except Exception as error:
                problems.append(str(error))
        assert not problems, "; ".join(problems)
        return result

    def restored(before, target, home):
        # The packaged browser already changes DOM outerHeight after an unscaled fullscreen
        # fit, despite restoring native bounds. Assert the logical device independently and
        # compare the complete 2x restoration against the measured 1x restoration below.
        expected = before["metrics"]
        metrics = until(facts, lambda value: value and all(value["metrics"][key] == expected[key]
                        for key in ("logical", "available", "position")), "original logical page metrics were not restored")
        bounds = until(lambda: window.bounds(target), lambda value: value == before["window"], "native window was not restored")
        layout = spectator.layout(home["width"], home["height"])
        display = Rfb(socket_path, token_file, "/v1/display?mode=view")
        try:
            initial = display.initial
            assert (initial["width"], initial["height"]) == (home["width"], home["height"]), initial
        finally:
            display.close()
        return {"metrics": metrics["metrics"], "window": bounds, "display": layout, "serverInit": initial}

    def jpeg_content(captured, width, height, density):
        data = (artifacts / captured["images"]["jpeg"]["file"]).read_bytes()
        points = [(round(x * density), round(y * density)) for x, y in samples(width, height)]
        value, _ = on_page("""return await p.evaluate(async ({url,points}) => {
          const image=new Image();await new Promise((resolve,reject)=>{image.onload=resolve;image.onerror=reject;image.src=url});
          const canvas=document.createElement('canvas');canvas.width=image.width;canvas.height=image.height;
          const drawing=canvas.getContext('2d');drawing.drawImage(image,0,0);
          return {width:image.width,height:image.height,pixels:points.map(([x,y])=>Array.from(drawing.getImageData(x,y,1,1).data).slice(0,3))};
        }, """ + json.dumps({"url": "data:image/jpeg;base64," + base64.b64encode(data).decode(), "points": points}) + ");")
        assert (value["width"], value["height"]) == (width * density, height * density), value
        assert all(abs(actual - wanted) <= 12 for pixel, expected in zip(value["pixels"], COLORS) for actual, wanted in zip(pixel, expected)), value
        captured["images"]["jpeg"]["decodedPixels"] = value["pixels"]

    try:
        client = connect(url)
        origin = f"http://127.0.0.1:{server.server_port}/"
        tab, _ = run("const p = await context.newPage(); return p.tabId;")
        on_page(f"await p.setViewportSize({json.dumps(ORIGINAL)}); "
                f"await p.goto({json.dumps(origin)}); return true;")
        target = next(item["targetId"] for item in request("GET", "/v1/tabs")["items"] if item["tabId"] == tab)
        window = Window()
        before_facts = at_size(ORIGINAL["width"], ORIGINAL["height"], 1)
        before = {"metrics": before_facts["metrics"], "window": window.bounds(target)}
        spectator = Rfb(socket_path, token_file, "/v1/display?mode=view")
        home = spectator.initial
        before["display"] = home
        evidence["before"] = before
        spectator.layout(home["width"], home["height"])

        for label, scale in (("unscaled", 1), ("scaled", 2)):
            viewer = {**PHONE, **({"scale": 2} if scale == 2 else {})}
            case = {"request": {"tabId": tab, "reads": "allow", "viewer": viewer, "ttlSeconds": 300}}
            evidence["cases"][label] = case
            takeover = request("POST", "/v1/takeovers", case["request"])
            case["takeover"] = takeover
            assert takeover["state"] == "active" and takeover.get("fitId"), takeover
            control = Rfb(socket_path, token_file, takeover["display"])
            case["serverInit"] = control.initial
            assert (control.initial["width"], control.initial["height"]) == (366 * scale, 700 * scale), control.initial
            case["alreadyOpenDisplay"] = spectator.layout(366 * scale, 700 * scale)
            control.layout(366 * scale, 700 * scale)
            case["fit"] = fitted(takeover["fitId"], 366, 700, scale)
            case["metrics"] = at_size(366, 700, 2)["metrics"]
            case["agentBox"], _ = on_page("return await p.locator('#br').boundingBox();")
            assert case["agentBox"] == {"x": 326, "y": 660, "width": 32, "height": 32}, case["agentBox"]
            case["window"] = window.bounds(target)
            assert case["window"]["bounds"]["windowState"] == "fullscreen", case["window"]
            assert [case["window"]["bounds"][side] for side in ("width", "height")] == [366 * scale, 700 * scale], case["window"]
            case["screenshots"] = screenshots(label, 366, 700, 2)
            assert at_size(366, 700, 2)["metrics"] == case["metrics"], "capture changed the page's phone metrics"
            assert window.bounds(target) == case["window"], "capture changed the native window"

            if scale == 2:
                refusal, _ = on_page("try {await p.mouse.click(24,24);return {sent:true}} catch(error) {return {code:error.code,delivery:error.delivery}}")
                case["agentInput"] = refusal
                assert refusal == {"code": "owner_has_control", "delivery": "not_sent"}, refusal
                for name, (x, y) in zip(("tl", "tr", "bl", "br"), samples(366, 700)[:4]):
                    count = facts()["eventSeq"]
                    control.click(x * scale, y * scale)
                    got = until(facts, lambda value: any(event["kind"] == "click" and event["target"] == name
                                for event in fresh_events(value, count)), f"RFB click missed {name}")
                    event = next(event for event in fresh_events(got, count) if event["kind"] == "click" and event["target"] == name)
                    assert event["trusted"] and abs(event["x"] - x) <= 2 and abs(event["y"] - y) <= 2, event
                count = facts()["eventSeq"]
                control.click((366 - 114) * scale, (700 - 94) * scale)
                got = until(facts, lambda value: any(event.get("frame") and event["kind"] == "click" for event in fresh_events(value, count)), "RFB click missed the cross-site input")
                child = next(event for event in fresh_events(got, count) if event.get("frame") and event["kind"] == "click")
                assert child["trusted"] and abs(child["x"] - 50) <= 2 and abs(child["y"] - 30) <= 2, child
                control.pointer(24 * scale, 24 * scale)
                count = facts()["eventSeq"]
                control.pointer(24 * scale, 24 * scale, 1)
                until(facts, lambda value: any(event["kind"] == "pointerdown" for event in fresh_events(value, count)), "RFB drag did not start")
                for x, y in ((342, 24), (342, 676), (24, 676)):
                    count = facts()["eventSeq"]
                    control.pointer(x * scale, y * scale, 1)
                    until(facts, lambda value: any(event["kind"] == "pointermove" and event["trusted"]
                          and abs(event["x"] - x) <= 2 and abs(event["y"] - y) <= 2 for event in fresh_events(value, count)), "RFB drag missed an edge")
                count = facts()["eventSeq"]
                control.pointer(24 * scale, 676 * scale)
                case["ownerEvents"] = until(facts, lambda value: any(event["kind"] == "pointerup"
                                                   for event in fresh_events(value, count)), "RFB drag did not end")["events"]

                case["refits"] = []
                for width, height, requested in ((366, 500, None), (366, 700, None), (366, 700, 1), (366, 700, 2)):
                    body = {"width": width, "height": height, **({"scale": requested} if requested else {})}
                    refit = request("POST", f"/v1/tabs/{tab}/fit", body)
                    assert refit["id"] == takeover["fitId"], "refit must retain the takeover's restoration owner"
                    active_scale = requested or 2
                    case["refits"].append({"request": body, "fit": fitted(refit["id"], width, height, active_scale),
                                           "metrics": at_size(width, height, 2)["metrics"],
                                           "display": spectator.layout(width * active_scale, height * active_scale)})
                case["remoteResize"] = {"ack": control.resize(728, 1200),
                                        "metrics": at_size(364, 600, 2)["metrics"],
                                        "fit": fitted(takeover["fitId"], 364, 600, 2)}
                request("POST", f"/v1/tabs/{tab}/fit", PHONE)
                fitted(takeover["fitId"], 366, 700, 2)
                at_size(366, 700, 2)
                spectator.layout(732, 1400)

            case["release"] = request("POST", f"/v1/takeovers/{takeover['id']}/release")
            assert case["release"]["state"] == "released" and case["release"].get("inputDrained") is True, case["release"]
            case["displayClose"] = control.released()
            control.close()
            control, takeover = None, None
            case["restored"] = restored(before, target, home)
            case["restored"]["outerMatchesBefore"] = case["restored"]["metrics"]["outer"] == before["metrics"]["outer"]
            if scale == 2:
                assert case["restored"]["metrics"] == evidence["cases"]["unscaled"]["restored"]["metrics"], "scaled restoration must match the complete unscaled restoration"
            # JPEG decoding runs only after release. It examines the captured bytes without
            # using evaluate to evade the active takeover's agent-write policy.
            jpeg_content(case["screenshots"], 366, 700, 2)

        unscaled, scaled = (evidence["cases"][label]["metrics"] for label in ("unscaled", "scaled"))
        evidence["metricsSideBySide"] = {key: {"unscaled": unscaled[key], "scaled": scaled[key], "same": unscaled[key] == scaled[key]}
                                         for key in unscaled}
        assert unscaled["logical"] == scaled["logical"], evidence["metricsSideBySide"]
        evidence["afterScreenshots"] = screenshots("restored", ORIGINAL["width"], ORIGINAL["height"], 1)
        jpeg_content(evidence["afterScreenshots"], ORIGINAL["width"], ORIGINAL["height"], 1)
        evidence["restoredAgentBox"], _ = on_page("return await p.locator('#br').boundingBox();")
        assert evidence["restoredAgentBox"] == {"x": 477, "y": 393, "width": 32, "height": 32}, evidence["restoredAgentBox"]
        count = facts()["eventSeq"]
        on_page(f"await p.mouse.click({ORIGINAL['width'] - 24},{ORIGINAL['height'] - 24}); return true;")
        value = until(facts, lambda value: any(event["kind"] == "click" and event["target"] == "br"
                      for event in fresh_events(value, count)), "agent CSS coordinates retained the takeover preview scale")
        evidence["restoredAgentInput"] = fresh_events(value, count)
        evidence["after"] = restored(before, target, home)
        evidence["ok"] = True
        evidence["detail"] = ("RFB 366x700 at omitted scale and 732x1400 at scale 2; equal logical phone metrics and DPR 2; "
                              "device PNG/JPEG 732x1400, CSS PNG 366x700 with fixture pixels; owner edge/frame clicks, "
                              "physical keyboard boxes and refits passed; display/window and 517x433 agent viewport restored")
    except Exception as error:
        evidence["problems"].append(f"{type(error).__name__}: {error}")
        evidence["detail"] = evidence["problems"][0]
    finally:
        # Release before closing the owned tab. A failed assertion must not strand the shared
        # screen, the tab's lock, or its emulation for the remaining image acceptance checks.
        for label, cleanup in (
            ("release", lambda: request("POST", f"/v1/takeovers/{takeover['id']}/release") if takeover else None),
            ("control", lambda: control.close() if control else None),
            ("spectator", lambda: spectator.close() if spectator else None),
            ("window observer", lambda: window.close() if window else None),
            ("owned tab", lambda: on_page("await p.close(); return true;") if tab is not None else None),
        ):
            try:
                cleanup()
            except Exception as error:
                evidence["ok"] = False
                evidence["problems"].append(f"cleanup {label}: {error}")
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
    if not evidence["ok"]:
        evidence["detail"] = "; ".join(evidence["problems"])
    (artifacts / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return evidence


if __name__ == "__main__":
    def timed_out(_signal, _frame):
        # Give the normal release/close path a bounded cleanup window after the work budget.
        signal.signal(signal.SIGALRM, signal.SIG_DFL)
        signal.alarm(30)
        raise TimeoutError("takeover scale acceptance exceeded 240 seconds")

    signal.signal(signal.SIGALRM, timed_out)
    signal.alarm(240)
    result = check(*sys.argv[1:])
    signal.alarm(0)
    print(json.dumps(result))
    sys.exit(0 if result["ok"] else 1)
