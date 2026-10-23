#!/usr/bin/env python3
"""Private-registry guard and hosted CAPTCHA smoke test. Standard library only."""

import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "neo"))
from smoke import McpClient  # noqa: E402


class ReleaseError(RuntimeError):
    """A safe diagnostic written by this script, never by a remote server."""


def require_private(registry="https://ghcr.io"):
    """Use an anonymous token, never Docker's login or the Actions token."""
    token_url = f"{registry}/token?scope=repository:browseros-ai/neo-captcha:pull&service=ghcr.io"
    try:
        with urllib.request.urlopen(token_url, timeout=30) as response:
            body = json.load(response)
        token = body.get("token") or body.get("access_token")
        if not isinstance(token, str) or not token:
            raise ReleaseError("Anonymous token missing; refusing keyed publication")
        request = urllib.request.Request(
            f"{registry}/v2/browseros-ai/neo-captcha/tags/list",
            headers={"Authorization": f"Bearer {token}"},
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            if response.status == 200:
                raise ReleaseError("neo-captcha is public: make the package private in GitHub Settings before retrying")
            raise ReleaseError("Registry privacy unconfirmed; refusing keyed publication")
    except urllib.error.HTTPError as error:
        if error.code not in (401, 403):
            raise ReleaseError("Registry privacy unconfirmed; refusing keyed publication") from None
    except (urllib.error.URLError, ValueError):
        raise ReleaseError("Registry privacy check failed; refusing keyed publication") from None


def verify(origin, hosted):
    for endpoint in ("ready", "health"):
        with urllib.request.urlopen(f"{origin}/system/{endpoint}", timeout=15) as response:
            body = response.read()
        if endpoint == "health":
            print(json.dumps(json.loads(body), sort_keys=True))
    client = McpClient(f"{origin}/mcp")
    client.call("initialize", {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "neo-captcha-release", "version": "1"},
    })
    client.notify("notifications/initialized")
    names = set()
    params = {}
    while True:
        page = client.call("tools/list", params)
        names.update(tool["name"] for tool in page["tools"])
        if not page.get("nextCursor"):
            break
        params = {"cursor": page["nextCursor"]}
    print(json.dumps(sorted(names)))
    if "tabs" not in names or ("solve_captcha" in names) != hosted:
        raise ReleaseError(f"Hosted mode {int(hosted)} has incorrect CAPTCHA availability (requires the hosted-gate PR)")


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["private"]:
            require_private()
        elif len(sys.argv) == 4 and sys.argv[1] == "verify" and sys.argv[3] in ("0", "1"):
            verify(sys.argv[2], sys.argv[3] == "1")
        else:
            raise ReleaseError("Usage: release.py private | verify ORIGIN 0|1")
    except Exception as error:
        # Never serialize HTTP bodies or container logs into public CI output.
        print(str(error) if isinstance(error, ReleaseError) else "Release verification failed", file=sys.stderr)
        sys.exit(1)
