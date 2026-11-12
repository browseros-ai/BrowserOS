#!/usr/bin/env python3
"""Check inside a running Neo container that its display matches the server it runs.

claw-sdk images disable X clipboard export to VNC clients and
run no unauthenticated noVNC viewer on 6080 (sdk-traces R1). claw-server images
keep both, as before. The release workflow streams this script to the container
with the server it expects, as it does check_zoom.py:

  docker exec -i <container> python3 - <claw-server|claw-sdk> < check_display.py

Standard library only. The image has no ps, so processes are read from /proc.
"""

import argparse
import socket
import sys
from pathlib import Path

# Xvnc switches that stop it sending the clipboard and the primary selection to clients.
CLIPBOARD_OFF = ["-SendCutText=0", "-SendPrimary=0"]

EXPECTED = {
    "claw-server": {
        "Xvnc clipboard-off switches": [],
        "noVNC websockify running": True,
        "127.0.0.1:6080": "accepts",
    },
    "claw-sdk": {
        "Xvnc clipboard-off switches": CLIPBOARD_OFF,
        "noVNC websockify running": False,
        "127.0.0.1:6080": "refuses",
    },
}


def processes() -> list[list[str]]:
    """The argv of every process in the container."""
    argvs = []
    for cmdline in Path("/proc").glob("[0-9]*/cmdline"):
        try:
            raw = cmdline.read_bytes()
        except OSError:  # the process exited
            continue
        if raw:  # kernel threads have no argv
            argvs.append(raw.rstrip(b"\0").decode(errors="replace").split("\0"))
    return argvs


def viewer_port() -> str:
    """How 127.0.0.1:6080 answers a connection: accepts, refuses, or the error."""
    try:
        socket.create_connection(("127.0.0.1", 6080), timeout=3).close()
    except ConnectionRefusedError:
        return "refuses"
    except OSError as error:
        return repr(error)
    return "accepts"


def observed() -> dict:
    argvs = processes()
    xvnc = [argv for argv in argvs if Path(argv[0]).name == "Xvnc"]
    return {
        # A container runs one display server; any other count fails the comparison.
        "Xvnc clipboard-off switches": (
            [flag for flag in CLIPBOARD_OFF if flag in xvnc[0]]
            if len(xvnc) == 1
            else f"{len(xvnc)} Xvnc processes"
        ),
        # websockify is a Python script, so the interpreter comes before it in argv.
        "noVNC websockify running": any(
            Path(arg).name == "websockify" for argv in argvs for arg in argv[:2]
        ),
        "127.0.0.1:6080": viewer_port(),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", choices=sorted(EXPECTED))
    expected = EXPECTED[parser.parse_args().server]
    actual = observed()
    passed = True
    for label, want in expected.items():
        ok = actual[label] == want
        print(f"{'ok  ' if ok else 'FAIL'} {label} (expected {want}, got {actual[label]})")
        passed &= ok
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
