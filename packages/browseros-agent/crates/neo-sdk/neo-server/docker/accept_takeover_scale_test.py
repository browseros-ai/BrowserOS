"""Check image-proof parsers without starting a browser, container, or external dependency."""

import base64
import hashlib
import struct
import time
import unittest
import zlib

from accept_takeover_scale import COLORS, Rfb, WebSocket, fresh_events, jpeg_size, png_pixels, samples


def chunk(kind, body):
    return struct.pack("!I", len(body)) + kind + body + struct.pack("!I", zlib.crc32(kind + body))


def png(channels, filtering, density=1):
    width, height = 80 * density, 80 * density
    points = dict(zip([(x * density, y * density) for x, y in samples(80, 80)], COLORS))
    previous, rows = bytes(width * channels), bytearray()
    for y in range(height):
        raw = bytes(channel for x in range(width) for channel in
                    (*points.get((x, y), (x % 256, y % 256, (x + y) % 256)), *([255] if channels == 4 else [])))
        row = bytearray([filtering])
        for at, value in enumerate(raw):
            left = raw[at - channels] if at >= channels else 0
            above = previous[at]
            corner = previous[at - channels] if at >= channels else 0
            paeth = min((left, above, corner), key=lambda candidate: abs(left + above - corner - candidate))
            predictor = (0, left, above, (left + above) // 2, paeth)[filtering]
            row.append((value - predictor) % 256)
        rows.extend(row)
        previous = raw
    header = struct.pack("!IIBBBBB", width, height, 8, 2 if channels == 3 else 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


class Connection:
    """Split upgrade/frame bytes across reads, including a first frame in the HTTP read."""

    def __init__(self):
        self.incoming, self.sent = bytearray(), []

    def settimeout(self, _timeout):
        pass

    def sendall(self, data):
        self.sent.append(data)
        if data.startswith(b"GET "):
            key = next(line.split(b": ", 1)[1] for line in data.split(b"\r\n") if line.startswith(b"Sec-WebSocket-Key:"))
            accept = base64.b64encode(hashlib.sha1(key + b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest())
            self.incoming.extend(b"HTTP/1.1 101 Switching Protocols\r\nSec-WebSocket-Accept: " + accept + b"\r\n\r\n"
                                 b"\x89\x01p\x02\x03abc\x80\x03def")

    def recv(self, count):
        result = bytes(self.incoming[:min(count, 11)])
        del self.incoming[:len(result)]
        return result

    def close(self):
        pass


class ProofParserTests(unittest.TestCase):
    def test_png_samples_reverse_every_filter_in_rgb_and_rgba(self):
        for channels in (3, 4):
            for filtering in range(5):
                with self.subTest(channels=channels, filtering=filtering):
                    decoded = png_pixels(png(channels, filtering), 1)
                    self.assertEqual((decoded["width"], decoded["height"]), (80, 80))
                    self.assertEqual(decoded["pixels"], [list(color) for color in COLORS])

    def test_png_rejects_corrupted_artifact(self):
        data = bytearray(png(3, 0))
        data[20] ^= 1
        with self.assertRaisesRegex(AssertionError, "CRC"):
            png_pixels(data, 1)

    def test_png_samples_use_css_positions_at_device_density(self):
        decoded = png_pixels(png(4, 4, 2), 2)
        self.assertEqual((decoded["width"], decoded["height"]), (160, 160))
        self.assertEqual(decoded["pixels"], [list(color) for color in COLORS])

    def test_jpeg_dimensions_come_from_frame_not_application_metadata(self):
        app = b"\xff\xe1\x00\x05abc"
        frame = b"\xff\xc2" + struct.pack("!HBHH", 8, 8, 1400, 732) + b"\x03"
        self.assertEqual(jpeg_size(b"\xff\xd8" + app + frame), {"width": 732, "height": 1400})

    def test_websocket_preserves_fragments_and_masks_each_client_length_form(self):
        connection = Connection()
        websocket = WebSocket(connection, "neo", "/v1/display?mode=view")
        deadline = time.monotonic() + 1
        self.assertEqual(websocket.receive(deadline), (2, b"abc", False))
        self.assertEqual(websocket.receive(deadline), (0, b"def", True))
        for length in (3, 126, 65536):
            payload = bytes(index % 251 for index in range(length))
            websocket.send(2, payload)
            frame = connection.sent[-1]
            self.assertEqual(frame[0], 0x82)
            self.assertTrue(frame[1] & 0x80)
            offset = 2 if length < 126 else 4 if length < 65536 else 10
            mask = frame[offset:offset + 4]
            decoded = bytes(value ^ mask[index % 4] for index, value in enumerate(frame[offset + 4:]))
            self.assertEqual(decoded, payload)

    def test_unchanged_rfb_layout_consumes_a_fresh_framebuffer_update(self):
        display = Rfb.__new__(Rfb)
        display.current_layout = {"width": 732, "height": 1400, "reason": 2, "status": 0}
        display.bytes_per_pixel = 4
        incoming = bytearray(struct.pack("!BBHHHHHi", 0, 0, 1, 0, 0, 1, 1, 0) + b"\0" * 4)
        sent = []

        def read(count, _deadline):
            answer = bytes(incoming[:count])
            del incoming[:count]
            return answer

        display.read, display.send = read, sent.append
        measured = display.layout(732, 1400)
        self.assertTrue(measured["confirmedByUpdate"])
        self.assertFalse(incoming)
        self.assertEqual(sent, [struct.pack("!BBHHHH", 3, 0, 0, 0, 1, 1)])

    def test_event_observations_survive_a_full_fixture_ring(self):
        value = {"events": [{"seq": serial} for serial in range(81, 161)]}
        self.assertEqual(fresh_events(value, 158), [{"seq": 159}, {"seq": 160}])


if __name__ == "__main__":
    unittest.main()
