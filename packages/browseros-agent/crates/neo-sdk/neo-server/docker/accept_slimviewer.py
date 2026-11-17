#!/usr/bin/env python3
"""Probe the installed viewer and its real TCP stream inside accept.sh's disposable containers."""

import hashlib
import json
import os
import pwd
import re
import signal
import socket
import struct
import sys
import time
from pathlib import Path

ASSETS = {
    'slimviewer.html': '9580b195c453123c44d83754b8d3e1625e22ac201f6a231c59130e824de682c2',
    'ui.js': 'e72e12a652957b7aa134f6ac7aaec666e64454b7cf4bc8e1c795405985b10eed',
}
LOOPBACK = [('0100007F', 5910), ('0100007F', 5911)]


def check_assets():
    viewer = json.loads(Path('/usr/share/neo-sdk/provenance.json').read_text())['slimviewer']
    binary = Path('/usr/local/bin/slimviewer')
    if viewer['sha256'] != hashlib.sha256(binary.read_bytes()).hexdigest() or viewer['encoders'] != ['openh264']:
        raise ValueError(f'{binary} or its encoders differ from provenance: {viewer}')

    version, revision = viewer['openh264']['version'], viewer['openh264']['revision']
    notices = Path('/usr/share/doc/slimviewer/THIRD-PARTY-LICENSES').read_text()
    section = notices.partition(f'OpenH264 {version} — BSD-2-Clause')[2].partition('\n====')[0]
    for required in (f'v{version} {revision}', 'Copyright (c) 2013, Cisco Systems',
                     'Redistributions of source code must retain', 'Redistributions in binary form must reproduce',
                     'THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"'):
        if required not in section:
            raise ValueError(f'missing from OpenH264 notice: {required}')

    folder = Path('/usr/share/slimviewer')
    if folder.stat().st_mode & 0o777 != 0o755:
        raise ValueError(f'{folder} mode {folder.stat().st_mode & 0o777:o}, expected 755')
    for name, expected in ASSETS.items():
        actual = hashlib.sha256((folder / name).read_bytes()).hexdigest()
        if actual != expected:
            raise ValueError(f'{name} sha256 {actual}, expected {expected}')
    return f'page hashes, viewer sha256, OpenH264 {version} source and BSD notice verified'


def viewer_pids():
    pids = []
    for comm in Path('/proc').glob('[0-9]*/comm'):
        try:
            if comm.read_bytes().rstrip() == b'slimviewer':
                pids.append(int(comm.parent.name))
        except (FileNotFoundError, ProcessLookupError):
            continue
    return pids


def listeners(pid=None):
    # The namespace check catches another process on a viewer port. Checking the viewer's
    # own socket inodes also catches it accidentally listening on a third port.
    owned = set()
    if pid is not None:
        for fd in Path(f'/proc/{pid}/fd').iterdir():
            try:
                owned.add(os.readlink(fd))
            except FileNotFoundError:
                continue
    found = []
    for table in ('tcp', 'tcp6'):
        path = Path('/proc/net') / table
        if table == 'tcp6' and not path.exists():
            continue
        for row in path.read_text().splitlines()[1:]:
            fields = row.split()
            address, port_hex = fields[1].split(':')
            port = int(port_hex, 16)
            include = port in (5910, 5911) if pid is None else f'socket:[{fields[9]}]' in owned
            if fields[3] == '0A' and include:
                found.append((address, port))
    return sorted(found)


def ready_pid(previous=None):
    deadline = time.monotonic() + 10
    while True:
        pids, bound = viewer_pids(), listeners()
        if len(pids) == 1 and pids[0] != previous and bound == LOOPBACK:
            return pids[0]
        if time.monotonic() >= deadline:
            raise ValueError(f'viewer not ready within 10s: pids={pids}, previous={previous}, listeners={bound}')
        time.sleep(0.1)


def check_process(pid):
    status = dict(line.split(':', 1) for line in Path(f'/proc/{pid}/status').read_text().splitlines())
    parent = int(status['PPid'])
    argv = Path(f'/proc/{parent}/cmdline').read_bytes().rstrip(b'\0').split(b'\0')
    entrypoint = b'/usr/local/bin/neo-entrypoint'
    if argv[0] != entrypoint and not (os.path.basename(argv[0]) == b'bash' and argv[1:2] == [entrypoint]):
        raise ValueError(f'viewer {pid} parent {parent} has argv {argv}')
    neo = pwd.getpwnam('neo')
    for field, account, configured in [('Uid', neo.pw_uid, 'PUID'), ('Gid', neo.pw_gid, 'PGID')]:
        if str(account) != os.environ[configured] or set(status[field].split()) != {str(account)}:
            raise ValueError(f'viewer {pid} {field}={status[field].strip()}, neo={account}, '
                             f'{configured}={os.environ[configured]}')
    bound = listeners(pid)
    if bound != LOOPBACK:
        raise ValueError(f'viewer {pid} owns listeners {bound}, expected {LOOPBACK}')
    return parent


def record(stream):
    prefix = stream.read(4)
    if len(prefix) != 4:
        raise ValueError('viewer closed before the record length')
    length = int.from_bytes(prefix, 'big')
    if not 0 < length <= 16 * 1024 * 1024:
        raise ValueError(f'invalid viewer record length {length}')
    data = stream.read(length)
    if len(data) != length:
        raise ValueError(f'viewer sent {len(data)} of {length} bytes')
    return data


def spectator_frame():
    with socket.create_connection(('127.0.0.1', 5911), timeout=5) as sock, sock.makefile('rb') as stream:
        hello = record(stream)
        if len(hello) != 3 or hello[:2] != b'\x01\x01' or not hello[2] & 1:
            raise ValueError(f'invalid spectator greeting {hello!r}')
        frame = record(stream)
        kind, seq, flags, width, height = struct.unpack_from('>BIBHH', frame)
        # A new spectator must be able to decode its first frame from this stream alone.
        nal_types = {b[0] & 0x1f for b in re.findall(rb'\x00\x00\x01(.)', frame[10:], re.DOTALL)}
        if kind != 2 or not flags & 1 or not width or not height or not {7, 8, 5} <= nal_types:
            raise ValueError(f'invalid H.264 keyframe: kind={kind}, flags={flags}, {width}x{height}, NALs={nal_types}')
        sock.sendall(struct.pack('>IBI', 5, 0x10, seq))
        return f'{width}x{height}'


def check_running():
    pid = ready_pid()
    parent = check_process(pid)
    frame = spectator_frame()
    start = time.monotonic()
    os.kill(pid, signal.SIGKILL)
    replacement = ready_pid(previous=pid)
    elapsed = time.monotonic() - start
    if check_process(replacement) != parent:
        raise ValueError('the entrypoint itself restarted')
    recovered_frame = spectator_frame()
    return (f'neo uid {os.environ["PUID"]}, entrypoint {parent}, IPv4 loopback only on 5910/5911, '
            f'H.264 {frame}; {pid} restarted as {replacement} in {elapsed:.1f}s, H.264 {recovered_frame}')


def check_disabled():
    # Observe a healthy container for longer than the supervisor's restart delay.
    deadline = time.monotonic() + 6
    while True:
        pids, bound = viewer_pids(), listeners()
        if pids or bound:
            raise ValueError(f'NEO_SLIMVIEWER=0 left pids={pids}, listeners={bound}')
        if time.monotonic() >= deadline:
            return 'healthy with NEO_SLIMVIEWER=0, no viewer process or listening socket'
        time.sleep(0.2)


if __name__ == '__main__':
    try:
        print({'assets': check_assets, 'running': check_running, 'disabled': check_disabled}[sys.argv[1]]())
    except (OSError, ValueError, KeyError, struct.error) as error:
        sys.exit(str(error))
