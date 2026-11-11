#!/usr/bin/env python3
"""Bound BrowserOS Cargo intermediates with exclusive, checkout-sticky leases.

The installed copy is named cargo. Its detached supervisor owns the lease and
waits for Cargo itself, so killing the foreground shim cannot free a live slot.
Only Python's standard library is needed on macOS and Linux.
"""

import argparse
import contextlib
import fcntl
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

COMPILE = {"build", "b", "check", "c", "test", "t", "clippy", "rustc", "rustdoc", "doc", "d"}
SIGNALS = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)
MARKER = b"Bound BrowserOS Cargo intermediates"


def is_shim(path):
    try:
        with path.open("rb") as source:
            return MARKER in source.read(300)
    except OSError:
        return False


def real_cargo():
    """Skip this installed shim, preserving rustup's cargo symlink/argv[0]."""
    own = Path(__file__).resolve()
    for entry in os.get_exec_path():
        candidate = Path(entry) / "cargo"
        if (
            candidate.is_file()
            and os.access(candidate, os.X_OK)
            and candidate.resolve() != own
            and not is_shim(candidate)
        ):
            return str(candidate.absolute())
    raise RuntimeError("no real cargo found after the shim on PATH")


def option(args, name):
    for i, arg in enumerate(args):
        if arg.startswith(name + "="):
            return arg.split("=", 1)[1]
        if arg == name and i + 1 < len(args):
            return args[i + 1]
    return None


def cargo_command(args):
    """Find the subcommand after Cargo's global flags and optional toolchain."""
    skip_value = False
    for arg in args:
        if skip_value:
            skip_value = False
        elif arg == "--color":
            skip_value = True
        elif not arg.startswith(("-", "+")):
            return arg
    return None


def workspace(args):
    """Recognize existing source layout, including old worktrees and mirrors."""
    args = args[: args.index("--")] if "--" in args else args
    if any(key in os.environ for key in ("CARGO_BUILD_BUILD_DIR", "CARGO_TARGET_DIR")):
        return None
    if any(
        a in ("--help", "-h", "--version", "-V", "--target-dir", "--config", "-C")
        or a.startswith(("--target-dir=", "--config=", "-Z"))
        for a in args
    ):
        return None
    command = cargo_command(args)
    if command not in COMPILE and not (command == "bench" and "--no-run" in args):
        return None
    manifest = option(args, "--manifest-path")
    start = Path(manifest).absolute().parent if manifest else Path.cwd()
    for root in (start, *start.parents):
        if (
            (root / "Cargo.toml").is_file()
            and (root / "crates/browseros-core/Cargo.toml").is_file()
            and (root / "apps/claw-server/Cargo.toml").is_file()
        ):
            return root.resolve()
        # A nested standalone workspace owns its own output and package set.
        # Walking past it would clean the outer workspace while leaving the
        # nested workspace's local crates behind on a later handoff.
        candidate = root / "Cargo.toml"
        if candidate.is_file() and "[workspace]" in candidate.read_text():
            break
    return None


def save(path, data):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(data))
    os.replace(temporary, path)


def read(path):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return {}


@contextlib.contextmanager
def locked(path, blocking=True):
    # CLOEXEC plus Popen(close_fds=True) keeps locks out of test servers/browsers.
    with path.open("a+") as handle:
        fcntl.flock(handle, fcntl.LOCK_EX | (0 if blocking else fcntl.LOCK_NB))
        yield handle


def tree_size(path):
    """Count allocated bytes (also account for files on platforms without st_blocks)."""
    total = 0
    for parent, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            stat = os.lstat(os.path.join(parent, name))
            total += getattr(stat, "st_blocks", (stat.st_size + 511) // 512) * 512
    return total


def wipe(slot):
    build = slot / "build"
    if build.is_symlink():
        raise RuntimeError(f"refusing symlink build directory: {build}")
    if build.exists():
        shutil.rmtree(build)


class CargoRunner:
    """Forward cancellation while retaining the lease through each Cargo exit.

    Metadata and handoff cleaning need the same lifetime guarantee as compilation:
    releasing during a still-running clean could delete the next owner's output.
    """

    def __init__(self, cargo):
        self.cargo = cargo
        self.child = None
        self.cancelled = None
        for signum in SIGNALS:
            signal.signal(signum, self.forward)

    def forward(self, signum, _frame):
        self.cancelled = signum
        if self.child is None or self.child.poll() is not None:
            raise SystemExit(128 + signum)
        try:
            os.killpg(self.child.pid, signum)
        except ProcessLookupError:
            pass

    def run(self, args, env, capture=False, check=True):
        # Block signals across spawning/publication: a signal must never observe
        # child=None after a Cargo process has actually started. The single-threaded
        # child restores the original signal mask before exec so Cargo can cancel.
        mask = signal.pthread_sigmask(signal.SIG_BLOCK, SIGNALS)
        try:
            self.child = subprocess.Popen(
                [self.cargo, *args],
                env=env,
                close_fds=True,
                start_new_session=True,
                preexec_fn=lambda: signal.pthread_sigmask(signal.SIG_SETMASK, mask),  # noqa: PLW1509
                stdout=subprocess.PIPE if capture else None,
            )
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, mask)
        output, _ = self.child.communicate()
        code = self.child.returncode
        if code or self.cancelled:
            # A killed Cargo cannot reap rustc/build-script workers. Stop its
            # private process group before releasing the lease: a surviving
            # build script could otherwise write generated A code into B's slot.
            # Successful tests may leave servers alive; they hold no lease.
            try:
                os.killpg(self.child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        if self.cancelled:
            raise SystemExit(128 + self.cancelled)
        if check and code:
            raise subprocess.CalledProcessError(code, args)
        return code, output


def metadata(runner, root, env, toolchain, frozen=False, flags=()):
    _, output = runner.run(
        [
            *toolchain,
            "metadata",
            "--format-version=1",
            "--all-features",
            "--manifest-path",
            str(root / "Cargo.toml"),
            *(["--locked", "--offline"] if frozen else flags),
        ],
        env,
        capture=True,
    )
    data = json.loads(output)
    # Full metadata includes optional, transitive and target-specific path deps.
    return sorted(
        (
            {"name": p["name"], "id": p["id"], "targets": p["targets"]}
            for p in data["packages"]
            if p.get("source") is None
        ),
        key=lambda p: p["id"],
    )


def clean_previous(runner, slot, previous, env):
    """Ask Cargo to remove local packages, retaining registry/git dependencies.

    Saved metadata is checked against the previous checkout before cleaning. If
    that checkout disappeared or changed its package/target layout, wipe instead:
    trusting an incomplete package list would reintroduce timestamp-fresh wrong
    code. Cleaning uses a disposable target dir, never the old checkout's target.
    """
    if not previous.get("ready") or previous.get("disposable"):
        wipe(slot)
        return
    root = Path(previous["owner"])
    clean_env = dict(env, CARGO_TARGET_DIR=str(slot / "clean-target"))
    try:
        packages = metadata(runner, root, clean_env, previous["toolchain"], frozen=True)
        if packages != previous["packages"]:
            wipe(slot)
            return
        args = [
            *previous["toolchain"],
            "clean",
            "--locked",
            "--offline",
            "--manifest-path",
            str(root / "Cargo.toml"),
        ]
        for package in packages:
            args += ["-p", package["name"]]
        args += previous["mode"]
        runner.run(args, clean_env)
    except (OSError, ValueError, subprocess.CalledProcessError):
        wipe(slot)
    finally:
        shutil.rmtree(slot / "clean-target", ignore_errors=True)


def lease(pool, count, owner):
    """Serialize allocation only; Cargo never runs under the allocator lock.

    Publish the new owner before unlocking allocation, so concurrent commands in
    one checkout find its busy slot and wait instead of taking a second slot.
    The ready=false journal makes interrupted handoffs require a full wipe.
    """
    pool.mkdir(parents=True, exist_ok=True)
    announced = False
    while True:
        with locked(pool / "allocator.lock"):
            slots = []
            for index in range(count):
                slot = pool / f"slot-{index}"
                slot.mkdir(exist_ok=True)
                slots.append((slot, read(slot / "state.json")))
            # Shrinking a pool reclaims idle retired slots under the same lock.
            # Lock files stay in place, so another waiter cannot lock a new inode.
            retiring = False
            for slot in pool.glob("slot-*"):
                if int(slot.name[5:]) >= count:
                    try:
                        with locked(slot / "lease.lock", False):
                            wipe(slot)
                            save(slot / "state.json", {})
                    except BlockingIOError:
                        retiring = True
            sticky = [item for item in slots if item[1].get("owner") == owner]
            candidates = sticky or sorted(slots, key=lambda item: item[1].get("used", 0))
            if retiring:
                candidates = []
            for slot, previous in candidates:
                handle = (slot / "lease.lock").open("a+")
                try:
                    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    handle.close()
                    continue
                save(slot / "state.json", {"owner": owner, "ready": False, "used": time.time_ns()})
                return slot, previous, handle
        if not announced:
            print("cargo-pool: waiting for a build slot", file=sys.stderr, flush=True)
            announced = True
        time.sleep(0.05)


def supervise(cargo, args, root):
    """Own the lease until wait() observes Cargo's exit, even if the shim dies."""
    runner = CargoRunner(cargo)
    pool = Path(
        os.environ.get(
            "BROWSEROS_CARGO_POOL_ROOT",
            str(
                Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
                / "build/browseros-pool"
            ),
        )
    ).absolute()
    count = int(os.environ.get("BROWSEROS_CARGO_POOL_SLOTS", "8"))
    limit = int(os.environ.get("BROWSEROS_CARGO_POOL_MAX_BYTES", str(7 * 1024**3 // 2)))
    if count < 1 or limit < 1:
        raise RuntimeError("pool slot count and max bytes must be positive")
    slot, previous, handle = lease(pool, count, str(root))
    try:
        env = dict(os.environ, CARGO_BUILD_BUILD_DIR=str(slot / "build"))
        toolchain = [args[0]] if args and args[0].startswith("+") else []
        mode = []
        compile_args = args[: args.index("--")] if "--" in args else args
        if "--release" in compile_args or "-r" in compile_args:
            mode += ["--release"]
        for name in ("--profile", "--target"):
            value = option(compile_args, name)
            if value:
                mode += [name, value]
        command = cargo_command(compile_args)
        # Documentation cannot be selectively cleaned by Cargo. Non-default
        # modes retain sticky reuse but are wiped on handoff or mode changes.
        disposable = (
            command in {"doc", "d", "rustdoc", "bench"}
            or bool(mode)
            or "CARGO_BUILD_TARGET" in os.environ
        )
        # Different output modes may accumulate unrelated profiles/targets. A
        # full wipe at the boundary keeps cleaning complete and disk predictable.
        same = previous.get("owner") == str(root) and previous.get("ready")
        compatible = (
            previous.get("mode") == mode
            and previous.get("toolchain") == toolchain
            and previous.get("disposable", False) == disposable
        )
        if (not previous.get("checked") and tree_size(slot / "build") > limit) or not compatible:
            wipe(slot)
            same = False
        elif not same and any(
            p.is_dir() and p.name not in {"debug", "tmp"} for p in (slot / "build").glob("*")
        ):
            wipe(slot)
        elif not same:
            clean_previous(runner, slot, previous, env)
        resolution_flags = [a for a in compile_args if a in {"--locked", "--offline", "--frozen"}]
        packages = (
            previous["packages"]
            if same
            else metadata(runner, root, env, toolchain, flags=resolution_flags)
        )
        state = {
            "owner": str(root),
            "packages": packages,
            "toolchain": toolchain,
            "mode": mode,
            "ready": True,
            "disposable": disposable,
            "used": time.time_ns(),
        }
        save(slot / "state.json", state)
        code, _ = runner.run(args, env, check=False)
        return 128 - code if code < 0 else code
    finally:
        # Only the supervisor inherits this fd; Cargo and its descendants never
        # do. Size eviction happens while still leased, including failed builds.
        try:
            if tree_size(slot / "build") > limit:
                wipe(slot)
            state = read(slot / "state.json")
            # Interrupted compilation is never reused, even by the same checkout.
            if runner.cancelled or (runner.child is not None and runner.child.returncode):
                state["ready"] = False
            state["checked"] = True
            state["used"] = time.time_ns()
            save(slot / "state.json", state)
        finally:
            handle.close()


def run_shim():
    cargo = real_cargo()
    args = sys.argv[1:]
    root = workspace(args)
    if root is None:
        os.execv(cargo, [cargo, *args])
    # The foreground PID is expendable. The detached child holds the OS lock
    # until its direct Cargo child exits, independent of grandchildren's lives.
    worker = os.fork()
    if worker == 0:
        os.setsid()
        try:
            code = supervise(cargo, args, root)
        except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
            print(f"cargo-pool: {error}", file=sys.stderr, flush=True)
            code = 1
        os._exit(code)

    def forward(signum, _frame):
        try:
            os.kill(worker, signum)
        except ProcessLookupError:
            pass

    for signum in SIGNALS:
        signal.signal(signum, forward)
    _, status = os.waitpid(worker, 0)
    return os.waitstatus_to_exitcode(status)


def manage():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("install", "uninstall", "status"))
    parser.add_argument("--bin-dir", type=Path, default=Path.home() / ".local/bin")
    args = parser.parse_args()
    destination = args.bin_dir.expanduser().absolute() / "cargo"
    installed = is_shim(destination)
    if args.command == "status":
        print(f"{'installed' if installed else 'not installed'}: {destination}")
        print(
            f"pool: {os.environ.get('BROWSEROS_CARGO_POOL_ROOT', str(Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo'))) / 'build/browseros-pool'))}"
        )
        return 0
    if args.command == "uninstall":
        if installed:
            destination.unlink()
        elif destination.exists():
            raise RuntimeError(f"refusing to remove another executable: {destination}")
        return 0
    entries = [Path(entry).absolute() for entry in os.get_exec_path()]
    cargo = real_cargo()
    if (
        cargo is None
        or destination.parent not in entries
        or entries.index(destination.parent) >= entries.index(Path(cargo).parent)
    ):
        raise RuntimeError("install directory must come before real cargo on PATH")
    if destination.exists() and not installed:
        raise RuntimeError(f"refusing to replace another executable: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(".pool-install")
    shutil.copyfile(__file__, temporary)
    temporary.chmod(0o755)
    os.replace(temporary, destination)
    print(f"installed: {destination}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(run_shim() if Path(sys.argv[0]).name == "cargo" else manage())
    except (OSError, RuntimeError, ValueError) as error:
        print(f"cargo-pool: {error}", file=sys.stderr)
        sys.exit(1)
