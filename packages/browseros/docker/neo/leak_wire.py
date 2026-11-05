"""The leak test's wire: how the host talks to a Neo container and keeps what it answers.

leak_sdk.py's Session reaches the container only through these clients, so every byte Neo
sends back can land in the evidence:

  Docker        the container's life, exec, copies and logs, through the docker CLI
  Agent         MCP `run` on the agent port: the exact request and the raw reply
  Host          the host API on TCP with the bearer token, one whole answer per call
  StreamReader  one long host answer (events SSE, MJPEG screencast, fMP4 video), a thread
  Display       /v1/display: RFB 3.8 over a WebSocket, a thread; the owner's keys go out

Each thread is the only writer of its blob and stamps every chunk with the host time it
arrived (int ms, time.time()). The parsers are pure and incremental, so any split of the
bytes parses the same: the Session reads a stream live with the code the judge replays it
with. Standard library only.
"""

from __future__ import annotations

import dataclasses
import http.client
import json
import re
import socket
import struct
import subprocess
import threading
import time
import urllib.error
import urllib.request
from collections.abc import Callable
from pathlib import Path

from leak_inside import WebSocket
from smoke import PROTOCOL, McpClient

# Every Place.host in leak_plan.PLACES. leak_inside.py serves them all on the container's
# loopback, so each must resolve there.
FIXTURE_HOSTS = ("a.test", "sub.a.test", "b.test")
RUN_META = "com.browseros.neo/run"
# The agent's HTTP read timeout must outlast a run's own, so a run that times out still
# answers with Neo's timeout error instead of a dropped socket.
MCP_READ_TIMEOUT_S = 120
RUN_TIMEOUT_MAX_MS = 110_000
KEY_DELAY_S = 0.015
VIEW_REFRESH_S = 0.3


def _now_ms() -> int:
    return int(time.time() * 1000)


def _header_dict(pairs: list[tuple[str, str]]) -> dict[str, str]:
    """Names lower-cased, as HTTP names are case-insensitive; a repeat joins with ", "."""
    headers: dict[str, str] = {}
    for name, value in pairs:
        key = name.strip().lower()
        value = value.strip()
        headers[key] = f"{headers[key]}, {value}" if key in headers else value
    return headers


class Docker:
    """One leak-test container, through the docker CLI.

    Names must start with `leak-`: `remove` is `docker rm -f`, and the prefix is what keeps
    the harness off every other container on the machine.
    """

    def __init__(self, name: str) -> None:
        if not name.startswith("leak-"):
            raise ValueError(f"leak-test containers are named leak-*, not {name!r}")
        self.name = name

    def _cli(self, *args: str, stdin: bytes | None = None, check: bool = True) -> bytes:
        done = subprocess.run(["docker", *args], input=stdin, capture_output=True)
        if check and done.returncode:
            error = done.stderr.decode(errors="replace").strip()
            raise RuntimeError(f"docker {args[0]} exited {done.returncode}: {error}")
        return done.stdout

    def run(self, image: str, token_file: Path) -> None:
        hosts = [f"--add-host={host}:127.0.0.1" for host in FIXTURE_HOSTS]
        # CLAW_LOG stays unset: the default level is the one the log channel is judged
        # at, and debug or trace dump run code and CDP frames by design.
        self._cli(
            *("run", "-d", "--name", self.name, "--shm-size=1g", "--hostname", "neo"),
            *("-p", "127.0.0.1::9010", "-p", "127.0.0.1::9300"),
            *("-v", f"{token_file.resolve()}:/run/neo-host-token:ro"),
            *("-e", "CLAW_HOST_TOKEN_FILE=/run/neo-host-token"),
            *("-e", "CLAW_HOST_TCP=0.0.0.0:9300"),
            *hosts,
            image,
        )

    def wait_healthy(self, timeout: float = 90) -> None:
        deadline = time.monotonic() + timeout
        while True:
            state = self._cli(
                "inspect", "-f", "{{.State.Status}} {{.State.Health.Status}}", self.name
            )
            status, health = state.decode().split()
            if health == "healthy":
                return
            if status in ("exited", "dead"):
                raise RuntimeError(f"{self.name} {status} before it was healthy")
            if time.monotonic() > deadline:
                raise TimeoutError(f"{self.name} not healthy in {timeout} s: {health}")
            time.sleep(0.5)

    def port(self, container_port: int) -> int:
        out = self._cli("port", self.name, f"{container_port}/tcp").decode()
        return int(out.split()[0].rsplit(":", 1)[1])

    def exec(self, *argv: str, stdin: bytes | None = None, check: bool = True) -> bytes:
        interactive = ["-i"] if stdin is not None else []
        return self._cli(
            "exec", *interactive, self.name, *argv, stdin=stdin, check=check
        )

    def exec_pipe(self, *argv: str) -> subprocess.Popen[bytes]:
        command = ["docker", "exec", "-i", self.name, *argv]
        return subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE)

    def cp_in(self, src: Path, dest: str) -> None:
        self._cli("cp", str(src), f"{self.name}:{dest}")

    def cp_out(self, src: str, dest: Path) -> None:
        self._cli("cp", f"{self.name}:{src}", str(dest))

    def logs(self) -> bytes:
        # One pipe for both streams keeps the order the CLI demultiplexed them in.
        command = ["docker", "logs", self.name]
        done = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        if done.returncode:
            raise RuntimeError(f"docker logs exited {done.returncode}")
        return done.stdout

    def stop(self, timeout: int = 20) -> None:
        self._cli("stop", "-t", str(timeout), self.name)

    def remove(self) -> None:
        self._cli("rm", "-f", self.name)

    def label(self, key: str) -> str | None:
        image = self._cli("inspect", "-f", "{{.Image}}", self.name).decode().strip()
        raw = self._cli("image", "inspect", "-f", "{{json .Config.Labels}}", image)
        return (json.loads(raw) or {}).get(key)


class Agent:
    """The agent's side of MCP: claw-sdk's one `run` tool, keeping every byte of a reply.

    smoke.McpClient does initialize and its notification. Runs are posted here instead:
    McpClient.call decodes the reply and raises on errors, and its `tool()` adds
    claw-server's agentName, which run refuses.
    """

    def __init__(self, url: str) -> None:
        self.client = McpClient(url)
        hello = {"name": "neo-leak-test", "version": "1"}
        self.client.call(
            "initialize",
            {"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": hello},
        )
        self.client.notify("notifications/initialized")

    def run(self, code: str, meta: dict, timeout_ms: int) -> tuple[str, bytes]:
        """Returns the JSON-RPC body sent and the raw reply body: the JSON, or the SSE
        frames up to the one holding the reply. An HTTP error status still returns its
        body; only a transport failure raises."""
        if timeout_ms > RUN_TIMEOUT_MAX_MS:
            raise ValueError(f"a run's timeout is at most {RUN_TIMEOUT_MAX_MS} ms")
        self.client.next_id += 1
        ident = self.client.next_id
        params = {
            "name": "run",
            "arguments": {"code": code, "timeout": timeout_ms},
            "_meta": {RUN_META: meta},
        }
        request = json.dumps(
            {"jsonrpc": "2.0", "id": ident, "method": "tools/call", "params": params}
        )
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        }
        if self.client.session_id:
            headers["mcp-session-id"] = self.client.session_id
            headers["mcp-protocol-version"] = PROTOCOL
        post = urllib.request.Request(self.client.url, request.encode(), headers)
        try:
            response = urllib.request.urlopen(post, timeout=MCP_READ_TIMEOUT_S)
        except urllib.error.HTTPError as refused:
            response = refused
        with response:
            if "event-stream" not in response.headers.get("content-type", ""):
                return request, response.read()
            raw = bytearray()
            parser = SseParser()
            for line in response:
                raw += line
                if any(_reply_id(event.data) == ident for event in parser.feed(line)):
                    break
        return request, bytes(raw)


def _reply_id(data: str | bytes) -> object:
    """The id of a JSON-RPC response (a result or an error), else None."""
    try:
        message = json.loads(data)
    except ValueError:
        return None
    if isinstance(message, dict) and ("result" in message or "error" in message):
        return message.get("id")
    return None


@dataclasses.dataclass(frozen=True)
class RunReply:
    """What one run answered, read from its raw reply.

    `error` is {"owner", "code", "message"} whichever layer answered with one: the run
    (structuredContent.error, else the run _meta error), MCP itself (a JSON-RPC error,
    owner "jsonrpc"), or bytes holding no reply at all (owner "harness", code "unparsed").
    """

    run_id: str | None
    status: str | None
    value: object
    text: str
    error: dict | None


def parse_run_reply(raw: bytes) -> RunReply:
    message = _last_response(raw)
    if message is None:
        return _unparsed(f"no reply in {len(raw)} bytes")
    try:
        if "error" in message:
            fault = message["error"]
            error = _error("jsonrpc", fault.get("code"), fault.get("message"))
            return RunReply(None, None, None, "", error)
        result = message["result"]
        meta = (result.get("_meta") or {}).get(RUN_META) or {}
        structured = result.get("structuredContent") or {}
        text = "\n".join(
            part.get("text", "")
            for part in result.get("content") or []
            if part.get("type") == "text"
        )
        fault = structured.get("error") or meta.get("error")
        error = None
        if fault:
            error = _error(fault.get("owner"), fault.get("code"), fault.get("message"))
        elif result.get("isError"):
            error = _error("mcp", "is_error", "isError without an error object")
        value = structured.get("value")
        return RunReply(meta.get("id"), meta.get("status"), value, text, error)
    except (AttributeError, TypeError):
        return _unparsed("a reply of another shape")


def _unparsed(why: str) -> RunReply:
    return RunReply(None, None, None, "", _error("harness", "unparsed", why))


def _error(owner: object, code: object, message: object) -> dict:
    return {"owner": str(owner), "code": str(code), "message": str(message)}


def _last_response(raw: bytes) -> dict | None:
    """The JSON-RPC response in a reply body: the body itself, or the last response among
    its SSE events. A final event cut before its blank line still counts."""
    if raw.lstrip()[:1] == b"{":
        candidates: list[str | bytes] = [raw]
    else:
        candidates = [event.data for event in SseParser().feed(raw + b"\n\n")]
    replies = []
    for candidate in candidates:
        try:
            message = json.loads(candidate)
        except ValueError:
            continue
        if isinstance(message, dict) and ("result" in message or "error" in message):
            replies.append(message)
    return replies[-1] if replies else None


@dataclasses.dataclass(frozen=True)
class Answer:
    """One whole host API answer. Header names are lower-cased."""

    status: int
    headers: dict[str, str]
    body: bytes


class Host:
    """The host API over TCP. Every request carries the bearer token unless told not to."""

    def __init__(self, port: int, token: str) -> None:
        self.port = port
        self.token = token

    def call(
        self,
        method: str,
        path: str,
        body: dict | None = None,
        *,
        token: bool = True,
        timeout: float = 70,
    ) -> Answer:
        """Any status comes back as an Answer; only a transport failure raises."""
        headers = {"Authorization": f"Bearer {self.token}"} if token else {}
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
        try:
            connection.request(method, path, body=data, headers=headers)
            response = connection.getresponse()
            answered = _header_dict(response.getheaders())
            return Answer(response.status, answered, response.read())
        finally:
            connection.close()

    def stream(
        self, path: str, blob: Path, *, headers: dict[str, str] | None = None
    ) -> StreamReader:
        reader = StreamReader(self.port, path, self.token, blob, headers)
        reader.start()
        return reader


class StreamReader(threading.Thread):
    """One long host answer (events SSE, MJPEG screencast, fMP4 video), on its own thread.

    The body is written to `blob` byte for byte as it arrives (de-chunked), with the host
    time each chunk arrived in `chunks` as (offset in blob, ms). The thread is the only
    writer of the blob and of these attributes; other threads read them, take data() or
    wait_bytes(), and call stop().
    The request goes out on construction, so a refused connection raises there. `status`,
    `headers` and `opened_ms` are set when the answer's head arrives, and `closed` when it
    ends: (ms, "eof" for a body that ended, "harness" after stop(), else the error's repr).
    """

    def __init__(
        self,
        port: int,
        path: str,
        token: str,
        blob: Path,
        headers: dict[str, str] | None = None,
    ) -> None:
        super().__init__(name=f"leak stream {path}", daemon=True)
        self.path = path
        self.blob = blob
        self.status: int | None = None
        self.headers: dict[str, str] = {}
        self.opened_ms = 0
        self.chunks: list[tuple[int, int]] = []
        self.closed: tuple[int, str] | None = None
        self._stopping = False
        self._written = 0
        self._changed = threading.Condition()
        self._connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
        self._connection.connect()
        # http.client drops its own reference when an answer will close the connection,
        # so stop() keeps one. A live answer can stay quiet for minutes (no event, a page
        # that does not repaint): only the connect is timed, and stop() ends any wait.
        self._socket = self._connection.sock
        self._socket.settimeout(None)
        sent = {"Authorization": f"Bearer {token}", **(headers or {})}
        self._connection.request("GET", path, headers=sent)
        self._out = blob.open("wb")

    def run(self) -> None:
        reason = "eof"
        try:
            response = self._connection.getresponse()
            self.headers = _header_dict(response.getheaders())
            self.opened_ms = _now_ms()
            self.status = response.status
            while chunk := response.read1(65536):
                arrived = _now_ms()
                # Whole and on disk before it is counted, so data() never reads short.
                self._out.write(chunk)
                self._out.flush()
                with self._changed:
                    self.chunks.append((self._written, arrived))
                    self._written += len(chunk)
                    self._changed.notify_all()
        except Exception as error:  # every end of the answer is recorded in `closed`
            reason = repr(error)
        finally:
            self._out.close()
        with self._changed:
            self.closed = (_now_ms(), "harness" if self._stopping else reason)
            self._changed.notify_all()

    def stop(self) -> None:
        self._stopping = True
        # shutdown, not close: closing a socket does not wake a recv blocked on it.
        try:
            self._socket.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.join(10)
        self._connection.close()

    def data(self) -> bytes:
        """Every byte received so far: exactly what `chunks` covers. The thread keeps no
        copy, so this reads the blob, which already holds each chunk before it counts."""
        received = self._written
        return self.blob.read_bytes()[:received]

    def wait_bytes(self, predicate: Callable[[bytes], bool], timeout: float) -> bool:
        """Waits until the bytes so far satisfy `predicate`; False at the timeout or once
        the answer ended without them."""
        deadline = time.monotonic() + timeout
        with self._changed:
            while not predicate(self.data()):
                left = deadline - time.monotonic()
                if self.closed is not None or left <= 0:
                    return False
                self._changed.wait(left)
        return True


class Display(threading.Thread):
    """/v1/display over a WebSocket, as an RFB 3.8 viewer with security None.

    The upgrade and the RFB handshake run in the constructor, which then starts the reading
    thread. It raises only when the connection or its upgrade fails at the socket. The
    viewer announces Raw alone (no extended clipboard, no pseudo-encoding), so every rect is
    Raw and every type-3 message is classic ServerCutText. `blob` gets every server byte
    after ServerInit, with arrival times in `chunks`, for RfbParser to replay. `opened_ms`
    is when ServerInit arrived, or when a refusal did. `closed` is (ms, why): "close <code>"
    (4001 paused, 4002 released, ...), "eof", "harness" after stop(), "http <status>" for a
    refused upgrade, or the repr of what ended the reading. A refused upgrade's HTTP body
    is that display's whole answer, so it goes into `blob` like the bytes of an open one:
    the judge searches it as it searches every answer.

    watch=True (a view display) asks for the whole screen every 300 ms, the first time
    non-incrementally so one whole frame always arrives. watch=False (a control display)
    asks for nothing: the owner's display is only a keyboard here.
    """

    def __init__(
        self, port: int, path: str, token: str, blob: Path, *, watch: bool
    ) -> None:
        super().__init__(name=f"leak display {path}", daemon=True)
        self.blob = blob
        self.width = self.height = 0
        self.pixel_format = b""
        self.chunks: list[tuple[int, int]] = []
        self.closed: tuple[int, str] | None = None
        self._stopping = False
        self._written = 0
        self._messages: list[RfbMessage] = []
        self._lock = threading.Lock()
        self._done = threading.Event()
        self._ws = WebSocket(
            "127.0.0.1",
            port,
            path,
            {"Authorization": f"Bearer {token}", "Sec-WebSocket-Protocol": "binary"},
        )
        self.status = self._ws.status
        self.opened_ms = _now_ms()
        self._out = blob.open("wb")
        if self.status != 101:
            length = re.search(r"(?im)^content-length:\s*(\d+)", self._ws.response)
            body = self._ws.read_rest(int(length[1]) if length else 65536)
            self._out.write(body)
            self.chunks.append((0, _now_ms()))
            self._written = len(body)
            self._end(f"http {self.status}")
            return
        try:
            self._ws.sock.settimeout(10)
            rest = self._handshake()
            self._ws.sock.settimeout(None)
            self.opened_ms = _now_ms()
            self._parser = RfbParser(self.width, self.height, self.pixel_format)
            if rest:
                self._record(rest)
        except EOFError:
            self._end(self._close_reason())
            return
        except ValueError as error:
            self._end(repr(error))
            return
        self.start()
        if watch:
            threading.Thread(target=self._refresh, daemon=True).start()

    def _handshake(self) -> bytes:
        """RFB 3.8 up to ServerInit, then SetEncodings [Raw]. Returns any server bytes
        that came after ServerInit in the same WebSocket message."""
        held = bytearray()

        def take(n: int) -> bytes:
            while len(held) < n:
                message = self._ws.recv()
                if message is None:
                    raise EOFError("the display closed during the RFB handshake")
                held.extend(message[1])
            out = bytes(held[:n])
            del held[:n]
            return out

        if (version := take(12)) != b"RFB 003.008\n":
            raise ValueError(f"the display speaks {version!r}")
        self._ws.send(version)
        if 1 not in take(take(1)[0]):
            raise ValueError("the display offers no security None")
        self._ws.send(b"\x01")
        if take(4) != b"\0\0\0\0":
            raise ValueError("the display refused security None")
        self._ws.send(b"\x01")  # ClientInit: shared, so no viewer disconnects another
        init = take(24)
        self.width, self.height = struct.unpack(">HH", init[:4])
        self.pixel_format = init[4:20]
        take(struct.unpack(">I", init[20:24])[0])  # the desktop name
        self._ws.send(struct.pack(">BxHi", 2, 1, 0))  # SetEncodings: Raw only
        return bytes(held)

    def _record(self, chunk: bytes) -> None:
        arrived = _now_ms()
        self._out.write(chunk)
        self._out.flush()
        self.chunks.append((self._written, arrived))
        self._written += len(chunk)
        parsed = self._parser.feed(chunk)
        with self._lock:
            self._messages.extend(parsed)

    def _close_reason(self) -> str:
        code = self._ws.close_code
        if self._stopping and code in (None, 1000):
            return "harness"  # our own close, or the display answering it
        return "eof" if code is None else f"close {code}"

    def _end(self, reason: str) -> None:
        self._out.close()
        self._done.set()
        self._ws.sock.close()
        self.closed = (_now_ms(), reason)

    def run(self) -> None:
        try:
            while (message := self._ws.recv()) is not None:
                self._record(message[1])
            reason = self._close_reason()
        except Exception as error:  # bytes RFB cannot frame, or a broken socket
            reason = repr(error)
        self._end(reason)

    def _refresh(self) -> None:
        incremental = 0
        while not self._done.is_set():
            request = struct.pack(
                ">BBHHHH", 3, incremental, 0, 0, self.width, self.height
            )
            try:
                self._ws.send(request)
            except OSError:
                return
            incremental = 1
            self._done.wait(VIEW_REFRESH_S)

    def key(self, keysym: int, down: bool) -> None:
        self._ws.send(struct.pack(">BBxxI", 4, down, keysym))

    def type_text(self, text: str, delay: float = KEY_DELAY_S) -> None:
        """Presses and releases each character; keysym = its Latin-1 code point. The text
        is checked whole first, so a bad character types nothing, and the error never
        quotes the text: it may be a planted value."""
        for char in text:
            if not (0x20 <= ord(char) <= 0x7E or 0xA0 <= ord(char) <= 0xFF):
                raise ValueError(
                    f"type_text takes printable Latin-1, not U+{ord(char):04X}"
                )
        for char in text:
            self.key(ord(char), True)
            time.sleep(delay)
            self.key(ord(char), False)
            time.sleep(delay)

    def press(self, keysym: int) -> None:
        self.chord(keysym)

    def chord(self, *keysyms: int) -> None:
        for keysym in keysyms:
            self.key(keysym, True)
            time.sleep(KEY_DELAY_S)
        for keysym in reversed(keysyms):
            self.key(keysym, False)
            time.sleep(KEY_DELAY_S)

    def messages(self) -> list[RfbMessage]:
        with self._lock:
            return list(self._messages)

    def stop(self) -> None:
        self._stopping = True
        self._done.set()
        try:
            self._ws.send(struct.pack(">H", 1000), opcode=8)
        except OSError:
            pass
        # shutdown, not close: closing a socket does not wake a recv blocked on it.
        try:
            self._ws.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        if self.is_alive():
            self.join(10)
        self._ws.sock.close()


@dataclasses.dataclass(frozen=True)
class SseEvent:
    """One dispatched event. `offset` is where its first line starts in the stream. `id`
    and `type` are what this event's own lines carried (None when absent), not the spec's
    sticky last-event id, so each event records exactly what was sent with it."""

    offset: int
    id: str | None
    type: str | None
    data: str


# CRLF before CR, so the pair is one line end.
_LINE_END = re.compile(rb"\r\n|\n|\r")


class SseParser:
    """text/event-stream, incrementally: an event is dispatched at its blank line if it
    had a data line; data lines join with "\\n"; comment lines are skipped."""

    def __init__(self) -> None:
        self._held = b""
        self._at = 0  # stream offset of _held[0]
        self._start: int | None = None
        self._id: str | None = None
        self._type: str | None = None
        self._data: list[str] = []

    def feed(self, data: bytes) -> list[SseEvent]:
        held = self._held + data
        events = []
        position = 0
        while end := _LINE_END.search(held, position):
            if end[0] == b"\r" and end.end() == len(held):
                break  # maybe the first half of a CRLF the next feed completes
            line = held[position : end.start()]
            line_at = self._at + position
            position = end.end()
            if not line:
                if self._data:
                    text = "\n".join(self._data)
                    events.append(SseEvent(self._start, self._id, self._type, text))
                self._start, self._id, self._type, self._data = None, None, None, []
                continue
            if self._start is None:
                self._start = line_at
            name, _, value = line.decode(errors="replace").partition(":")
            value = value.removeprefix(" ")
            if name == "data":
                self._data.append(value)
            elif name == "event":
                self._type = value
            elif name == "id":
                self._id = value
        self._held = held[position:]
        self._at += position
        return events


@dataclasses.dataclass(frozen=True)
class MjpegPart:
    """One multipart part. `offset` is where its boundary line starts in the stream;
    header names are lower-cased."""

    offset: int
    headers: dict[str, str]
    jpeg: bytes

    @property
    def frame_at(self) -> int | None:
        value = self.headers.get("x-frame-at", "")
        return int(value) if value.isdigit() else None


class MjpegParser:
    """multipart/x-mixed-replace, incrementally. Each body is sized by its Content-Length,
    never by searching for the next boundary, which JPEG bytes may contain."""

    def __init__(self, boundary: str = "frame") -> None:
        self._delimiter = b"--" + boundary.encode()
        self._held = bytearray()
        self._at = 0  # stream offset of _held[0]
        # (offset, headers, Content-Length) of a part whose body is still to come
        self._part: tuple[int, dict[str, str], int] | None = None

    def feed(self, data: bytes) -> list[MjpegPart]:
        self._held += data
        parts = []
        while True:
            if self._part:
                offset, headers, length = self._part
                if len(self._held) < length:
                    break
                parts.append(MjpegPart(offset, headers, bytes(self._held[:length])))
                self._take(length)
                self._part = None
                continue
            start = self._held.find(self._delimiter)
            end = self._held.find(b"\r\n\r\n", start) if start >= 0 else -1
            if end < 0:
                break
            lines = self._held[start:end].decode("latin-1").split("\r\n")[1:]
            headers = _header_dict([line.partition(":")[::2] for line in lines])
            length = headers.get("content-length", "")
            if not length.isdigit():
                raise ValueError(f"an MJPEG part at {self._at + start} has no length")
            self._part = (self._at + start, headers, int(length))
            self._take(end + 4)
        return parts

    def _take(self, n: int) -> None:
        del self._held[:n]
        self._at += n


@dataclasses.dataclass(frozen=True)
class RfbMessage:
    """One server-to-client RFB message. `offset` is its type byte's offset in the bytes
    after ServerInit. A FramebufferUpdate (0) lists its rects as (x, y, w, h, encoding);
    a ServerCutText (3) carries its text."""

    offset: int
    type: int
    rects: tuple[tuple[int, int, int, int, int], ...] = ()
    text: bytes | None = None


class RfbParser:
    """Server-to-client RFB after ServerInit, incrementally: 0 FramebufferUpdate, 1
    SetColourMapEntries, 2 Bell, 3 ServerCutText.

    Only Raw was announced, so a rect's pixels are w*h*bytes-per-pixel and are skipped,
    never held. Anything else (another encoding, another message type) means the server
    broke the handshake's terms; its length is unknown, so the parser raises ValueError
    rather than lose its place.
    """

    def __init__(self, width: int, height: int, pixel_format: bytes) -> None:
        self.width, self.height = width, height
        self._pixel_bytes = pixel_format[0] // 8
        self._held = bytearray()
        self._at = 0  # stream offset of _held[0]
        self._skip = 0  # pixel bytes of the current rect still to come
        self._update: tuple[int, int, list] | None = None  # offset, rects left, rects

    def feed(self, data: bytes) -> list[RfbMessage]:
        self._held += data
        out = []
        while True:
            if self._skip:
                n = min(self._skip, len(self._held))
                self._take(n)
                self._skip -= n
                if self._skip:
                    break
            if self._update:
                offset, left, rects = self._update
                if not left:
                    out.append(RfbMessage(offset, 0, tuple(rects)))
                    self._update = None
                    continue
                if len(self._held) < 12:
                    break
                x, y, w, h, encoding = struct.unpack_from(">HHHHi", self._held)
                if encoding != 0:
                    raise ValueError(f"encoding {encoding} at {self._at}, not Raw")
                rects.append((x, y, w, h, encoding))
                self._update = (offset, left - 1, rects)
                self._take(12)
                self._skip = w * h * self._pixel_bytes
                continue
            if not self._held:
                break
            kind, offset = self._held[0], self._at
            if kind == 0:
                if len(self._held) < 4:
                    break
                self._update = (offset, struct.unpack_from(">H", self._held, 2)[0], [])
                self._take(4)
            elif kind == 1:
                if len(self._held) < 6:
                    break
                size = 6 + 6 * struct.unpack_from(">H", self._held, 4)[0]
                if len(self._held) < size:
                    break
                out.append(RfbMessage(offset, 1))
                self._take(size)
            elif kind == 2:
                out.append(RfbMessage(offset, 2))
                self._take(1)
            elif kind == 3:
                if len(self._held) < 8:
                    break
                # A negative length is the extended-clipboard form, which only a
                # viewer that announced it may get. Framed anyway: type 3 is the
                # finding either way.
                size = 8 + abs(struct.unpack_from(">i", self._held, 4)[0])
                if len(self._held) < size:
                    break
                out.append(RfbMessage(offset, 3, text=bytes(self._held[8:size])))
                self._take(size)
            else:
                raise ValueError(f"RFB message type {kind} at {offset}")
        return out

    def _take(self, n: int) -> None:
        del self._held[:n]
        self._at += n


def mp4_boxes(data: bytes) -> list[tuple[int, str, int]]:
    """Top-level (offset, type, size) of an MP4 stream. The last box may run past the end
    of `data` when the stream was cut mid-box."""
    boxes = []
    offset = 0
    while offset + 8 <= len(data):
        size, kind = struct.unpack_from(">I4s", data, offset)
        if size == 1:
            if offset + 16 > len(data):
                break
            size = struct.unpack_from(">Q", data, offset + 8)[0]
        elif size == 0:
            size = len(data) - offset
        if size < 8:
            break
        boxes.append((offset, kind.decode("latin-1"), size))
        offset += size
    return boxes
