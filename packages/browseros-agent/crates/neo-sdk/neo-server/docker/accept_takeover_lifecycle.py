#!/usr/bin/env python3
"""Measure takeover recovery without restarting the browser or repairing the observed viewport.

Run inside a private image acceptance container as root in the host socket's group. The Python
fixture and its browser CDP connection outlive a killed SDK server. The same restart command at
scale 1 runs against a base image without depending on new fit response fields. Standard library
only. Every observation is saved before assertions, including the original viewport outcome.
"""

import argparse
from dataclasses import asdict, dataclass
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time
import urllib.request

from accept_client import call_run, connect, host
from accept_takeover_scale import ORIGINAL, PHONE, Rfb, Site, Window, fresh_events, until


@dataclass(frozen=True)
class ProcessIdentity:
    """Identify a live process across PID reuse before an acceptance probe may signal it."""

    pid: int
    start_ticks: int
    argv0: str

    @classmethod
    def read(cls, pid):
        directory = Path("/proc") / str(pid)
        fields = (directory / "stat").read_text().rsplit(") ", 1)[1].split()
        assert fields[0] != "Z", f"process {pid} is a zombie"
        argv0 = (directory / "cmdline").read_bytes().split(b"\0", 1)[0].decode()
        assert argv0, f"process {pid} has no argv[0]"
        return cls(pid, int(fields[19]), argv0)

    def revalidate(self):
        assert self == self.read(self.pid), f"process identity changed before use: {self}"


def server_identity(server_path):
    """The image supplies its exact executable path; never select a process by a name pattern."""
    found = []
    for directory in Path("/proc").iterdir():
        if not directory.name.isdecimal():
            continue
        try:
            if (directory / "cmdline").read_bytes().split(b"\0", 1)[0].decode() == server_path:
                found.append(ProcessIdentity.read(int(directory.name)))
        except (FileNotFoundError, ProcessLookupError):
            continue
    assert len(found) == 1, f"expected one exact server executable, found {found}"
    return found[0]


def page_size(value, width, height, dpr):
    logical = value.get("metrics", {}).get("logical", {}) if value else {}
    visual = logical.get("visualViewport", {})
    return (value and value.get("frameReady") and logical.get("screen") == [width, height]
            and logical.get("inner") == [width, height] and logical.get("dpr") == dpr
            and visual.get("width") == width and visual.get("height") == height and visual.get("scale") == 1)


class Probe:
    """Own one fixture tab and collect host, browser, process and RFB facts for a lifecycle row.

    Only setup sets an agent viewport. Recovery must be observed before cleanup, so reconnecting
    MCP after a server restart cannot silently repair the state being compared with a base image.
    """

    def __init__(self, args):
        self.args = args
        self.artifacts = Path(args.artifacts)
        self.artifacts.mkdir(parents=True, exist_ok=True)
        self.client = self.window = self.control = self.takeover = self.tab = None
        self.target = self.server_path = None
        self.task = f"accept-takeover-lifecycle-{os.getpid()}"
        self.evidence = {"ok": False, "case": args.case, "scale": args.scale, "problems": [],
                         "releaseCalls": [], "artifacts": {"evidence": "evidence.json"},
                         "originalViewportPolicy": args.original_viewport,
                         "fixtureProcess": asdict(ProcessIdentity.read(os.getpid()))}
        self.fixture = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Site)
        self.thread = threading.Thread(target=self.fixture.serve_forever, kwargs={"poll_interval": 0.1}, daemon=True)
        self.thread.start()

    def save(self):
        (self.artifacts / "evidence.json").write_text(json.dumps(self.evidence, indent=2) + "\n")

    def reply(self, method, path, body=None):
        return host(self.args.socket, self.args.token_file, path, method, "" if body is None else json.dumps(body))

    def request(self, method, path, body=None):
        reply = self.reply(method, path, body)
        assert reply.get("status") == 200, f"{method} {path}: {reply}"
        return reply["body"]

    def run(self, code):
        reply = call_run(self.client, code, {"taskId": self.task, "hostData": {"check": "takeover-lifecycle"}})
        assert not reply["isError"] and reply["meta"].get("status") == "ok", reply
        return (reply.get("value") or {}).get("value")

    def on_page(self, code):
        return self.run(f"const p = context.pages().find(p => p.tabId === {json.dumps(self.tab)});\n" + code)

    def facts(self):
        # A timer sample can predate the native-window read during restart. Refresh only the
        # fixture's observations synchronously; never reapply a viewport to make them match.
        return self.on_page("return await p.evaluate(() => {window.dispatchEvent(new Event('neo-proof-refresh')); "
                            "const text = document.getElementById('proof').textContent; return text ? JSON.parse(text) : null;});")

    def at_size(self, width, height, dpr):
        return until(self.facts, lambda value: page_size(value, width, height, dpr),
                     f"page did not reach {width}x{height} DPR {dpr}")

    def processes(self):
        browser = [item for item in self.window.command("SystemInfo.getProcessInfo")["processInfo"] if item["type"] == "browser"]
        assert len(browser) == 1, f"CDP did not identify exactly one browser process: {browser}"
        with urllib.request.urlopen("http://127.0.0.1:9110/json/version", timeout=5) as response:
            debugger_url = json.load(response)["webSocketDebuggerUrl"]
        return {"server": asdict(server_identity(self.server_path)),
                "browser": asdict(ProcessIdentity.read(browser[0]["id"])),
                "browserDebuggerUrl": debugger_url}

    def display(self):
        display = Rfb(self.args.socket, self.args.token_file, "/v1/display?mode=view")
        try:
            return display.initial
        finally:
            display.close()

    def observe(self, label):
        value = {"metrics": self.facts()["metrics"], "window": self.window.bounds(self.target),
                 "tabId": self.tab, "targetId": self.target, "processes": self.processes(),
                 "display": self.display(), "monotonic": time.monotonic(), "wallTime": time.time()}
        if self.takeover:
            value["takeover"] = self.reply("GET", f"/v1/takeovers/{self.takeover['id']}")
        self.evidence[label] = value
        filename = f"{label}.json"
        (self.artifacts / filename).write_text(json.dumps(value, indent=2) + "\n")
        self.evidence["artifacts"][label] = filename
        self.save()
        return value

    def setup(self):
        self.server_path = subprocess.check_output(["neo-server-info", "path"], text=True, timeout=5).strip()
        self.evidence["serverPath"] = self.server_path
        self.client = connect(self.args.url)
        self.tab = self.run("const p = await context.newPage(); return p.tabId;")
        origin = f"http://127.0.0.1:{self.fixture.server_port}/"
        self.on_page(f"await p.setViewportSize({json.dumps(ORIGINAL)}); await p.goto({json.dumps(origin)}); return true;")
        self.target = next(item["targetId"] for item in self.request("GET", "/v1/tabs")["items"] if item["tabId"] == self.tab)
        self.window = Window()
        self.at_size(ORIGINAL["width"], ORIGINAL["height"], 1)
        self.observe("original")

    def start(self, ttl):
        # Omission is the baseline contract. No generated types or scale-2 fields are used here.
        body = self.takeover_request(ttl)
        self.evidence["request"] = body
        self.evidence["startMonotonic"] = time.monotonic()
        self.takeover = self.request("POST", "/v1/takeovers", body)
        self.evidence["takeover"] = self.takeover
        self.save()
        assert self.takeover["state"] == "active" and self.takeover.get("fitId"), self.takeover
        self.control = Rfb(self.args.socket, self.args.token_file, self.takeover["display"])
        self.at_size(PHONE["width"], PHONE["height"], 2)
        active = self.observe("active")
        expected = [PHONE[side] * self.args.scale for side in ("width", "height")]
        assert [active["display"][side] for side in ("width", "height")] == expected, active
        assert [self.control.initial[side] for side in ("width", "height")] == expected, self.control.initial
        assert active["window"]["bounds"]["windowState"] == "fullscreen", active
        assert [active["window"]["bounds"][side] for side in ("width", "height")] == expected, active
        blocked = self.on_page("try {await p.mouse.click(24,24);return {sent:true}} catch(error) {return {code:error.code,delivery:error.delivery}}")
        self.evidence["activeAgentInput"] = blocked
        self.save()
        assert blocked == {"code": "owner_has_control", "delivery": "not_sent"}, blocked

    def takeover_request(self, ttl):
        body = {"tabId": self.tab, "reads": "allow", "viewer": {**PHONE}, "ttlSeconds": ttl}
        if self.args.scale != 1:
            body["viewer"]["scale"] = self.args.scale
        return body

    def recovered(self, strict_viewport):
        original = self.evidence["original"]
        # Read the changed viewport before checking restoration. No setViewportSize is allowed
        # after setup. A base image can therefore expose lost emulation instead of being fixed.
        self.observe("afterInitial")
        until(lambda: self.window.bounds(self.target), lambda value: value == original["window"],
              "native window was not restored")
        expected_display = [original["display"][side] for side in ("width", "height")]
        until(self.display, lambda value: [value[side] for side in ("width", "height")] == expected_display,
              "home display was not restored")
        if strict_viewport:
            expected = original["metrics"]
            until(self.facts, lambda value: value and all(value["metrics"][key] == expected[key]
                  for key in ("logical", "available", "position")), "original logical viewport was not restored")
        after = self.observe("after")
        assert [after["display"][side] for side in ("width", "height")] == [original["display"][side] for side in ("width", "height")], after
        assert after["window"] == original["window"], after
        assert after["processes"]["browser"] == original["processes"]["browser"], after
        assert after["processes"]["browserDebuggerUrl"] == original["processes"]["browserDebuggerUrl"], after
        tabs = self.request("GET", "/v1/tabs")["items"]
        target = next((item["targetId"] for item in tabs if item["tabId"] == self.tab), None)
        assert target == self.target, f"original tab/target changed: {target} != {self.target}"
        self.evidence["originalViewportRestored"] = after["metrics"]["logical"] == original["metrics"]["logical"]
        self.evidence["existingBrowserConnectionWorks"] = True
        self.evidence["sameTarget"] = True
        before = self.facts()["eventSeq"]
        self.on_page("await p.mouse.click(24,24); return true;")
        got = until(self.facts, lambda value: any(event["kind"] == "click" and event["target"] == "tl"
                    and event["trusted"] for event in fresh_events(value, before)), "agent input was not unlocked")
        self.evidence["restoredAgentInput"] = fresh_events(got, before)
        self.save()

    def release(self, reason):
        self.evidence["releaseCalls"].append({"reason": reason, "monotonic": time.monotonic()})
        self.save()
        return self.request("POST", f"/v1/takeovers/{self.takeover['id']}/release")

    def close(self):
        for label, cleanup in (
            ("release", lambda: self.release("cleanup after observation") if self.takeover and
             self.reply("GET", f"/v1/takeovers/{self.takeover['id']}").get("body", {}).get("state") == "active" else None),
            ("control", lambda: self.control.close() if self.control else None),
            ("window", lambda: self.window.close() if self.window else None),
            ("owned tab", lambda: self.on_page("await p.close(); return true;") if self.tab is not None else None),
        ):
            try:
                cleanup()
            except Exception as error:
                self.evidence["ok"] = False
                self.evidence["problems"].append(f"cleanup {label}: {error}")
        self.fixture.shutdown()
        self.fixture.server_close()
        self.thread.join(timeout=2)


def restart_server(probe):
    """Kill exactly the recorded SDK process while preserving the fixture and browser observer."""
    assert os.geteuid() == 0, "server-only restart requires root inside the private acceptance container"
    probe.start(300)
    active = probe.evidence["active"]["processes"]
    recorded = ProcessIdentity(**active["server"])
    assert recorded.argv0 == probe.server_path, recorded
    assert recorded.pid != active["browser"]["pid"] and recorded.pid != os.getpid(), recorded
    recorded.revalidate()
    probe.evidence["signal"] = {"process": asdict(recorded), "signal": "SIGKILL", "monotonic": time.monotonic()}
    probe.save()
    # The browser owns relaunching its bundled server. Never signal the browser, its supervisor,
    # or a process found by substring. Revalidate the PID/start pair immediately before the kill.
    recorded.revalidate()
    os.kill(recorded.pid, signal.SIGKILL)
    deadline, last = time.monotonic() + 45, None
    while time.monotonic() < deadline:
        try:
            replacement = server_identity(probe.server_path)
            if replacement != recorded:
                ready = probe.reply("GET", "/v1/tabs")
                with urllib.request.urlopen(probe.args.url.rsplit("/", 1)[0] + "/system/ready", timeout=2) as response:
                    assert response.status == 200, response.status
                if ready.get("status") == 200:
                    break
        except (OSError, AssertionError) as error:
            last = str(error)
        time.sleep(0.2)
    else:
        raise AssertionError(f"server did not restart within 45 seconds: {last}")
    probe.evidence["replacementServer"] = asdict(replacement)
    probe.save()
    assert replacement.pid != recorded.pid, f"expected a new server PID: {replacement}"
    probe.client = connect(probe.args.url)
    probe.recovered(probe.args.original_viewport == "restored")
    after = probe.evidence["after"]
    assert after["processes"]["server"] == asdict(replacement), after
    takeover = after["takeover"]
    assert takeover.get("status") == 404 or (takeover.get("status") == 200 and takeover["body"].get("state") == "released"), takeover
    probe.evidence["detail"] = (f"server {recorded.pid} restarted as {replacement.pid}; same browser process and CDP connection; "
                                f"display/window restored, original viewport restored={probe.evidence['originalViewportRestored']}; agent input unlocked")


def ttl_expiry(probe):
    """Wait for the takeover's own timer, without a host release or a shorter test-only TTL."""
    probe.start(60)
    path = f"/v1/takeovers/{probe.takeover['id']}"
    released = until(lambda: probe.request("GET", path), lambda item: item.get("state") == "released",
                     "takeover did not release itself", seconds=70)
    elapsed = time.monotonic() - probe.evidence["startMonotonic"]
    probe.evidence["expiredTakeover"] = released
    probe.evidence["selfReleaseElapsedSeconds"] = elapsed
    probe.save()
    assert not probe.evidence["releaseCalls"], "TTL proof must not trigger an explicit release"
    assert released.get("releaseReason") == "expired", released
    assert 45 <= elapsed <= 70, f"60-second TTL should self-release about 50 seconds after start: {elapsed}"
    probe.evidence["displayClose"] = probe.control.released()
    probe.recovered(True)
    assert probe.evidence["after"]["processes"]["server"] == probe.evidence["original"]["processes"]["server"], "TTL must not restart the server"
    probe.evidence["detail"] = (f"60-second takeover expired itself after {elapsed:.2f}s; "
                                "original display/window and 517x433 DPR 1 viewport restored; agent input unlocked")


def resize_refusal(probe):
    """Require a real Xvnc resize refusal to leave the original geometry and input usable.

    accept.sh starts only this disposable container with resizing disabled. This parameter is
    startup-only in the packaged Xvnc, so changing it at runtime would not arm the failure.
    """
    result = subprocess.run(["vncconfig", "-display", ":1", "-get", "AcceptSetDesktopSize"],
                            capture_output=True, text=True, timeout=5, check=True)
    disabled = result.stdout.strip()
    probe.evidence["xvncParameter"] = {"name": "AcceptSetDesktopSize", "atStartup": disabled}
    probe.save()
    assert disabled.lower() in ("0", "false", "no"), f"Xvnc must refuse resizes from startup: {disabled}"
    body = probe.takeover_request(300)
    reply = probe.reply("POST", "/v1/takeovers", body)
    probe.evidence["failedRequest"] = {"request": body, "response": reply}
    probe.observe("refused")
    assert reply.get("status") == 503 and reply.get("body", {}).get("code") == "display_unavailable", reply
    probe.recovered(True)
    refused = probe.evidence["after"]
    original = probe.evidence["original"]
    # Keep every metric for the base comparison. Chromium can change DOM outer dimensions
    # while logical emulation and the actual native window remain unchanged.
    probe.evidence["pageMetricChanges"] = {
        key: {"original": original["metrics"][key], "after": value}
        for key, value in refused["metrics"].items() if value != original["metrics"][key]
    }
    assert refused["processes"] == original["processes"], "resize rejection restarted a process"
    probe.evidence["detail"] = ("Xvnc AcceptSetDesktopSize=0 caused HTTP 503 display_unavailable; "
                                "original display/window/logical viewport unchanged and input unlocked; page metric changes recorded")


def check(args):
    probe = Probe(args)
    try:
        probe.setup()
        {"restart": restart_server, "ttl": ttl_expiry, "refusal": resize_refusal}[args.case](probe)
        probe.evidence["ok"] = True
    except Exception as error:
        probe.evidence["problems"].append(f"{type(error).__name__}: {error}")
    finally:
        probe.close()
    if not probe.evidence["ok"]:
        probe.evidence["detail"] = "; ".join(probe.evidence["problems"])
    probe.save()
    return probe.evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=("restart", "ttl", "refusal"), required=True)
    parser.add_argument("--scale", type=int, choices=(1, 2), default=2)
    parser.add_argument("--original-viewport", choices=("record", "restored"), default="record",
                        help="Restart records the original viewport outcome unless strict restoration is explicitly required. TTL always requires restoration.")
    parser.add_argument("url", help="MCP URL inside the private acceptance container")
    parser.add_argument("socket", help="host Unix socket")
    parser.add_argument("token_file")
    parser.add_argument("artifacts", help="directory for this row's JSON observations")
    args = parser.parse_args()

    def timed_out(_signal, _frame):
        signal.signal(signal.SIGALRM, signal.SIG_DFL)
        signal.alarm(30)
        raise TimeoutError("takeover lifecycle acceptance exceeded 150 seconds")

    signal.signal(signal.SIGALRM, timed_out)
    signal.alarm(150)
    result = check(args)
    signal.alarm(0)
    print(json.dumps(result))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
