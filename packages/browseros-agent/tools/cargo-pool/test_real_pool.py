"""Real-Cargo regression: older local sources must survive a slot handoff."""

import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

SOURCE = Path(__file__).with_name("cargo-pool.py")


@unittest.skipUnless(shutil.which("cargo"), "real Cargo is required")
class RealCargoTests(unittest.TestCase):
    def test_older_sources_and_transitive_optional_path_dependency(self):
        with tempfile.TemporaryDirectory(prefix="cargo-pool-real-") as directory:
            base = Path(directory)
            bindir = base / "bin"
            bindir.mkdir()
            env = {
                k: v
                for k, v in os.environ.items()
                if not k.startswith(("CARGO_BUILD_", "CARGO_TARGET_", "BROWSEROS_CARGO_POOL_"))
            }
            env.update(
                PATH=f"{bindir}:{os.environ['PATH']}",
                BROWSEROS_CARGO_POOL_ROOT=str(base / "pool"),
                BROWSEROS_CARGO_POOL_SLOTS="1",
                CARGO_INCREMENTAL="0",
            )
            subprocess.run(
                [sys.executable, str(SOURCE), "install", "--bin-dir", str(bindir)],
                env=env,
                check=True,
                capture_output=True,
            )
            for owner in ("A", "B"):
                root = base / owner
                files = {
                    "Cargo.toml": '[workspace]\nmembers=["crates/*", "apps/claw-server"]\nresolver="2"\n',
                    "crates/browseros-core/Cargo.toml": '[package]\nname="browseros-core"\nversion="0.1.0"\nedition="2021"\n[dependencies]\nexternal={path="../../../external-'
                    + owner
                    + '",optional=true}\n[features]\nprobe=["dep:external"]\n',
                    "crates/browseros-core/src/lib.rs": '#[cfg(feature="probe")]\npub fn owner()-> &\'static str { external::owner() }\n',
                    "apps/claw-server/Cargo.toml": '[package]\nname="claw-server"\nversion="0.1.0"\nedition="2021"\n[dependencies]\nbrowseros-core={path="../../crates/browseros-core",features=["probe"]}\nitoa="1.0"\n',
                    "apps/claw-server/src/main.rs": 'fn main(){println!("{}",browseros_core::owner());}\n',
                }
                for name, content in files.items():
                    file = root / name
                    file.parent.mkdir(parents=True, exist_ok=True)
                    file.write_text(content)
                (root / ".cargo").mkdir()
                (root / ".cargo/config.toml").write_text(
                    f'[build]\nbuild-dir="{base / "old-shared"}"\n'
                )
                external = base / f"external-{owner}"
                (external / "src").mkdir(parents=True)
                (external / "Cargo.toml").write_text(
                    '[package]\nname="external"\nversion="0.1.0"\nedition="2021"\n'
                )
                (external / "src/lib.rs").write_text(
                    f'pub fn owner()-> &\'static str {{"{owner}"}}\n'
                )
                # Both workspace and non-member path sources predate A's build.
                for folder in (root, external):
                    for file in folder.rglob("*"):
                        os.utime(file, (946684800, 946684800))
            dependency = None
            dependency_stamp = None
            for owner in ("A", "B", "A"):
                root = base / owner
                result = subprocess.run(
                    [str(bindir / "cargo"), "build"],
                    check=False,
                    cwd=root,
                    env=env,
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                if dependency is None:
                    dependency = next(
                        (base / "pool/slot-0/build/debug/deps").glob("libitoa-*.rlib")
                    )
                    dependency_stamp = dependency.stat().st_mtime_ns
                else:
                    self.assertEqual(
                        dependency.stat().st_mtime_ns,
                        dependency_stamp,
                        "handoff should retain registry dependencies",
                    )
                binary = root / "target/debug/claw-server"
                self.assertEqual(subprocess.check_output([str(binary)], text=True).strip(), owner)
            # A no-run benchmark is another finite compilation, not a running
            # benchmark. In an old checkout its shared config must be overridden.
            for owner in ("A", "B"):
                root = base / owner
                manifest = root / "apps/claw-server/Cargo.toml"
                with manifest.open("a") as output:
                    output.write('\n[[bench]]\nname="probe"\nharness=false\n')
                benchmark = root / "apps/claw-server/benches/probe.rs"
                benchmark.parent.mkdir()
                benchmark.write_text('fn main(){println!("{}",browseros_core::owner());}\n')
                for path in root.rglob("*"):
                    if "target" not in path.parts:
                        os.utime(path, (946684800, 946684800))
            for owner in ("A", "B"):
                result = subprocess.run(
                    [str(bindir / "cargo"), "bench", "--no-run"],
                    cwd=base / owner,
                    env=env,
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                executable = next(
                    path
                    for path in (base / "pool/slot-0/build/release/deps").glob("probe-*")
                    if path.is_file() and os.access(path, os.X_OK)
                )
                self.assertEqual(
                    subprocess.check_output([str(executable)], text=True).strip(), owner
                )
            # Killing Cargo must also stop its compiler/build-script workers,
            # which could otherwise overwrite the next checkout's generated code.
            for owner in ("A", "B"):
                root = base / owner
                code = "pub fn owner()-> &'static str {" + json.dumps(owner) + "}"
                script = (
                    "use std::{env,fs,path::PathBuf,thread,time::Duration};\n"
                    'fn main(){let out=PathBuf::from(env::var("OUT_DIR").unwrap());'
                    'if let Ok(base)=env::var("POOL_TEST_WAIT") {'
                    "let base=PathBuf::from(base);"
                    'fs::write(base.join("script.pid"),std::process::id().to_string()).unwrap();'
                    'while !base.join("gate").exists(){thread::sleep(Duration::from_millis(10));}}'
                    "fs::create_dir_all(&out).unwrap();"
                    'fs::write(out.join("generated.rs"),' + json.dumps(code) + ").unwrap();"
                    'println!("cargo:rerun-if-changed=build.rs");}'
                )
                (root / "crates/browseros-core/build.rs").write_text(script)
                (root / "crates/browseros-core/src/lib.rs").write_text(
                    'include!(concat!(env!("OUT_DIR"),"/generated.rs"));\n'
                )
            child = subprocess.Popen(
                [str(bindir / "cargo"), "build"],
                cwd=base / "A",
                env=dict(env, POOL_TEST_WAIT=str(base)),
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            script_pid = None
            try:
                deadline = time.monotonic() + 15
                while not (base / "script.pid").exists():
                    self.assertIsNone(child.poll())
                    self.assertLess(time.monotonic(), deadline, "build script never started")
                    time.sleep(0.01)
                script_pid = int((base / "script.pid").read_text())
                cargo_pid = int(
                    subprocess.check_output(["ps", "-o", "ppid=", "-p", str(script_pid)], text=True)
                )
                os.kill(cargo_pid, signal.SIGKILL)
                self.assertEqual(child.wait(timeout=5), 137)
                for attempt in range(2):
                    if attempt:
                        (base / "gate").touch()
                        time.sleep(0.1)
                    result = subprocess.run(
                        [str(bindir / "cargo"), "build"],
                        cwd=base / "B",
                        env=env,
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(
                        subprocess.check_output(
                            [str(base / "B/target/debug/claw-server")], text=True
                        ).strip(),
                        "B",
                    )
            finally:
                if child.poll() is None:
                    child.terminate()
                    child.wait(timeout=5)
                if script_pid:
                    try:
                        os.kill(script_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            self.assertFalse((base / "old-shared").exists())
            # Missing old source must cause a safe wipe, not a failed handoff.
            shutil.rmtree(base / "A")
            result = subprocess.run(
                [str(bindir / "cargo"), "build"],
                check=False,
                cwd=base / "B",
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                subprocess.check_output(
                    [str(base / "B/target/debug/claw-server")], text=True
                ).strip(),
                "B",
            )

            # Preparatory metadata must honor the caller's lock/offline contract.
            root = base / "B"
            manifest = root / "apps/claw-server/Cargo.toml"
            manifest.write_text(manifest.read_text().replace('version="0.1.0"', 'version="0.2.0"'))
            lock_before = (root / "Cargo.lock").read_bytes()
            env["BROWSEROS_CARGO_POOL_ROOT"] = str(base / "fresh-pool")
            result = subprocess.run(
                [str(bindir / "cargo"), "build", "--locked", "--offline"],
                cwd=root,
                env=env,
                capture_output=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((root / "Cargo.lock").read_bytes(), lock_before)


if __name__ == "__main__":
    unittest.main()
