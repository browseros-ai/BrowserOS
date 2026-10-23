#!/usr/bin/env python3
"""Check bundled Neo extension state inside a running Docker container.

The release workflow streams this script to the container's Python so it can
inspect the loopback CDP endpoint and profile without publishing CDP or copying
profile data to the host.
"""

import argparse
import json
from pathlib import Path
import time
import urllib.request

MANAGED_IDS = {
    "bflpfmnmnokmjhmgnolecpppdbdophmk",
    "adlpneommgkgeanpaekgoaolcpncohkf",
    "pjimfkbpehlcllblajnpfamdfjhhlgkc",
}
NEO_IDS = {"adlpneommgkgeanpaekgoaolcpncohkf", "pjimfkbpehlcllblajnpfamdfjhhlgkc"}


def extension_state() -> dict:
    prefs = json.loads(Path("/data/profile/Default/Preferences").read_text())
    records = prefs.get("extensions", {}).get("settings", {})
    # Chromium retains per-ID bookkeeping after uninstalling a managed extension.
    # Manifest/path metadata distinguishes installed extensions from those remnants.
    installed = {
        extension_id: record
        for extension_id, record in records.items()
        if extension_id in MANAGED_IDS and ("manifest" in record or "path" in record)
    }
    enabled = {
        extension_id
        for extension_id, record in installed.items()
        if not record.get("disable_reasons")
    }
    with urllib.request.urlopen(
        "http://127.0.0.1:9110/json/list", timeout=3
    ) as response:
        targets = json.load(response)
    running = {
        extension_id
        for extension_id in MANAGED_IDS
        # Restored tabs can retain a removed extension's URL on an error page.
        # Background targets, together with install metadata, identify execution.
        if any(
            target.get("type") in {"service_worker", "background_page"}
            and target.get("url", "").startswith(f"chrome-extension://{extension_id}/")
            for target in targets
        )
    }
    return {
        "installed": sorted(installed),
        "enabled": sorted(enabled),
        "running": sorted(running),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("disabled", choices=("false", "true"))
    args = parser.parse_args()
    disabled = args.disabled == "true"

    # Bundle installation, orphan removal, and preference writes finish after
    # server readiness. Bound the wait instead of assuming a fixed startup delay.
    deadline = time.monotonic() + 30
    observed = {}
    while time.monotonic() < deadline:
        try:
            observed = extension_state()
            if disabled:
                matches = not observed["installed"] and not observed["running"]
            else:
                matches = (
                    set(observed["installed"]) == NEO_IDS
                    and set(observed["enabled"]) == NEO_IDS
                )
            if matches:
                print(
                    f"ok   bundled BrowserOS extensions disabled={args.disabled}: {json.dumps(observed)}"
                )
                return
        except (OSError, ValueError) as error:
            observed = {"error": str(error)}
        time.sleep(0.5)
    raise SystemExit(
        f"FAIL bundled BrowserOS extensions disabled={args.disabled}: {json.dumps(observed)}"
    )


if __name__ == "__main__":
    main()
