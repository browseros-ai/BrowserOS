"""Offline checks of leak_wire: any split of a stream parses the same, a run reply always
parses, garbage included, and the container starts as the plan expects it."""

import json
import socket
import struct
import tempfile
import threading
import unittest
import urllib.parse
from pathlib import Path

from leak_plan import NEO_FILES, PLACES, NeoFile
from leak_wire import (
    FIXTURE_HOSTS,
    Display,
    Docker,
    MjpegParser,
    MjpegPart,
    RfbMessage,
    RfbParser,
    RunReply,
    SseEvent,
    SseParser,
    mp4_boxes,
    parse_run_reply,
)


def splits(data: bytes):
    """Every way a test cuts a stream: whole, in two at each byte, and byte by byte."""
    yield [data]
    for cut in range(1, len(data)):
        yield [data[:cut], data[cut:]]
    yield [data[i : i + 1] for i in range(len(data))]


def fed(parser, chunks: list[bytes]) -> list:
    return [item for chunk in chunks for item in parser.feed(chunk)]


KEEPALIVE = b": keep-alive\n\n"
STARTED = b'id: 7-1\nevent: run.started\ndata: {"a":1}\n\n'
ENDED = b"id: 7-2\r\nevent: run.ended\r\ndata: line one\r\ndata:line two\r\n\r\n"
PRIMING = b"data: \n\n"
UNFINISHED = b"data: not yet\n"
SSE = KEEPALIVE + STARTED + ENDED + PRIMING + UNFINISHED
SSE_EVENTS = [
    SseEvent(len(KEEPALIVE), "7-1", "run.started", '{"a":1}'),
    SseEvent(len(KEEPALIVE + STARTED), "7-2", "run.ended", "line one\nline two"),
    SseEvent(len(KEEPALIVE + STARTED + ENDED), None, None, ""),
]


def mjpeg_part(jpeg: bytes, url: str, frame_at: int) -> bytes:
    """claw-sdk's part layout (neo-service live_tabs.rs, mjpeg_part)."""
    head = (
        f"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {len(jpeg)}\r\n"
        f"X-Tab-Id: 42\r\nX-Url: {url}\r\nX-Frame-At: {frame_at}\r\n\r\n"
    )
    return head.encode() + jpeg + b"\r\n"


# The first JPEG holds a whole fake part: only its Content-Length ends it in the right place.
JPEG_1 = b"\xff\xd8\r\n--frame\r\nContent-Length: 1\r\n\r\nX\xff\xd9"
JPEG_2 = b"\xff\xd8\xff\xd9"
URL_1 = "http://a.test:8800/fx/i01/main?k=%5Bprivate%5D"
URL_2 = "http://a.test:8800/fx/i01/main"
PART_1 = mjpeg_part(JPEG_1, URL_1, 1800000000123)
MJPEG = PART_1 + mjpeg_part(JPEG_2, URL_2, 1800000000456)


def part_headers(jpeg: bytes, url: str, frame_at: int) -> dict[str, str]:
    return {
        "content-type": "image/jpeg",
        "content-length": str(len(jpeg)),
        "x-tab-id": "42",
        "x-url": url,
        "x-frame-at": str(frame_at),
    }


# 32 bits per pixel, depth 24, little-endian true colour: Xvnc's default.
PIXEL_FORMAT = struct.pack(">BBBBHHHBBBxxx", 32, 24, 0, 1, 255, 255, 255, 16, 8, 0)
# Pixels made of message-type bytes, so only the rects' sizes can skip them.
UPDATE = (
    struct.pack(">BxH", 0, 2)
    + struct.pack(">HHHHi", 0, 0, 2, 1, 0)
    + bytes([3, 2, 1, 0] * 2)
    + struct.pack(">HHHHi", 5, 6, 1, 2, 0)
    + bytes([2, 3, 0, 0] * 2)
)
BELL = b"\x02"
CUT = struct.pack(">BxxxI", 3, 6) + b"copied"
COLOURS = struct.pack(">BxHH", 1, 0, 1) + b"\xff\xff\x00\x00\x00\x00"
NO_RECTS = struct.pack(">BxH", 0, 0)
RFB = UPDATE + BELL + CUT + COLOURS + NO_RECTS
RFB_MESSAGES = [
    RfbMessage(0, 0, ((0, 0, 2, 1, 0), (5, 6, 1, 2, 0))),
    RfbMessage(len(UPDATE), 2),
    RfbMessage(len(UPDATE + BELL), 3, text=b"copied"),
    RfbMessage(len(UPDATE + BELL + CUT), 1),
    RfbMessage(len(UPDATE + BELL + CUT + COLOURS), 0),
]

# The result shapes claw-sdk answered with on neo-leak:0.52.1.2-ec1ded9.
RUN_OK = {
    "content": [{"type": "text", "text": '{"n":42}'}],
    "structuredContent": {"value": {"n": 42}},
    "isError": False,
    "_meta": {
        "com.browseros.neo/run": {
            "effects": "none",
            "id": "019a-run",
            "status": "ok",
            "unknown": [],
        }
    },
}
HELD = {
    "code": "method_unavailable",
    "delivery": "not_sent",
    "message": "`page.evaluate` is held while a privacy fix lands.",
    "operationId": "019b-run.1",
    "owner": "script",
    "retryable": True,
}
RUN_HELD = {
    "content": [
        {
            "type": "text",
            "text": "script / method_unavailable: `page.evaluate` is held while a "
            "privacy fix lands. (op 1, delivery not_sent)",
        }
    ],
    "structuredContent": {"error": HELD},
    "isError": True,
    "_meta": {
        "com.browseros.neo/run": {
            "effects": "none",
            "error": HELD,
            "id": "019b-run",
            "status": "error",
            "unknown": [],
        }
    },
}


def response(result: dict) -> bytes:
    return json.dumps({"jsonrpc": "2.0", "id": 2, "result": result}).encode()


class SseParserTest(unittest.TestCase):
    def test_any_split_gives_the_same_events(self):
        for index, chunks in enumerate(splits(SSE)):
            with self.subTest(split=index):
                self.assertEqual(fed(SseParser(), chunks), SSE_EVENTS)


class MjpegParserTest(unittest.TestCase):
    def test_any_split_gives_the_same_parts(self):
        expected = [
            MjpegPart(0, part_headers(JPEG_1, URL_1, 1800000000123), JPEG_1),
            MjpegPart(len(PART_1), part_headers(JPEG_2, URL_2, 1800000000456), JPEG_2),
        ]
        for index, chunks in enumerate(splits(MJPEG)):
            with self.subTest(split=index):
                self.assertEqual(fed(MjpegParser(), chunks), expected)

    def test_frame_at_is_the_capture_time(self):
        (first, second) = MjpegParser().feed(MJPEG)
        self.assertEqual(
            (first.frame_at, second.frame_at), (1800000000123, 1800000000456)
        )


class RfbParserTest(unittest.TestCase):
    def test_any_split_gives_the_same_messages(self):
        for index, chunks in enumerate(splits(RFB)):
            with self.subTest(split=index):
                parser = RfbParser(1280, 800, PIXEL_FORMAT)
                self.assertEqual(fed(parser, chunks), RFB_MESSAGES)

    def test_an_encoding_nobody_announced_is_refused(self):
        copy_rect = struct.pack(">BxH", 0, 1) + struct.pack(">HHHHi", 0, 0, 8, 8, 1)
        with self.assertRaises(ValueError):
            RfbParser(1280, 800, PIXEL_FORMAT).feed(copy_rect)


class RunReplyTest(unittest.TestCase):
    def test_a_json_body(self):
        reply = parse_run_reply(response(RUN_OK))
        self.assertEqual(reply, RunReply("019a-run", "ok", {"n": 42}, '{"n":42}', None))

    def test_sse_frames_after_the_priming_frame(self):
        # rmcp may prime a stream with an empty data frame; claw-sdk's reply frame is
        # one data line, no event line.
        raw = b"data: \n\n" + b"data: " + response(RUN_OK) + b"\n\n"
        reply = parse_run_reply(raw)
        self.assertEqual(reply, RunReply("019a-run", "ok", {"n": 42}, '{"n":42}', None))

    def test_a_run_that_failed(self):
        reply = parse_run_reply(response(RUN_HELD))
        self.assertEqual(
            (reply.run_id, reply.status, reply.value), ("019b-run", "error", None)
        )
        self.assertEqual(
            reply.error,
            {
                "owner": "script",
                "code": "method_unavailable",
                "message": "`page.evaluate` is held while a privacy fix lands.",
            },
        )

    def test_a_json_rpc_error(self):
        raw = json.dumps(
            {
                "jsonrpc": "2.0",
                "id": 2,
                "error": {"code": -32602, "message": "bad params"},
            }
        ).encode()
        self.assertEqual(
            parse_run_reply(raw).error,
            {"owner": "jsonrpc", "code": "-32602", "message": "bad params"},
        )

    def test_garbage_never_raises(self):
        for raw in (
            b"",
            b"<html>502 Bad Gateway</html>",
            b'{"jsonrpc": "2.0", "id": 2, "result": [1]}',
            b"data: {not json\n\n",
        ):
            with self.subTest(raw=raw):
                reply = parse_run_reply(raw)
                self.assertEqual((reply.run_id, reply.value), (None, None))
                self.assertEqual(
                    (reply.error["owner"], reply.error["code"]), ("harness", "unparsed")
                )


class Mp4BoxesTest(unittest.TestCase):
    def test_top_level_boxes_with_a_large_size_and_a_cut_tail(self):
        ftyp = struct.pack(">I4s", 16, b"ftyp") + b"iso6\0\0\0\0"
        moov = struct.pack(">I4s", 12, b"moov") + b"\0" * 4
        mdat = struct.pack(">I4sQ", 1, b"mdat", 20) + b"abcd"
        cut = struct.pack(">I4s", 100, b"moof") + b"\0" * 10
        self.assertEqual(
            mp4_boxes(ftyp + moov + mdat + cut),
            [(0, "ftyp", 16), (16, "moov", 12), (28, "mdat", 20), (48, "moof", 100)],
        )


class DisplayRefusalTest(unittest.TestCase):
    def test_a_refused_display_records_the_refusal_body(self):
        """A display Neo refuses (409 display_paused) answers with an HTTP body
        instead of RFB. That body is the channel's answer: it lands in the blob, where
        the judge searches it like any other answer."""
        body = b'{"error":{"code":"display_paused"}}'
        server = socket.create_server(("127.0.0.1", 0))

        def refuse() -> None:
            conn, _ = server.accept()
            with conn:
                head = b""
                while b"\r\n\r\n" not in head:
                    head += conn.recv(4096)
                conn.sendall(
                    b"HTTP/1.1 409 Conflict\r\nContent-Type: application/json\r\n"
                    b"Content-Length: %d\r\n\r\n%s" % (len(body), body)
                )

        thread = threading.Thread(target=refuse)
        thread.start()
        with server, tempfile.TemporaryDirectory() as root:
            blob = Path(root) / "view.bin"
            display = Display(
                server.getsockname()[1], "/v1/display?mode=view", "t", blob, watch=True
            )
            thread.join()
            self.assertEqual((display.status, display.closed[1]), (409, "http 409"))
            self.assertEqual(blob.read_bytes(), body)
            self.assertEqual([offset for offset, _ in display.chunks], [0])


class FixtureHostsTest(unittest.TestCase):
    def test_the_container_resolves_every_fixture_host(self):
        self.assertEqual(set(FIXTURE_HOSTS), {place.host for place in PLACES.values()})


class DockerRunTest(unittest.TestCase):
    def test_neos_secret_files_sit_where_the_deny_list_probes_name_them(self):
        """The host token and the fake CAPTCHA key are mounted read-only at the paths
        the deny-list probes upload (leak_plan.NEO_FILES), each named by its variable as
        Neo reads it, and the key's provider is a loopback port nothing serves, so the
        fake key never leaves the container."""
        sent = []

        class Recording(Docker):
            def _cli(self, *args, stdin=None, check=True):
                sent.append(args)
                return b""

        Recording("leak-test").run("img", Path("/h/token"), Path("/h/key"))
        (argv,) = sent
        pairs = list(zip(argv, argv[1:]))
        mounts = {value for flag, value in pairs if flag == "-v"}
        env = dict(value.split("=", 1) for flag, value in pairs if flag == "-e")
        token, key = NEO_FILES[NeoFile.HOST_TOKEN], NEO_FILES[NeoFile.CAPTCHA_KEY]
        self.assertEqual(mounts, {f"/h/token:{token}:ro", f"/h/key:{key}:ro"})
        for mount in mounts:
            self.assertFalse(
                mount.split(":")[1].startswith("/home/neo/"),
                "the entrypoint chowns /home/neo: a read-only mount there stops Neo",
            )
        self.assertEqual(env["CLAW_HOST_TOKEN_FILE"], token)
        self.assertEqual(env["BROWSEROS_CAPSOLVER_API_KEY_FILE"], key)
        provider = urllib.parse.urlsplit(env["BROWSEROS_CAPSOLVER_API_URL"])
        self.assertEqual(provider.hostname, "127.0.0.1")


if __name__ == "__main__":
    unittest.main()
