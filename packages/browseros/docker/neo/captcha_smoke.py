#!/usr/bin/env python3
"""Verify CAPTCHA tool availability in the released Neo image, without paid calls.

Both native release jobs exercise this interface with hosted mode on and off.
A keyless image must hide the tool in both modes. Remote errors are redacted so
provider details never reach public workflow output. Standard library only.
"""

import json
import sys
import urllib.request

from smoke import PROTOCOL, McpClient


class VerificationError(RuntimeError):
    """A diagnostic owned by this verifier, safe to write to workflow output."""


def verify(origin: str, hosted: bool, keyed: bool = True) -> None:
    """Require readiness and tool visibility to match the image's configuration."""
    for endpoint in ("ready", "health"):
        with urllib.request.urlopen(f"{origin}/system/{endpoint}", timeout=15) as response:
            body = response.read()
        if endpoint == "health":
            print(json.dumps(json.loads(body), sort_keys=True))
    client = McpClient(f"{origin}/mcp")
    client.call("initialize", {
        "protocolVersion": PROTOCOL, "capabilities": {},
        "clientInfo": {"name": "neo-docker-smoke", "version": "1"},
    })
    client.notify("notifications/initialized")
    names = {tool["name"] for tool in client.tools()}
    print(json.dumps(sorted(names)))
    if "tabs" not in names or ("solve_captcha" in names) != (hosted and keyed):
        raise VerificationError(f"Incorrect CAPTCHA availability (hosted={int(hosted)}, keyed={int(keyed)})")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 4 or any(value not in ("0", "1") for value in sys.argv[2:]):
            raise VerificationError("Usage: captcha_smoke.py ORIGIN HOSTED_0_OR_1 KEYED_0_OR_1")
        verify(sys.argv[1], sys.argv[2] == "1", sys.argv[3] == "1")
    except Exception as error:
        # Never serialize HTTP error bodies or container logs into public CI output.
        print(str(error) if isinstance(error, VerificationError) else "Docker CAPTCHA verification failed", file=sys.stderr)
        sys.exit(1)
