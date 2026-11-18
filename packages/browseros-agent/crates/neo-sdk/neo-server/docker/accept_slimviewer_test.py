"""Exercise the spectator probe against length-prefixed data from a local socket."""

import io
import socket
import struct
import unittest
from unittest.mock import patch

import accept_slimviewer as viewer


def record(payload):
    return struct.pack('>I', len(payload)) + payload


class SpectatorTest(unittest.TestCase):
    def probe(self, hello, video):
        client, server = socket.socketpair()
        with client, server, patch.object(socket, 'create_connection', return_value=client):
            header = struct.pack('>BIBHH', 2, 37, 1, 640, 480)
            server.sendall(record(hello) + record(header + video))
            server.shutdown(socket.SHUT_WR)
            return viewer.spectator_frame(), server.recv(9)

    def test_acknowledges_an_independently_decodable_spectator_frame(self):
        units = b'\0\0\0\1\x67\x42\0\0\1\x68\xce\0\0\1\x65\x88'
        self.assertEqual(self.probe(b'\1\1\1', units), ('640x480', struct.pack('>IBI', 5, 0x10, 37)))

    def test_rejects_an_owner_greeting_on_the_spectator_port(self):
        with self.assertRaisesRegex(ValueError, 'spectator greeting'):
            self.probe(b'\1\1\0', b'')

    def test_byte_stream_with_a_start_code_but_no_keyframe_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'H.264 keyframe'):
            self.probe(b'\1\1\1', b'\0\0\1\x61')

    def test_rejects_a_truncated_or_excessive_record_before_parsing_it(self):
        for data in (b'\0', struct.pack('>I', 5) + b'a', struct.pack('>I', 2**32 - 1)):
            with self.subTest(data=data), self.assertRaises(ValueError):
                viewer.record(io.BytesIO(data))
