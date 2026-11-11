"""Exercise the installed CLI against fake Cargo, using only temporary state."""

import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

SOURCE = Path(__file__).with_name("cargo-pool.py")
FAKE = """#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys, time
args = sys.argv[1:]
root = pathlib.Path(args[args.index("--manifest-path") + 1]).parent if "--manifest-path" in args else pathlib.Path.cwd()
if "metadata" in args:
    print(json.dumps({"packages": [{"name": "local", "id": "local", "source": None, "targets": []}, {"name": "external-path", "id": "external-path", "source": None, "targets": []}]}))
    sys.exit(0)
slot = os.environ.get("CARGO_BUILD_BUILD_DIR")
if "clean" in args:
    with open(os.environ["FAKE_LOG"], "a") as f: f.write(json.dumps({"event": "clean", "args": args, "slot": slot}) + "\\n")
    if slot: pathlib.Path(slot, "local-output").unlink(missing_ok=True)
    sys.exit(0)
if slot:
    pathlib.Path(slot).mkdir(parents=True, exist_ok=True)
    pathlib.Path(slot, "local-output").write_text(str(root))
    pathlib.Path(slot, "dependency").write_text("cached")
with open(os.environ["FAKE_LOG"], "a") as f: f.write(json.dumps({"event": "start", "pid": os.getpid(), "slot": slot, "root": str(root), "args": args}) + "\\n")
if os.environ.get("FAKE_GRANDCHILD"):
    p = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    pathlib.Path(os.environ["FAKE_GRANDCHILD"]).write_text(str(p.pid))
if os.environ.get("FAKE_WAIT"):
    while not pathlib.Path(os.environ["FAKE_WAIT"]).exists(): time.sleep(.01)
if os.environ.get("FAKE_BYTES") and slot:
    pathlib.Path(slot, "large").write_bytes(b"x" * int(os.environ["FAKE_BYTES"]))
with open(os.environ["FAKE_LOG"], "a") as f: f.write(json.dumps({"event": "end", "pid": os.getpid(), "slot": slot}) + "\\n")
sys.exit(int(os.environ.get("FAKE_EXIT", "0")))
"""


class PoolTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cargo-pool-test-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.bin = self.base / "bin"
        real = self.base / "real"
        self.bin.mkdir()
        real.mkdir()
        (real / "cargo").write_text(FAKE)
        (real / "cargo").chmod(0o755)
        self.env = {
            k: v
            for k, v in os.environ.items()
            if not k.startswith(("CARGO_", "BROWSEROS_CARGO_POOL_"))
        }
        self.env.update(
            PATH=f"{self.bin}:{real}:{os.environ['PATH']}",
            BROWSEROS_CARGO_POOL_ROOT=str(self.base / "pool"),
            BROWSEROS_CARGO_POOL_SLOTS="1",
            FAKE_LOG=str(self.base / "log"),
        )
        self.a = self.checkout("a")
        self.b = self.checkout("b")
        self.manage("install")
        self.children = []
        self.addCleanup(self.stop_children)

    def stop_children(self):
        for child in self.children:
            if child.poll() is None:
                child.terminate()
            try:
                child.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.communicate()

    def checkout(self, name):
        root = self.base / name
        for sub in ("", "crates/browseros-core", "apps/claw-server"):
            (root / sub).mkdir(parents=True, exist_ok=True)
            (root / sub / "Cargo.toml").write_text(
                '[package]\nname="fixture"\n' if sub else "[workspace]\n"
            )
        return root

    def manage(self, command, env=None, check=True):
        return subprocess.run(
            [sys.executable, str(SOURCE), command, "--bin-dir", str(self.bin)],
            env=env or self.env,
            capture_output=True,
            text=True,
            check=check,
        )

    def start(self, root=None, args=None, **env):
        child = subprocess.Popen(
            [str(self.bin / "cargo"), *(args or ["build"])],
            cwd=root or self.a,
            env=dict(self.env, **env),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.children.append(child)
        return child

    def finish(self, child, code=0):
        out, err = child.communicate(timeout=15)
        self.assertEqual(child.returncode, code, err)
        return out, err

    def events(self, kind="start"):
        path = self.base / "log"
        events = (
            [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
        )
        return [event for event in events if event["event"] == kind]

    def await_starts(self, count):
        deadline = time.monotonic() + 10
        while len(self.events()) < count:
            if time.monotonic() > deadline:
                self.fail(f"only {len(self.events())} starts, expected {count}")
            time.sleep(0.01)
        return self.events()

    def test_sticky_exclusive_and_handoff(self):
        gate = self.base / "gate"
        first = self.start(FAKE_WAIT=str(gate))
        slot = self.await_starts(1)[0]["slot"]
        second = self.start()
        third = self.start(self.b)
        time.sleep(0.15)
        self.assertEqual(len(self.events()), 1)
        gate.touch()
        self.finish(first)
        self.finish(second)
        self.finish(third)
        starts = self.events()
        self.assertEqual({e["slot"] for e in starts}, {slot})
        self.assertTrue(self.events("clean"))
        self.assertIn("external-path", self.events("clean")[0]["args"])
        self.assertTrue(Path(slot, "dependency").exists())

    def test_same_checkout_waits_even_with_another_free_slot(self):
        self.env["BROWSEROS_CARGO_POOL_SLOTS"] = "2"
        gate = self.base / "gate"
        first = self.start(FAKE_WAIT=str(gate))
        slot = self.await_starts(1)[0]["slot"]
        second = self.start()
        time.sleep(0.2)
        self.assertEqual(len(self.events()), 1)
        gate.touch()
        self.finish(first)
        self.assertIn("waiting", self.finish(second)[1])
        self.assertEqual(self.events()[1]["slot"], slot)

    def test_two_checkouts_compile_concurrently(self):
        self.env["BROWSEROS_CARGO_POOL_SLOTS"] = "2"
        gate = self.base / "gate"
        first = self.start(FAKE_WAIT=str(gate))
        self.await_starts(1)
        second = self.start(self.b, FAKE_WAIT=str(gate))
        starts = self.await_starts(2)
        self.assertNotEqual(starts[0]["slot"], starts[1]["slot"])
        gate.touch()
        self.finish(first)
        self.finish(second)

    def test_busy_pool_waits_and_prints_once(self):
        gate = self.base / "gate"
        first = self.start(FAKE_WAIT=str(gate))
        self.await_starts(1)
        second = self.start(self.b)
        time.sleep(0.2)
        gate.touch()
        self.finish(first)
        self.assertEqual(self.finish(second)[1].count("waiting for a build slot"), 1)

    def test_killing_shim_keeps_lease_until_cargo_exits(self):
        gate = self.base / "gate"
        first = self.start(FAKE_WAIT=str(gate))
        self.await_starts(1)
        first.kill()
        second = self.start(self.b)
        time.sleep(0.2)
        self.assertEqual(len(self.events()), 1)
        gate.touch()
        self.finish(first, -signal.SIGKILL)
        self.finish(second)

    def test_killing_cargo_releases_lease_with_live_grandchild(self):
        gate = self.base / "gate"
        pidfile = self.base / "grandchild"
        first = self.start(FAKE_WAIT=str(gate), FAKE_GRANDCHILD=str(pidfile))
        cargo_pid = self.await_starts(1)[0]["pid"]
        deadline = time.monotonic() + 5
        while not pidfile.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        grandchild = int(pidfile.read_text())
        try:
            os.kill(cargo_pid, signal.SIGKILL)
            self.finish(first, 137)
            self.finish(self.start(self.b))
        finally:
            try:
                os.kill(grandchild, signal.SIGTERM)
            except ProcessLookupError:
                pass

    def test_successful_cargo_does_not_leave_lease_in_live_grandchild(self):
        pidfile = self.base / "grandchild"
        first = self.start(FAKE_GRANDCHILD=str(pidfile))
        self.finish(first)
        grandchild = int(pidfile.read_text())
        try:
            os.kill(grandchild, 0)
            self.finish(self.start(self.b))
            os.kill(grandchild, 0)
        finally:
            os.kill(grandchild, signal.SIGTERM)

    def test_failure_and_signals_release(self):
        self.finish(self.start(FAKE_EXIT="17"), 17)
        gate = self.base / "gate"
        child = self.start(FAKE_WAIT=str(gate))
        self.await_starts(2)
        child.terminate()
        self.finish(child, 143)
        self.finish(self.start(self.b))

    def test_ceiling_applies_even_without_handoff(self):
        self.env["BROWSEROS_CARGO_POOL_MAX_BYTES"] = "8192"
        self.finish(self.start(FAKE_BYTES="65536"))
        slot = Path(self.events()[0]["slot"])
        self.assertFalse(slot.exists())
        self.finish(self.start())

    def test_passthrough(self):
        cases = [
            (["run"], {}),
            (["bench"], {}),
            (["r"], {}),
            (["clean"], {}),
            (["metadata"], {}),
            (["fmt"], {}),
            (["--version"], {}),
            (["build", "--help"], {}),
            (["build", "--target-dir", "explicit"], {}),
            (["build", "--target-dir=explicit"], {}),
            (["build", "--config", "build.target-dir='explicit'"], {}),
            (["build"], {"CARGO_TARGET_DIR": "explicit"}),
            (["test"], {"CARGO_BUILD_BUILD_DIR": ""}),
        ]
        for args, env in cases:
            with self.subTest(args=args, env=env):
                self.finish(self.start(args=args, **env))
        self.finish(self.start(self.base, ["build"]))
        self.assertFalse((self.base / "pool").exists())

    def test_nested_manifest_toolchain_alias_and_test_args(self):
        nested = self.a / "crates/browseros-core"
        self.finish(self.start(nested, ["+stable", "t", "--", "--target-dir", "test-argument"]))
        self.finish(self.start(self.base, ["build", "--manifest-path", str(self.b / "Cargo.toml")]))
        self.assertEqual(len(self.events()), 2)
        self.assertTrue(all(event["slot"] for event in self.events()))

    def test_nondefault_mode_keeps_sticky_reuse_and_wipes_on_handoff(self):
        self.finish(self.start(args=["build", "--release"]))
        slot = Path(self.events()[-1]["slot"])
        sentinel = slot / "retained"
        sentinel.touch()
        self.finish(self.start(args=["build", "--release"]))
        self.assertTrue(sentinel.exists())
        self.finish(self.start(self.b, ["build", "--release"]))
        self.assertFalse(sentinel.exists())

    def test_bench_no_run_and_global_flags_are_pooled(self):
        self.finish(self.start(args=["--color", "always", "bench", "--no-run"]))
        self.assertIsNotNone(self.events()[-1]["slot"])
        self.finish(self.start(self.b, args=["--color=always", "build"]))
        self.assertIsNotNone(self.events()[-1]["slot"])

    def test_nested_standalone_workspace_passes_through(self):
        nested = self.a / "nested"
        nested.mkdir()
        (nested / "Cargo.toml").write_text("[workspace]\n")
        self.finish(self.start(nested))
        self.assertIsNone(self.events()[-1]["slot"])

    def test_free_slot_selection_is_lru_and_sticky(self):
        self.env["BROWSEROS_CARGO_POOL_SLOTS"] = "2"
        self.finish(self.start())
        first_slot = self.events()[-1]["slot"]
        self.finish(self.start(self.b))
        second_slot = self.events()[-1]["slot"]
        self.assertNotEqual(first_slot, second_slot)
        self.finish(self.start())
        self.assertEqual(self.events()[-1]["slot"], first_slot)
        self.finish(self.start(self.checkout("c")))
        self.assertEqual(self.events()[-1]["slot"], second_slot)

    def test_waiter_cancellation_does_not_start_cargo(self):
        gate = self.base / "gate"
        first = self.start(FAKE_WAIT=str(gate))
        self.await_starts(1)
        second = self.start(self.b)
        time.sleep(0.2)
        second.terminate()
        self.finish(second, 143)
        self.assertEqual(len(self.events()), 1)
        gate.touch()
        self.finish(first)

    def test_shrinking_pool_reclaims_retired_slots(self):
        self.env["BROWSEROS_CARGO_POOL_SLOTS"] = "2"
        self.finish(self.start())
        gate = self.base / "gate"
        active = self.start(self.b, FAKE_WAIT=str(gate))
        retired = Path(self.await_starts(2)[-1]["slot"])
        self.env["BROWSEROS_CARGO_POOL_SLOTS"] = "1"
        waiting = self.start(self.b)
        time.sleep(0.2)
        self.assertEqual(len(self.events()), 2)
        gate.touch()
        self.finish(active)
        self.finish(waiting)
        self.assertFalse(retired.exists())

    def test_multiple_installed_copies_find_real_cargo(self):
        other = self.base / "another-bin"
        other.mkdir()
        env = dict(self.env, PATH=f"{other}:{self.env['PATH']}")
        subprocess.run(
            [sys.executable, str(SOURCE), "install", "--bin-dir", str(other)],
            env=env,
            capture_output=True,
            check=True,
        )
        result = subprocess.run(
            [str(other / "cargo"), "--version"],
            env=env,
            cwd=self.a,
            capture_output=True,
            timeout=5,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_install_uninstall_idempotent_and_path_order(self):
        self.manage("install")
        self.assertIn("installed:", self.manage("status").stdout)
        self.manage("uninstall")
        self.manage("uninstall")
        self.assertIn("not installed:", self.manage("status").stdout)
        bad = dict(self.env, PATH=f"{self.base / 'real'}:{self.bin}:{os.environ['PATH']}")
        result = self.manage("install", bad, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("before real cargo", result.stderr)


if __name__ == "__main__":
    unittest.main()
