#!/usr/bin/env python3
"""Real X11 selection + TCP test. Run only in a disposable Neo, as its Xvnc user.

DISPLAY=:1 python3 tests/desktop-clipboard-x11.py
Needs the running slimviewer on localhost:5910/5911 and system libX11, no pip packages.
"""
import ctypes as C
import ctypes.util
import json
import os
import select
import socket
import struct
import time

X = C.CDLL(ctypes.util.find_library('X11'))
L = C.c_ulong
P = C.c_void_p


class SelectionRequest(C.Structure):
    _fields_ = [('type', C.c_int), ('serial', L), ('send_event', C.c_int),
                ('display', P), ('owner', L), ('requestor', L), ('selection', L),
                ('target', L), ('property', L), ('time', L)]


class SelectionNotify(C.Structure):
    _fields_ = [('type', C.c_int), ('serial', L), ('send_event', C.c_int),
                ('display', P), ('requestor', L), ('selection', L), ('target', L),
                ('property', L), ('time', L)]


class Property(C.Structure):
    _fields_ = [('type', C.c_int), ('serial', L), ('send_event', C.c_int),
                ('display', P), ('window', L), ('atom', L), ('time', L), ('state', C.c_int)]


class Event(C.Union):
    _fields_ = [('type', C.c_int), ('request', SelectionRequest),
                ('selection', SelectionNotify), ('property', Property), ('pad', C.c_long * 24)]


def api(name, result, *args):
    fn = getattr(X, name)
    fn.restype, fn.argtypes = result, list(args)
    return fn


open_display = api('XOpenDisplay', P, C.c_char_p)
root_window = api('XDefaultRootWindow', L, P)
create_window = api('XCreateSimpleWindow', L, P, L, C.c_int, C.c_int, C.c_uint, C.c_uint, C.c_uint, L, L)
intern = api('XInternAtom', L, P, C.c_char_p, C.c_int)
set_owner = api('XSetSelectionOwner', C.c_int, P, L, L, L)
change_property = api('XChangeProperty', C.c_int, P, L, L, L, C.c_int, C.c_int, P, C.c_int)
send_event = api('XSendEvent', C.c_int, P, L, C.c_int, C.c_long, C.POINTER(Event))
select_input = api('XSelectInput', C.c_int, P, L, C.c_long)
pending = api('XPending', C.c_int, P)
next_event = api('XNextEvent', C.c_int, P, C.POINTER(Event))
flush = api('XFlush', C.c_int, P)
connection = api('XConnectionNumber', C.c_int, P)
destroy = api('XDestroyWindow', C.c_int, P, L)
close = api('XCloseDisplay', C.c_int, P)
# Cancelled requests destroy their private window. The simulated owner may see BadWindow.
error_handler = C.CFUNCTYPE(C.c_int, P, P)(lambda _display, _error: 0)
api('XSetErrorHandler', P, P)(C.cast(error_handler, P))

display = open_display(os.getenv('DISPLAY', ':1').encode())
assert display, 'Cannot open disposable X display'
window = create_window(display, root_window(display), 0, 0, 1, 1, 0, 0, 0)
clip, utf8, incr = [intern(display, name, 0) for name in (b'CLIPBOARD', b'UTF8_STRING', b'INCR')]
peers, transfers = [], {}
payload, mode, announced, paused = b'', 'direct', 0, False


class Peer:
    def __init__(self, port):
        self.sock = socket.create_connection(('127.0.0.1', port))
        self.sock.setblocking(False)
        self.buffer = bytearray()
        self.clips, self.hellos = [], []
        self.closed = False
        peers.append(self)

    def send(self, kind, data=b''):
        self.sock.sendall(struct.pack('>I', len(data) + 1) + bytes([kind]) + data)

    def read(self):
        chunk = self.sock.recv(65536)
        if not chunk:
            self.closed = True
            return
        self.buffer.extend(chunk)
        while len(self.buffer) >= 4:
            size, = struct.unpack('>I', self.buffer[:4])
            assert 0 < size <= 16 * 1024 * 1024
            if len(self.buffer) < size + 4:
                break
            record = bytes(self.buffer[4:size + 4])
            del self.buffer[:size + 4]
            if record[0] == 1:
                self.hellos.append(record[2])
            elif record[0] == 2:
                self.send(0x10, record[1:5])
            elif record[0] == 3:
                self.clips.append(record[1:])


def put(requestor, property_atom, kind, bits, data):
    if bits == 32:
        value = L(data)
        change_property(display, requestor, property_atom, kind, bits, 0, C.byref(value), 1)
    else:
        value = C.create_string_buffer(data)
        change_property(display, requestor, property_atom, kind, bits, 0, value, len(data))


def events():
    while pending(display):
        event = Event()
        next_event(display, C.byref(event))
        if event.type == 30:
            req = event.request
            if req.target != utf8 or mode == 'hang':
                continue
            if mode == 'incr':
                select_input(display, req.requestor, 1 << 22)
                transfers[req.requestor, req.property] = [payload[i:i + 4096] for i in range(0, len(payload), 4096)] + [b'']
                put(req.requestor, req.property, incr, 32, announced)
            else:
                put(req.requestor, req.property, utf8, 8, payload)
            reply = Event()
            reply.selection = SelectionNotify(31, 0, 1, display, req.requestor, req.selection, req.target, req.property, req.time)
            send_event(display, req.requestor, 0, 0, C.byref(reply))
        elif event.type == 28 and event.property.state == 1:
            key = event.property.window, event.property.atom
            if key in transfers and not paused:
                chunks = transfers[key]
                put(*key, utf8, 8, chunks.pop(0))
                if not chunks:
                    del transfers[key]
    flush(display)


def pump(seconds=0.05, until=lambda: False):
    end = time.monotonic() + seconds
    while time.monotonic() < end and not until():
        events()
        sockets = {p.sock: p for p in peers if not p.closed}
        ready, _, _ = select.select([connection(display), *sockets], [], [], min(0.01, max(0, end - time.monotonic())))
        for fd in ready:
            if fd in sockets:
                sockets[fd].read()
    events()
    return until()


def publish(text, kind='direct', size=None):
    global payload, mode, announced
    payload, mode = text, kind
    announced = len(text) if size is None else size
    set_owner(display, clip, window, 0)
    flush(display)


def check(name, peer, text, kind='direct', size=None, accepted=True):
    before = len(peer.clips)
    publish(text, kind, size)
    if accepted:
        assert pump(2, lambda: len(peer.clips) > before), name + ': missing copy'
        assert peer.clips[-1] == text, name + ': wrong clipboard'
    else:
        pump(0.2)
        assert len(peer.clips) == before, name + ': forbidden copy'
    print(json.dumps({'test': name, 'bytes': len(text), 'pass': True}), flush=True)


try:
    owner, spectator = Peer(5910), Peer(5911)
    assert pump(2, lambda: bool(owner.hellos and spectator.hellos))
    assert owner.hellos == [14] and spectator.hellos == [1]
    owner.send(0x24)
    spectator.send(0x24)
    pump(0.1)
    check('direct Unicode', owner, 'copied 日本 👩🏽‍💻'.encode())
    check('exact 64 KiB', owner, b'x' * 65536)
    check('direct over cap rejected', owner, b'x' * 65537, accepted=False)
    check('invalid UTF-8 rejected', owner, b'\xff', accepted=False)
    check('NUL rejected', owner, b'secret\0tail', accepted=False)
    check('INCR 64 KiB', owner, b'y' * 65536, 'incr')
    check('INCR announced over cap rejected', owner, b'z' * 65537, 'incr', accepted=False)
    check('INCR cumulative over cap rejected', owner, b'z' * 65537, 'incr', size=0, accepted=False)
    paused = True
    publish(b'previous owner private selection', 'incr')
    pump(0.1)
    replacement = Peer(5910)
    assert pump(2, lambda: replacement.hellos and owner.hellos[-1] == 1)
    replacement.send(0x24)
    pump(0.1)
    paused = False
    for key, chunks in list(transfers.items()):
        put(*key, utf8, 8, b'previous owner private selection')
    flush(display)
    pump(0.2)
    assert replacement.clips == [], 'pending previous-owner data leaked into replacement'
    print(json.dumps({'test': 'handoff discards pending INCR and old snapshot', 'pass': True}), flush=True)
    check('replacement receives fresh copy', replacement, b'new owner copied sentence')
    publish(b'never replied', 'hang')
    pump(2.2)
    check('timeout recovers on next copy', replacement, b'after timeout')
    assert spectator.clips == [], 'spectator received clipboard'
    replacement.sock.close()
    replacement.closed = True
    pump(0.1)
    publish(b'no active owner private clipboard')
    pump(0.2)
    assert spectator.clips == [], 'spectator watch request enabled copy-out'
    print(json.dumps({'test': 'spectator isolation with and without owner', 'pass': True}), flush=True)
finally:
    for peer in peers:
        peer.sock.close()
    destroy(display, window)
    close(display)
