#!/usr/bin/env python3
"""Send an explicit foreground job to an always-on Crabbox/SSH fleet.

The shared SSH coordinator owns VM slots. This client freezes approved working
files locally, never uploads Git metadata or ambient credentials, and releases
its slot only after the guest supervisor confirms completion or cancellation.
"""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import os
import platform
import re
from pathlib import Path, PurePosixPath
import shlex
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import uuid

VERSION = "0.83.0"
HELPER = "/usr/local/lib/browseros-remote-builds/"
INCLUDE = ["packages/browseros-agent", ".github/workflows/test.yml"]
LEGACY_SUPERVISOR = ["packages/browseros/docker/neo/" + name for name in
                     ("neo-server-info.sh", "openbox-rc.xml", "entrypoint.sh", "healthcheck.sh", "neo-quit.py")]
JOB_KINDS = {"gate", "image", "viewer-image", "spec", "detector", "run"}
EXCLUDE = [".git", ".internal-docs", ".llm", ".scratch", ".grove", ".agents", ".claude", ".auctor", ".crabbox",
           ".env", ".env.*", "*.pem", "*.key", "*.p12", "*.pfx", "id_rsa*", "id_ed25519*",
           ".credentials", ".credentials.*", "credentials.json", "credentials.yaml", "credentials.yml",
           "credentials.toml", "secrets.json", "secrets.yaml", "secrets.yml", "secrets.toml", ".azure", ".aws",
           ".ssh", ".gnupg", ".npmrc", ".netrc", ".pypirc", ".git-credentials", ".docker"]


class Failure(RuntimeError):
    pass


def job_kind(action: str, arguments: list[str]) -> str:
    """Classify explicit `run` calls too, so a spec batch cannot hide as generic work."""
    if action != "run":
        return action
    command = " ".join(arguments)
    if any(value in command for value in ("test:neo-spec", "recipes.sh spec", "neo-playwright-tests/spec.ts", "neo-playwright-tests/node/run.mjs")):
        return "spec"
    if "devtools_detection_live" in command or "recipes.sh detector" in command:
        return "detector"
    if "crates/neo-sdk/check.sh" in command:
        return "gate"
    if "neo-server/docker/build.sh" in command or re.search(r"\bdocker\s+(?:buildx\s+)?build\b", command):
        return "image"
    return "run"


def worker_info(repository: Path, profile: dict, kind: str) -> dict:
    """A worktree is a worker; aliases group the administrator's proof checkouts.

    This is scheduling/display identity for trusted builders, not an authentication
    boundary. Hostnames and full paths are hashed rather than sent as display text.
    """
    canonical = Path(profile.get("workerAliases", {}).get(str(repository), str(repository)))
    label = canonical.name
    temporary = re.search(r"-wt-feat-([^/]+)", str(canonical))
    if temporary:
        label = temporary.group(1)
    machine = hashlib.sha256(platform.node().encode()).hexdigest()[:16]
    try:
        common = checked(["git", "-C", str(canonical), "rev-parse", "--git-common-dir"]).strip()
        common = str((canonical / common).resolve())
    except (Failure, OSError):
        common = str(canonical)  # A retired legacy checkout may still have a lease to reap.
    identity = hashlib.sha256((machine + "\0" + common + "\0" + label).encode()).hexdigest()[:32]
    return {"worker": identity, "label": label, "kind": kind,
            "clientPid": os.getpid(), "clientMachine": machine, "protocol": 2}


def queue_report(client) -> None:
    """Show shared metadata plus pre-upgrade waiters still alive on this Mac.

    Older clients did not register waiters. Read their process/cwd metadata until
    they drain; never signal or alter another worker just to inspect the queue.
    """
    state = client.control("queue")
    runs = Path(client.profile.get("runs", "~/.local/state/browseros-remote-builds/runs")).expanduser()
    rows = {}
    local = []
    try:
        output = checked(["ps", "-ww", "-axo", "pid=,ppid=,command="])
        for line in output.splitlines():
            parts = line.strip().split(None, 2)
            if len(parts) == 3:
                rows[int(parts[0])] = (int(parts[1]), parts[2])
        for pid, (_, command) in rows.items():
            try:
                args = shlex.split(command)
            except ValueError:
                continue
            index = next((n for n, value in enumerate(args) if value.endswith("remote.py")
                          and "remote-builds" in value), None)
            if index is None:
                continue
            tail = args[index + 1:]
            if tail[:1] == ["--config"]:
                tail = tail[2:]
            if not tail or tail[0] not in JOB_KINDS:
                continue
            try:
                cwd = checked(["lsof", "-a", "-p", str(pid), "-d", "cwd", "-Fn"])
                directory = next((line[1:] for line in cwd.splitlines() if line.startswith("n")), "")
                repository = Path(checked(["git", "-C", directory, "rev-parse", "--show-toplevel"]).strip())
                info = worker_info(repository, client.profile, job_kind(tail[0], tail[1:]))
            except (Failure, OSError, ValueError):
                continue  # A client may finish while this read-only view is collected.
            descendants = {pid}
            while True:
                expanded = descendants | {child for child, (parent, _) in rows.items() if parent in descendants}
                if expanded == descendants:
                    break
                descendants = expanded
            info.update(clientPid=pid, commands="\n".join(rows[child][1] for child in descendants))
            local.append(info)
    except (Failure, OSError, ValueError):
        # Other platforms still see every new registered request through SSH.
        pass
    active = [host for host in state["hosts"] if host.get("job")]
    print(f"RUNNING {len(active)}/{len(state['hosts'])}")
    for host in active:
        job = host["job"]
        match = next((row for row in local if job["id"] in row["commands"]), None)
        saved = runs / job["id"] / "job.json"
        try:
            metadata = json.loads(saved.read_text())
        except (OSError, ValueError):
            # A client may still be adding its host receipt; central queue metadata
            # remains authoritative, and a display fallback must not break the view.
            metadata = {}
        label = job.get("label") or (match or {}).get("label")
        if not label and metadata.get("worktree"):
            label = worker_info(Path(metadata["worktree"]), client.profile, "run")["label"]
        commands = "\n".join(command for _, command in rows.values() if job["id"] in command)
        kind = job.get("kind") or (match or {}).get("kind") or metadata.get("kind")
        if not kind:
            kind = job_kind("run", [commands])
        if kind == "run":
            kind = job_kind("run", [commands])
        print(f"  {host['name']}  {label or 'legacy/unknown'}  {kind}  {job['id'][:8]}")
    waiting = list(state.get("waiting", []))
    registered = {(row.get("clientMachine"), row.get("clientPid")) for row in waiting}
    for row in local:
        if (row["clientMachine"], row["clientPid"]) in registered:
            continue
        if any(host["job"]["id"] in row["commands"] for host in active):
            continue
        if any("guest.py " + operation in row["commands"] for operation in ("artifacts", "cleanup", "image")):
            continue  # Results/export happen after releasing compute.
        waiting.append({**row, "id": "legacy:" + str(row["clientPid"])})
    print(f"WAITING {len(waiting)} (includes pre-upgrade clients visible on this Mac)")
    for row in waiting:
        print(f"  {row.get('label', 'unknown')}  {row.get('kind', 'run')}  {row['id'][:16]}")


def checked(argv: list[str], **options) -> str:
    result = subprocess.run(argv, capture_output=True, text=True, **options)
    if result.returncode:
        raise Failure(result.stderr.strip() or f"command failed with exit {result.returncode}")
    return result.stdout


def clean_environment(config: Path, job: str = "") -> dict:
    # Config-file isolation alone does not override CRABBOX_* environment values.
    # Remove inherited overrides before selecting this job's reviewed transfer policy.
    environment = {key: value for key, value in os.environ.items() if not key.startswith("CRABBOX_")}
    environment["CRABBOX_CONFIG"] = str(config)
    if job:
        environment["CRABBOX_STATIC_ID"] = job
    return environment


def source_policy(key: Path, include: list[str] | None = None) -> dict:
    return {"provider": "ssh", "target": "linux", "architecture": "amd64",
            "workRoot": "/work/remote-builds", "static": {"user": "buildrunner", "port": "22"},
            "ssh": {"key": str(key), "port": "22"}, "env": {"allow": []},
            "sync": {"source": "git", "gitSeed": False,
                     "gitSeedSource": "origin", "gitOverlay": False, "delete": True,
                     "include": INCLUDE if include is None else include, "exclude": EXCLUDE}}


def manifest(crabbox: str, root: Path, config: Path) -> list[dict]:
    data = json.loads(checked([crabbox, "sync-plan", "--json", "--limit", "200000"],
                              cwd=root, env=clean_environment(config)))
    files = data["topFiles"]
    if len(files) != data["candidate"]["files"]:
        raise Failure("sync-plan did not enumerate the entire selected file set")
    return sorted(files, key=lambda row: row["path"])


def fingerprint(path: Path, allow_link: bool = False, allow_directory: bool = False) -> tuple:
    value = path.lstat()
    if (not stat.S_ISREG(value.st_mode) and not (allow_link and stat.S_ISLNK(value.st_mode))
            and not (allow_directory and stat.S_ISDIR(value.st_mode))):
        raise Failure("source contains a symlink or non-regular file")
    return value.st_dev, value.st_ino, value.st_size, value.st_mode, value.st_mtime_ns, value.st_ctime_ns


def tracked_symlinks(repository: Path, include: list[str]) -> set[str]:
    entries = checked(["git", "ls-files", "--stage", "-z", "--", *include], cwd=repository)
    return {entry.split("\t", 1)[1] for entry in entries.split("\0")
            if entry.startswith("120000 ") and "\t" in entry}


def snapshot_source(repository: Path, relative: Path, selected: set[str], links: set[str]) -> tuple[Path, tuple]:
    """Dereference only tracked file links to other explicitly approved source files.

    Materializing a link keeps the remote archive inert: neither Crabbox nor tar can
    follow it into an excluded file. Observe every hop again before accepting bytes;
    directory links, absolute targets, escapes and cycles are never eligible.
    """
    observed = []
    seen = set()
    while True:
        if relative.as_posix() not in selected or relative in seen:
            raise Failure("symlink target is outside the approved transfer manifest or forms a cycle")
        seen.add(relative)
        for index in range(1, len(relative.parts)):
            if (repository / Path(*relative.parts[:index])).is_symlink():
                raise Failure("symlink source directories are not transferred")
        source = repository / relative
        before = fingerprint(source, allow_link=True)
        observed.append((relative.as_posix(), before))
        if not source.is_symlink():
            return source, tuple(observed)
        if relative.as_posix() not in links:
            raise Failure("only tracked relative source symlinks may be transferred")
        target = Path(os.readlink(source))
        if target.is_absolute():
            raise Failure("absolute source symlinks are not transferred")
        # Inspect the actual traversal before collapsing '..'. For example,
        # '.scratch/../CLAUDE.md' can resolve outside this tree when .scratch is
        # a directory link, even though lexical normalization looks harmless.
        directories = {parent for path in selected for parent in Path(path).parents}
        candidate = relative.parent
        for index, component in enumerate(target.parts):
            if component == "..":
                if candidate == Path("."):
                    raise Failure("source symlink escapes the transferred tree")
                candidate = candidate.parent
            else:
                candidate /= component
            if index < len(target.parts) - 1:
                directory = repository / candidate
                if candidate not in directories or directory.is_symlink() or not directory.is_dir():
                    raise Failure("source symlink traverses an excluded path or directory link")
                observed.append((candidate.as_posix(), fingerprint(directory, allow_directory=True)))
        relative = candidate


def index_snapshot(snapshot: Path) -> None:
    """Use an uncommitted local index to preserve legitimate tracked src/target paths.

    Crabbox's directory mode applies broad built-in build-output filters. Its Git mode
    preserves explicitly indexed source paths. This index has no commits or original
    history, and gitSeed/gitOverlay plus explicit .git exclusion keep it on the Mac.
    """
    checked(["git", "init", "--quiet", str(snapshot)])
    checked(["git", "add", "--force", "--", "."], cwd=snapshot)


def freeze(crabbox: str, repository: Path, destination: Path, config: Path) -> dict:
    """Copy only the audited manifest, rejecting edits during snapshot acceptance."""
    include = json.loads(config.read_text())["sync"]["include"]
    revision = checked(["git", "rev-parse", "HEAD"], cwd=repository).strip()
    dirty = checked(["git", "status", "--porcelain", "--", *include], cwd=repository)
    selected = manifest(crabbox, repository, config)
    paths = {row["path"] for row in selected}
    links = tracked_symlinks(repository, include)
    records = []
    for row in selected:
        relative = PurePosixPath(row["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise Failure("invalid source path")
        source, observed = snapshot_source(repository, Path(relative), paths, links)
        before = fingerprint(source)
        data = source.read_bytes()
        if snapshot_source(repository, Path(relative), paths, links) != (source, observed):
            raise Failure("source changed during snapshot; retry after the edit finishes")
        # Native excludes are authoritative, but this is a second boundary for accidental
        # policy regressions. Rust modules named credentials.rs or src/target remain valid.
        if any(part in {".git", ".llm", ".scratch", ".internal-docs", ".azure", ".aws"}
               or part == ".env" or part.startswith(".env.") for part in relative.parts):
            raise Failure("forbidden metadata or environment path in source manifest")
        if re.search(rb"-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----\r?\n", data):
            raise Failure("private-key material in selected source")
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        target.chmod(stat.S_IMODE(before[3]))
        records.append({"path": str(relative), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                        "observed": observed})
    # Verify again after copying, so an editor cannot change early files while later
    # files are copied and silently produce a mixed source snapshot.
    for record in records:
        source, observed = snapshot_source(repository, Path(record["path"]), paths, links)
        if observed != record.pop("observed") or hashlib.sha256(source.read_bytes()).hexdigest() != record["sha256"]:
            raise Failure("source changed during snapshot; retry after the edit finishes")
    if manifest(crabbox, repository, config) != selected or checked(["git", "rev-parse", "HEAD"], cwd=repository).strip() != revision:
        raise Failure("source selection or HEAD changed during snapshot")
    if (checked(["git", "status", "--porcelain", "--", *include], cwd=repository) != dirty
            or tracked_symlinks(repository, include) != links):
        raise Failure("source index changed during snapshot")
    encoded = json.dumps(records, sort_keys=True, separators=(",", ":")).encode()
    manifest_path = destination.parent / "source-files.json"
    manifest_path.write_bytes(encoded)
    manifest_path.chmod(0o600)
    return {"revision": revision, "dirty": bool(dirty), "files": len(records),
            "bytes": sum(record["bytes"] for record in records), "sha256": hashlib.sha256(encoded).hexdigest()}


class PoolClient:
    """SSH transport to the shared slot file and a reserved guest; keys stay local."""
    def __init__(self, profile: dict):
        self.profile = profile
        self.key = Path(profile["key"]).expanduser().resolve()
        self.known_hosts = self.key.parent / "known_hosts"
        self.submission = None
        if not self.key.is_file() or not self.known_hosts.is_file():
            raise Failure("configure a private SSH key and its adjacent pinned known_hosts file")

    def ssh_args(self, host: dict, command: list[str]) -> list[str]:
        target = host.get("user", "buildrunner") + "@" + host["host"]
        args = ["ssh", "-i", str(self.key), "-p", str(host.get("port", 22)),
                "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes", "-o", "ForwardAgent=no",
                "-o", "ControlMaster=no", "-o", "ControlPath=none",
                "-o", "StrictHostKeyChecking=yes", "-o", "UserKnownHostsFile=" + str(self.known_hosts),
                "-o", "ConnectTimeout=30", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=4",
                target, shlex.join(command)]
        return args

    def ssh(self, host: dict, command: list[str], **options) -> str:
        return checked(self.ssh_args(host, command), **options)

    def artifacts(self, host: dict, job: str, run_dir: Path) -> None:
        # Save an archive without extracting remote paths into the developer's tree.
        # A failed download leaves the remote directory available for manual recovery.
        archive = run_dir / "artifacts.tar.gz"
        if not archive.exists():
            with tempfile.NamedTemporaryFile(prefix="artifacts-", suffix=".partial", dir=run_dir,
                                             delete=False) as output:
                temporary = Path(output.name)
                result = subprocess.run(self.ssh_args(host, ["python3", HELPER + "guest.py",
                                        "artifacts", "--job", job]), stdout=output)
            if result.returncode:
                raise Failure("artifact download failed; remote job directory retained")
            temporary.replace(archive)
        with tarfile.open(archive) as contents:
            try:
                log = contents.extractfile(job + ".log")
            except KeyError:  # Legacy pilot receipts predate independent guest logging.
                log = None
            if log is not None:
                with log, tempfile.NamedTemporaryFile(prefix="guest-log-", dir=run_dir, delete=False) as output:
                    shutil.copyfileobj(log, output)
                    saved_log = Path(output.name)
                saved_log.replace(run_dir / "guest.log")
        self.guest(host, "cleanup", job)

    def fetch_image(self, job: str) -> None:
        if not re.fullmatch(r"[a-f0-9]{32}", job):
            raise Failure("fetch-image requires a job UUID")
        run_dir = Path(self.profile.get("runs", "~/.local/state/browseros-remote-builds/runs")).expanduser() / job
        host = json.loads((run_dir / "job.json").read_text())["host"]
        image = json.loads((run_dir / "result.json").read_text())["image"]
        if image.get("tag") != "neo-sdk:remote-" + job or not re.fullmatch(r"sha256:[0-9a-f]{64}", image.get("id", "")):
            raise Failure("local result does not describe this job's image")
        existing = subprocess.run(["docker", "image", "inspect", image["tag"], "--format", "{{.Id}}"],
                                  capture_output=True, text=True)
        if existing.returncode == 0 and existing.stdout.strip() != image["id"]:
            raise Failure("another local image already owns this tag; refusing to replace it")
        archive = run_dir / "image.tar.gz.partial"
        with archive.open("wb") as output:
            result = subprocess.run(self.ssh_args(host, ["python3", HELPER + "guest.py",
                                    "image", "--job", job]), stdout=output)
        if result.returncode:
            raise Failure("image export failed; no local image was loaded")
        subprocess.run(["docker", "load", "--input", str(archive)], check=True)
        actual = checked(["docker", "image", "inspect", image["tag"], "--format", "{{.Id}}"]).strip()
        if actual != image["id"]:
            raise Failure("loaded image digest differs from the remote result")
        archive.unlink()
        print("Viewer image loaded: " + image["tag"])
        # A worker must not restart the owner's default viewer or claim its port.
        print("Run: bun run neo:viewer -- --name remote-" + job + " --port 0 --image " + image["tag"])

    def control(self, operation: str, job: str = "") -> dict:
        # UUID-addressed operations are idempotent. A Mac scheduling stall or lost
        # reply can be retried without reserving a second VM or releasing another job.
        for attempt in range(3):
            try:
                result = subprocess.run(self.ssh_args(self.profile["coordinator"], ["python3", HELPER + "pool.py"]),
                                        input=json.dumps({"operation": operation, "job": job, "lease": True,
                                                          "submission": self.submission}),
                                        capture_output=True, text=True, timeout=100)
                # Application refusals are definitive, not a reason to retry admission.
                # Parse stdout even on exit 2 so the caller sees its two existing jobs.
                if result.stdout.strip():
                    value = json.loads(result.stdout)
                    break
                raise Failure(result.stderr.strip() or "coordinator returned no result")
            except (Failure, ValueError, subprocess.TimeoutExpired):
                if attempt == 2:
                    raise
                time.sleep(attempt + 1)
        if "error" in value:
            raise Failure(value["error"])
        return value

    @contextlib.contextmanager
    def queue_guard(self, job: str, run_dir: Path):
        """An IPC observer withdraws a waiter even if its client is SIGKILLed.

        The observer has its own process group, while only the parent holds the
        pipe's write end. Whole-Mac loss is covered by the short server-side expiry.
        Withdrawal never frees a potentially running reservation.
        """
        profile = run_dir / "queue-profile.json"
        profile.write_text(json.dumps(self.profile))
        profile.chmod(0o600)
        with (run_dir / "queue-watch.log").open("a") as diagnostic:
            watcher = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--queue-watch", str(profile), job],
                                       stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=diagnostic,
                                       start_new_session=True, close_fds=True)
        try:
            yield
        finally:
            watcher.stdin.close()
            with contextlib.suppress(subprocess.TimeoutExpired):
                watcher.wait(timeout=2)

    @contextlib.contextmanager
    def lease(self, job: str):
        """Renew independently of log output and long Crabbox sync/build subprocesses.

        The coordinator reaper survives this client. If the Mac stalls or disappears
        for two minutes, it cancels the UUID before permitting another job on its VM.
        """
        stopped = threading.Event()

        def renew():
            while not stopped.wait(20):
                try:
                    if not self.control("heartbeat", job).get("renewed"):
                        return
                except (Failure, OSError, ValueError, subprocess.SubprocessError) as error:
                    print(f"Lease renewal failed for {job}: {error}", file=sys.stderr)

        thread = threading.Thread(target=renew, name="remote-build-lease", daemon=True)
        thread.start()
        try:
            yield
        finally:
            stopped.set()
            thread.join(timeout=1)

    def guest(self, host: dict, operation: str, job: str) -> dict:
        return json.loads(self.ssh(host, ["python3", HELPER + "guest.py", operation, "--job", job]))

    def acquire(self, job: str) -> dict:
        reported_wait = False
        while True:
            host = self.control("acquire", job)["host"]
            if host:
                try:
                    self.ssh(host, ["true"], timeout=40)
                    return host
                except (Failure, subprocess.TimeoutExpired):
                    # Nothing has been dispatched yet. Cool down this host for all
                    # workers and try the next free slot, using the same job UUID.
                    self.control("unavailable", job)
                    print(f"{host['name']} did not answer SSH; trying another free host.", file=sys.stderr)
                    continue
            if not reported_wait:
                print("All machines busy; waiting for the next free slot.", file=sys.stderr)
                reported_wait = True
            time.sleep(2)


def command_for(action: str, arguments: list[str]) -> list[str]:
    arguments = arguments[1:] if arguments[:1] == ["--"] else arguments
    if action == "run":
        if not arguments:
            raise Failure("run needs a command after --")
        return arguments
    if action == "gate":
        return ["bash", "crates/neo-sdk/check.sh", *arguments]
    if action == "viewer-image":
        return ["bash", HELPER + "recipes.sh", "image", "--platform", arguments[0]]
    return ["bash", HELPER + "recipes.sh", action, *arguments]


def run_directory(profile: dict, job: str) -> Path:
    if not re.fullmatch(r"[a-f0-9]{32}", job):
        raise Failure("expected a 32-character hexadecimal job identifier")
    return Path(profile.get("runs", "~/.local/state/browseros-remote-builds/runs")).expanduser() / job


def save_receipt(run_dir: Path, job: str, receipt: dict) -> None:
    if receipt.get("job") != job:
        raise Failure("guest returned a result for a different job")
    (run_dir / "result.json").write_text(json.dumps(receipt, indent=2))


def execute(profile: dict, action: str, arguments: list[str]) -> int:
    client = PoolClient(profile)
    if action == "viewer-image":
        if arguments:
            raise Failure("viewer-image takes no arguments; it uses the local Docker architecture")
        arch = checked(["docker", "version", "--format", "{{.Server.Arch}}"]).strip()
        if arch not in {"amd64", "arm64"}:
            raise Failure("viewer-image requires an amd64 or arm64 local Docker engine")
        arguments = ["linux/" + arch]
    crabbox = str(Path(profile.get("crabbox", shutil.which("crabbox") or "crabbox")).expanduser())
    if checked([crabbox, "version"]).strip() != VERSION:
        raise Failure("this client requires the complete Crabbox v0.83.0 release")
    repository = Path(checked(["git", "rev-parse", "--show-toplevel"]).strip())
    current = Path.cwd().resolve().relative_to(repository.resolve())
    cwd = current if action == "run" else Path("packages/browseros-agent")
    job = uuid.uuid4().hex
    runs = Path(profile.get("runs", "~/.local/state/browseros-remote-builds/runs")).expanduser()
    run_dir = runs / job
    run_dir.mkdir(parents=True, mode=0o700)
    config = run_dir / "crabbox.json"
    include = list(INCLUDE)
    dockerfile = repository / "packages/browseros-agent/crates/neo-sdk/neo-server/docker/Dockerfile"
    if action in {"image", "viewer-image"} and "--from=neo-docker" in dockerfile.read_text():
        # Older SDK images share five supervisor sources with production. Select
        # only these actual inputs, without editing them or sending that whole tree.
        include.extend(LEGACY_SUPERVISOR)
    config.write_text(json.dumps(source_policy(client.key, include)))
    config.chmod(0o600)
    host = None
    acquiring = False
    safe = False
    process = None
    with tempfile.TemporaryDirectory(prefix="source-", dir=run_dir) as name:
        snapshot = Path(name)
        metadata = freeze(crabbox, repository, snapshot, config)
        (run_dir / "source.json").write_text(json.dumps(metadata, indent=2))
        index_snapshot(snapshot)
        # Preview the frozen index too; none of its metadata is eligible for transfer.
        frozen = manifest(crabbox, snapshot, config)
        if len(frozen) != metadata["files"] or sum(row["bytes"] for row in frozen) != metadata["bytes"]:
            raise Failure("frozen transfer differs from accepted source manifest")
        print(f"Job {job}; logs: {run_dir}", file=sys.stderr)
        client.submission = worker_info(repository, profile, job_kind(action, arguments))
        job_metadata = {"job": job, "source": metadata, "worktree": str(repository),
                        "kind": action, **client.submission}
        (run_dir / "job.json").write_text(json.dumps(job_metadata))
        prefix = ["python3", HELPER + "guest.py", "run", "--job", job, "--source-root"]
        command = ["--cwd", str(cwd), "--revision", metadata["revision"],
                   "--dirty", str(metadata["dirty"]).lower(), "--", *command_for(action, arguments)]
        # Only the owned source-root position expands on the guest. All caller
        # arguments, including a literal "$PWD", retain their exact argument value.
        shell = "exec " + shlex.join(prefix) + ' "$PWD" ' + shlex.join(command)
        try:
            acquiring = True
            with client.queue_guard(job, run_dir):
                host = client.acquire(job)
            acquiring = False
            (run_dir / "job.json").write_text(json.dumps({**job_metadata, "host": host}))
            print(f"Running on {host['name']}", file=sys.stderr)
            argv = [crabbox, "run", "--provider", "ssh", "--static-host", host["host"],
                    "--static-user", host.get("user", "buildrunner"), "--static-port", str(host.get("port", 22)),
                    "--static-no-control-master=true",
                    "--no-hydrate", "--record-local", "--timing-json", "--", "bash", "-lc", shell]
            with client.lease(job), (run_dir / "run.log").open("w") as output:
                process = subprocess.Popen(argv, cwd=snapshot, env=clean_environment(config, job),
                                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
                for line in process.stdout:
                    output.write(line)
                    output.flush()
                    print(line, end="", flush=True)
                result = process.wait()
            receipt = client.guest(host, "result", job)
            save_receipt(run_dir, job, receipt)
            safe = bool(receipt.get("safeToRelease"))
            if safe:
                # Free compute immediately after completion. Artifact reads and deleting
                # this UUID's directory can proceed alongside the next job's own directory.
                client.control("release", job)
                finished_host, host = host, None
                client.artifacts(finished_host, job, run_dir)
                if action == "viewer-image" and result == 0:
                    client.fetch_image(job)
                    print("Run native acceptance under your local heavy/image locks: bash "
                          "crates/neo-sdk/neo-server/docker/accept.sh neo-sdk:remote-" + job)
            return result
        except KeyboardInterrupt:
            if process and process.poll() is None:
                process.send_signal(2)
                with contextlib.suppress(subprocess.TimeoutExpired):
                    process.wait(timeout=20)
            if host:
                receipt = client.guest(host, "cancel", job)
                save_receipt(run_dir, job, receipt)
                safe = bool(receipt.get("safeToRelease"))
                if safe:
                    client.control("release", job)
                    finished_host, host = host, None
                    client.artifacts(finished_host, job, run_dir)
            return 130
        finally:
            if acquiring:
                with contextlib.suppress(Failure, OSError, subprocess.SubprocessError):
                    client.control("withdraw", job)
            if host:
                if safe:
                    client.control("release", job)
                else:
                    client.control("unknown", job)
                    print(f"Slot retained: run remote recover {job} after checking the job.", file=sys.stderr)


def main() -> int:
    if sys.argv[1:2] == ["--queue-watch"]:
        if len(sys.argv) != 4 or not re.fullmatch(r"[a-f0-9]{32}", sys.argv[3]):
            return 2
        try:
            profile = json.loads(Path(sys.argv[2]).read_text())
            sys.stdin.buffer.read()  # EOF proves that this submitting process released its pipe.
            PoolClient(profile).control("withdraw", sys.argv[3])
            return 0
        except (Failure, OSError, ValueError, subprocess.SubprocessError) as error:
            print(f"queue withdrawal: {error}", file=sys.stderr)
            return 1
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path,
                        default=Path(os.environ.get("REMOTE_BUILDS_CONFIG", "~/.config/browseros-remote-builds/config.json")).expanduser())
    parser.add_argument("action", choices=["run", "gate", "image", "viewer-image", "spec", "detector", "status", "queue", "recover", "fetch-image"])
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    arguments = parser.parse_args()
    try:
        profile = json.loads(arguments.config.read_text())
        if arguments.action == "queue":
            queue_report(PoolClient(profile))
            return 0
        if arguments.action == "fetch-image":
            if len(arguments.arguments) != 1:
                raise Failure("fetch-image needs exactly one completed image job UUID")
            PoolClient(profile).fetch_image(arguments.arguments[0])
            return 0
        if arguments.action == "status":
            for host in PoolClient(profile).control("status")["hosts"]:
                state = (host.get("job") or {}).get("id", "free")
                if host.get("maintenance"):
                    state += " (maintenance)"
                elif host.get("unavailableUntil", 0) > time.time():
                    state += " (SSH cooldown)"
                print(host["name"], state)
            return 0
        if arguments.action == "recover":
            if len(arguments.arguments) != 1:
                raise Failure("recover requires exactly one job identifier")
            job = arguments.arguments[0]
            run_dir = run_directory(profile, job)
            client = PoolClient(profile)
            host = next((host for host in client.control("status")["hosts"]
                         if (host.get("job") or {}).get("id") == job), None)
            if host:
                receipt = client.guest(host, "cancel", job)
            else:
                # Artifact collection can fail after a successful release. Resume it
                # using the saved endpoint, without cancelling a later job on that VM.
                host = json.loads((run_dir / "job.json").read_text())["host"]
                receipt = client.guest(host, "result", job)
            if not receipt.get("safeToRelease"):
                raise Failure("job process state remains uncertain")
            run_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
            if not (run_dir / "job.json").exists():
                (run_dir / "job.json").write_text(json.dumps({"job": job, "host": host}))
            save_receipt(run_dir, job, receipt)
            client.control("release", job)
            client.artifacts(host, job, run_dir)
            print("Job stopped; result/logs collected, source removed, slot free. VM remains running.")
            return 0
        return execute(profile, arguments.action, arguments.arguments)
    except (Failure, OSError, ValueError, KeyError, tarfile.TarError, subprocess.SubprocessError) as error:
        print(f"remote: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
