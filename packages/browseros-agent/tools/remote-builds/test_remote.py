"""Behavioral checks for shared admission, transfer boundaries and snapshot provenance."""
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
AGENT = HERE.parent.parent
REPO = AGENT.parent.parent
spec = importlib.util.spec_from_file_location("remote", HERE / "remote.py")
remote = importlib.util.module_from_spec(spec)
spec.loader.exec_module(remote)
CRABBOX = os.environ.get("TEST_CRABBOX")
guest_spec = importlib.util.spec_from_file_location("guest", HERE / "guest.py")
guest = importlib.util.module_from_spec(guest_spec)
guest_spec.loader.exec_module(guest)
sys.path.insert(0, str(HERE))
import pool
import reaper
sys.path.pop(0)


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.root_patch = patch.object(guest, "ROOT", self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def test_completed_cancellation_cannot_clean_a_later_jobs_docker(self):
        old, current = "a" * 32, "b" * 32
        terminal = {"job": old, "exitCode": 130, "safeToRelease": True}
        (self.root / (old + ".result.json")).write_text(json.dumps(terminal))
        (self.root / "owner.json").write_text(json.dumps({"job": current}))
        # B owns the VM while two stale recovery calls for A arrive.
        with guest.host_guard(current), patch.object(guest.subprocess, "run", return_value=SimpleNamespace(returncode=0)), \
                patch.object(guest, "cleanup_docker", side_effect=AssertionError("must not clean B")):
            self.assertEqual(terminal, guest.cancel(old))
            self.assertEqual(terminal, guest.cancel(old))
        self.assertEqual(current, json.loads((self.root / "owner.json").read_text())["job"])

    def test_an_orphaned_host_owner_blocks_admission_until_recovered(self):
        old = "a" * 32
        (self.root / "owner.json").write_text(json.dumps({"job": old}))
        with self.assertRaisesRegex(ValueError, "previous host owner"):
            with guest.host_guard("b" * 32):
                self.fail("admitted another job while the old outcome was unknown")
        with guest.host_guard(old):
            guest.finish(old, {"job": old, "safeToRelease": True, "exitCode": 130})
        with guest.host_guard("b" * 32):
            pass

    def test_unavailable_docker_is_not_an_empty_inventory(self):
        with patch.object(guest.subprocess, "run", return_value=SimpleNamespace(returncode=1)) as invoke:
            with self.assertRaisesRegex(ValueError, "Docker must be running"):
                guest.docker_baseline()
        self.assertEqual(1, invoke.call_count)

    def test_expiry_never_kills_a_process_that_reused_the_streamer_pid(self):
        job = "a" * 32
        receipt = {"job": job, "safeToRelease": True, "exitCode": 0}
        (self.root / (job + ".info.json")).write_text(json.dumps({"streamer": {"pid": 12345, "start": "10"}}))
        (self.root / (job + ".result.json")).write_text(json.dumps(receipt))
        with patch.object(guest, "process_start", return_value="20"), \
                patch.object(guest.os, "kill", side_effect=AssertionError("PID belongs to a later process")), \
                patch.object(guest.subprocess, "run", return_value=SimpleNamespace(returncode=0)):
            self.assertEqual(receipt, guest.cancel(job, interrupt_streamer=True))

    def test_recovery_after_release_collects_without_cancelling_current_work(self):
        old = "a" * 32
        run_dir = self.root / old
        run_dir.mkdir()
        (run_dir / "job.json").write_text(json.dumps({"host": {"name": "vm-1", "host": "192.0.2.1"}}))
        config = self.root / "config.json"
        config.write_text(json.dumps({"runs": str(self.root)}))
        operations = []

        class Client:
            def __init__(self, profile):
                pass

            def control(self, operation, job=""):
                operations.append((operation, job))
                return {"hosts": [{"name": "vm-1", "job": {"id": "b" * 32}}]}

            def guest(self, host, operation, job):
                self_outer.assertEqual("result", operation)
                return {"job": old, "safeToRelease": True, "exitCode": 0}

            def artifacts(self, host, job, directory):
                operations.append(("artifacts", job))

        self_outer = self
        with patch.object(remote, "PoolClient", Client), \
                patch.object(sys, "argv", ["remote", "--config", str(config), "recover", old]):
            self.assertEqual(0, remote.main())
        self.assertIn(("artifacts", old), operations)
        self.assertEqual(0, json.loads((run_dir / "result.json").read_text())["exitCode"])

    def test_artifact_retry_keeps_the_guest_log_without_extracting_remote_paths(self):
        import io
        import tarfile
        key = self.root / "key"; key.touch()
        (self.root / "known_hosts").touch()
        job = "a" * 32
        payload = io.BytesIO()
        with tarfile.open(fileobj=payload, mode="w:gz") as archive:
            for name, data in [(job + ".log", b"complete remote stdout\n"), ("../escape", b"never extracted")]:
                entry = tarfile.TarInfo(name); entry.size = len(data)
                archive.addfile(entry, io.BytesIO(data))
        client = remote.PoolClient({"key": str(key)})

        def download(arguments, stdout):
            stdout.write(payload.getvalue())
            return SimpleNamespace(returncode=0)

        with patch.object(remote.subprocess, "run", side_effect=download) as invoke, \
                patch.object(client, "guest", return_value={"cleaned": True}):
            client.artifacts({"host": "192.0.2.1"}, job, self.root)
            client.artifacts({"host": "192.0.2.1"}, job, self.root)
        self.assertEqual(1, invoke.call_count)
        self.assertEqual(b"complete remote stdout\n", (self.root / "guest.log").read_bytes())
        self.assertFalse((self.root.parent / "escape").exists())


class PoolTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.state = Path(self.temporary.name) / "pool.json"
        self.state.write_text(json.dumps({"hosts": [
            {"name": f"vm-{index}", "host": f"192.0.2.{index}", "job": None}
            for index in range(1, 5)]}))

    def request(self, operation, job=""):
        result = subprocess.run([sys.executable, str(HERE / "pool.py"), "--state", str(self.state)],
                                input=json.dumps({"operation": operation, "job": job}),
                                text=True, capture_output=True)
        return json.loads(result.stdout)

    def test_concurrent_agents_get_at_most_four_distinct_slots(self):
        with ThreadPoolExecutor(max_workers=12) as executor:
            replies = list(executor.map(lambda index: self.request("acquire", f"{index:032x}"), range(12)))
        assigned = [reply["host"] for reply in replies if reply["host"]]
        self.assertEqual(4, len(assigned))
        self.assertEqual(4, len({host["name"] for host in assigned}))

    def test_retry_unknown_and_wrong_release_do_not_reassign_work(self):
        job = "a" * 32
        first = self.request("acquire", job)["host"]
        self.assertEqual(first, self.request("acquire", job)["host"])
        self.assertFalse(self.request("release", "b" * 32)["released"])
        self.request("unknown", job)
        self.assertIn("error", self.request("acquire", job))
        self.assertTrue(self.request("release", job)["released"])
        self.assertEqual(first["name"], self.request("acquire", "c" * 32)["host"]["name"])

    def test_old_reservations_never_expire_on_wall_clock(self):
        for index in range(4):
            self.request("acquire", f"{index:032x}")
        state = json.loads(self.state.read_text())
        for host in state["hosts"]:
            host["job"]["started"] = 0
        self.state.write_text(json.dumps(state))
        self.assertIsNone(self.request("acquire", "f" * 32)["host"])

    def test_maintenance_and_failed_preflight_are_skipped_by_every_worker(self):
        state = json.loads(self.state.read_text())
        state["hosts"][0]["maintenance"] = "inspection"
        self.state.write_text(json.dumps(state))
        job = "a" * 32
        self.assertEqual("vm-2", self.request("acquire", job)["host"]["name"])
        self.assertTrue(self.request("unavailable", job)["quarantined"])
        self.assertEqual("vm-3", self.request("acquire", job)["host"]["name"])
        self.assertEqual("vm-4", self.request("acquire", "b" * 32)["host"]["name"])

    def test_preflight_failure_cannot_free_an_unknown_job(self):
        job = "a" * 32
        self.request("acquire", job)
        self.request("unknown", job)
        self.assertIn("error", self.request("unavailable", job))
        self.assertEqual(job, self.request("status")["hosts"][0]["job"]["id"])


class ReachabilityTests(unittest.TestCase):
    def test_unreachable_slot_is_cooled_down_before_trying_the_next(self):
        client = object.__new__(remote.PoolClient)
        first, second = {"name": "broken"}, {"name": "healthy"}
        calls = []

        def control(operation, job):
            calls.append(operation)
            return {"host": first if len(calls) == 1 else second}

        with patch.object(client, "control", side_effect=control), \
                patch.object(client, "ssh", side_effect=[remote.Failure("timeout"), ""]) as probe:
            self.assertEqual(second, client.acquire("a" * 32))
        self.assertEqual(["acquire", "unavailable", "acquire"], calls)
        self.assertEqual(2, probe.call_count)


class LeaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.state = Path(self.temporary.name) / "pool.json"
        self.state.write_text(json.dumps({"hosts": [{"name": "vm-1", "host": "192.0.2.1", "job": None}]}))
        self.job = "a" * 32
        self.profile = {"key": "/private/key", "knownHosts": "/private/known_hosts", "hosts": {"vm-1": "192.0.2.1"}}
        self.call = lambda op: pool.request(self.state, {"operation": op, "job": self.job})

    def expire(self):
        with patch.object(pool.time, "time", return_value=100):
            pool.request(self.state, {"operation": "acquire", "job": self.job, "lease": True})

    def test_expiry_fences_renewal_without_releasing_unproven_work(self):
        self.expire()
        with patch.object(pool.time, "time", return_value=221):
            self.assertEqual(1, len(self.call("expired")["hosts"]))
            with self.assertRaisesRegex(ValueError, "no longer renewable"):
                self.call("heartbeat")
            self.assertIsNone(pool.request(self.state, {"operation": "acquire", "job": "b" * 32})["host"])

    def test_heartbeat_extends_a_live_lease(self):
        self.expire()
        with patch.object(pool.time, "time", return_value=200):
            self.assertTrue(self.call("heartbeat")["renewed"])
        with patch.object(pool.time, "time", return_value=221):
            self.assertEqual([], self.call("expired")["hosts"])

    def test_reaper_retries_uncertain_cleanup_then_frees_only_a_matching_safe_receipt(self):
        self.expire()
        unsafe = json.dumps({"job": self.job, "safeToRelease": False})
        wrong = json.dumps({"job": "b" * 32, "safeToRelease": True})
        safe = json.dumps({"job": self.job, "safeToRelease": True})
        for receipt in (unsafe, wrong):
            with patch.object(reaper.subprocess, "run", return_value=SimpleNamespace(stdout=receipt)):
                reaper.sweep(self.state, self.profile)
            self.assertEqual(self.job, self.call("status")["hosts"][0]["job"]["id"])
        with patch.object(reaper.subprocess, "run", return_value=SimpleNamespace(stdout=safe)):
            reaper.sweep(self.state, self.profile)
        self.assertIsNone(self.call("status")["hosts"][0]["job"])


@unittest.skipUnless(CRABBOX, "set TEST_CRABBOX to the complete pinned 0.83.0 release")
class TransferTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.repo, self.frozen = root / "repo", root / "snapshot"
        self.repo.mkdir(); self.frozen.mkdir()
        self.config = root / "crabbox.json"
        self.config.write_text(json.dumps(remote.source_policy(root / "unused-key")))
        subprocess.run(["git", "init", "--quiet", self.repo], check=True)
        self.write(".gitignore", "node_modules/\ntarget/\n")
        self.write(".github/workflows/test.yml", "toolchain: 1.98.0\n")
        self.write("packages/browseros-agent/src/target/mod.rs", "// valid Rust source\n")
        self.write("packages/browseros-agent/src/credentials.rs", "// valid Rust source\n")
        subprocess.run(["git", "-C", self.repo, "add", "--force", "."], check=True)
        subprocess.run(["git", "-C", self.repo, "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                        "commit", "--quiet", "-m", "fixture"], check=True)
        self.write("packages/browseros-agent/src/credentials.rs", "// uncommitted working bytes\n")
        for path in (".env", ".llm/private.txt", ".scratch/private.txt", "secret.key", "credentials.json",
                     ".npmrc", ".netrc", ".ssh/config", ".docker/config.json",
                     "node_modules/dependency.js", "target/debug/output"):
            self.write("packages/browseros-agent/" + path, "excluded\n")

    def write(self, relative, text):
        path = self.repo / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def test_native_git_and_frozen_index_send_identical_approved_working_bytes(self):
        metadata = remote.freeze(CRABBOX, self.repo, self.frozen, self.config)
        self.assertTrue(metadata["dirty"])
        self.assertEqual(3, metadata["files"])
        self.assertEqual("// uncommitted working bytes\n",
                         (self.frozen / "packages/browseros-agent/src/credentials.rs").read_text())
        self.assertFalse((self.frozen / ".git").exists())
        remote.index_snapshot(self.frozen)
        files = remote.manifest(CRABBOX, self.frozen, self.config)
        self.assertEqual(metadata["files"], len(files))

    def test_selected_symlink_is_rejected(self):
        path = self.repo / "packages/browseros-agent/leak"
        path.symlink_to(self.repo / ".git/config")
        with self.assertRaises(remote.Failure):
            remote.freeze(CRABBOX, self.repo, self.frozen, self.config)

    def track_link(self, name, target):
        path = self.repo / "packages/browseros-agent" / name
        path.symlink_to(target)
        subprocess.run(["git", "-C", str(self.repo), "add", "--force", "--", str(path)], check=True)
        return path

    def test_tracked_relative_link_materializes_only_selected_working_bytes(self):
        self.write("packages/browseros-agent/CLAUDE.md", "selected dirty instructions\n")
        self.track_link("AGENTS.md", "CLAUDE.md")
        metadata = remote.freeze(CRABBOX, self.repo, self.frozen, self.config)
        copied = self.frozen / "packages/browseros-agent/AGENTS.md"
        self.assertFalse(copied.is_symlink())
        self.assertEqual("selected dirty instructions\n", copied.read_text())
        remote.index_snapshot(self.frozen)
        files = remote.manifest(CRABBOX, self.frozen, self.config)
        self.assertEqual(metadata["files"], len(files))
        self.assertEqual(metadata["bytes"], sum(row["bytes"] for row in files))

    def test_tracked_relative_link_chain_is_materialized(self):
        self.write("packages/browseros-agent/CLAUDE.md", "approved\n")
        self.track_link("instructions.md", "CLAUDE.md")
        self.track_link("AGENTS.md", "instructions.md")
        remote.freeze(CRABBOX, self.repo, self.frozen, self.config)
        self.assertEqual("approved\n", (self.frozen / "packages/browseros-agent/AGENTS.md").read_text())

    def test_tracked_absolute_escaping_and_excluded_targets_are_refused(self):
        for target in (str(self.repo / "packages/browseros-agent/src/credentials.rs"),
                       "../../.git/config", "../../unselected.txt", ".env"):
            with self.subTest(target=target):
                link = self.track_link("AGENTS.md", target)
                try:
                    with self.assertRaises(remote.Failure):
                        remote.freeze(CRABBOX, self.repo, self.frozen, self.config)
                finally:
                    link.unlink()

    def test_untracked_internal_link_is_refused(self):
        (self.repo / "packages/browseros-agent/AGENTS.md").symlink_to("src/credentials.rs")
        with self.assertRaises(remote.Failure):
            remote.freeze(CRABBOX, self.repo, self.frozen, self.config)

    def test_retargeting_a_link_during_freeze_is_refused(self):
        self.write("packages/browseros-agent/CLAUDE.md", "approved instructions\n")
        link = self.track_link("AGENTS.md", "CLAUDE.md")
        original = Path.read_bytes

        def retarget(path):
            data = original(path)
            if path == self.repo / "packages/browseros-agent/CLAUDE.md":
                link.unlink(); link.symlink_to("src/credentials.rs")
            return data

        with patch.object(Path, "read_bytes", retarget):
            with self.assertRaisesRegex(remote.Failure, "source changed"):
                remote.freeze(CRABBOX, self.repo, self.frozen, self.config)

    def test_parent_traversal_cannot_hide_a_directory_symlink_or_excluded_hop(self):
        self.write("packages/browseros-agent/CLAUDE.md", "approved\n")
        agent = self.repo / "packages/browseros-agent"
        shutil.rmtree(agent / ".scratch")
        (agent / ".scratch").symlink_to(self.repo / ".git", target_is_directory=True)
        self.track_link("AGENTS.md", ".scratch/../CLAUDE.md")
        with self.assertRaisesRegex(remote.Failure, "excluded path or directory link"):
            remote.freeze(CRABBOX, self.repo, self.frozen, self.config)

    def test_relative_parent_link_within_selected_directories_is_allowed(self):
        self.write("packages/browseros-agent/CLAUDE.md", "approved\n")
        self.track_link("src/AGENTS.md", "../CLAUDE.md")
        remote.freeze(CRABBOX, self.repo, self.frozen, self.config)
        self.assertEqual("approved\n", (self.frozen / "packages/browseros-agent/src/AGENTS.md").read_text())

    def test_legacy_image_manifest_adds_only_its_five_declared_inputs(self):
        for path in remote.LEGACY_SUPERVISOR:
            self.write(path, "legacy supervisor source\n")
        self.write("packages/browseros/docker/neo/other.txt", "unselected production input\n")
        self.write("packages/browseros/docker/neo/.env", "never transfer\n")
        self.config.write_text(json.dumps(remote.source_policy(self.repo / "unused-key",
                                                             remote.INCLUDE + remote.LEGACY_SUPERVISOR)))
        metadata = remote.freeze(CRABBOX, self.repo, self.frozen, self.config)
        self.assertTrue(metadata["dirty"])
        self.assertEqual(8, metadata["files"])
        selected = {row["path"] for row in json.loads((self.frozen.parent / "source-files.json").read_text())}
        self.assertTrue(set(remote.LEGACY_SUPERVISOR).issubset(selected))
        self.assertFalse(any(path.endswith(("other.txt", ".env")) for path in selected))

    def test_private_key_in_ordinary_source_name_is_rejected(self):
        self.write("packages/browseros-agent/src/ordinary.txt", "-----BEGIN " + "OPENSSH PRIVATE KEY-----\nopaque\n")
        with self.assertRaises(remote.Failure):
            remote.freeze(CRABBOX, self.repo, self.frozen, self.config)

    def test_edits_during_snapshot_are_rejected(self):
        original = Path.read_bytes

        def edit_after_read(path):
            data = original(path)
            if path == self.repo / "packages/browseros-agent/src/credentials.rs":
                path.write_text("// edited while snapshotting\n")
            return data

        with patch.object(Path, "read_bytes", edit_after_read):
            with self.assertRaisesRegex(remote.Failure, "source changed"):
                remote.freeze(CRABBOX, self.repo, self.frozen, self.config)


class MetadataTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.agent = self.root / "packages/browseros-agent"
        self.docker = self.agent / "crates/neo-sdk/neo-server/docker"
        self.docker.mkdir(parents=True)
        for name in ("build.sh", "browser.lock"):
            shutil.copyfile(AGENT / "crates/neo-sdk/neo-server/docker" / name, self.docker / name)
        self.environment = {key: value for key, value in os.environ.items() if not key.startswith("NEO_SDK_SOURCE_")}
        self.environment.update(NEO_SDK_SOURCE_ROOT=str(self.root), NEO_SDK_SOURCE_REVISION="a" * 40,
                                NEO_SDK_SOURCE_DIRTY="true")

    def test_image_builder_accepts_metadata_without_calling_git_and_keeps_digest_check(self):
        binary = b"binary browseros-claw-sdk-source=" + b"b" * 64
        import hashlib
        provenance = {"revision": "a" * 40, "dirty": True, "source_digest": "b" * 64,
                      "server": {"path": "/usr/bin/sdk", "sha256": hashlib.sha256(binary).hexdigest()}}
        bindir = self.root / "bin"; bindir.mkdir()
        (bindir / "git").write_text("#!/bin/sh\nexit 99\n")
        (bindir / "git").chmod(0o755)
        # Emulate only Docker's transport. The real build.sh still verifies embedded source
        # digest and binary hash, and is the code that decides whether tagging is allowed.
        fake = bindir / "docker"
        fake.write_text(f'''#!{sys.executable}
import io,json,os,pathlib,sys,tarfile
args=sys.argv[1:]
if args[:2]==['buildx','build']:
 pathlib.Path(args[args.index('--iidfile')+1]).write_text('sha256:fixture')
elif args[0]=='create': print('fixture-container')
elif args[0]=='cp':
 data=json.dumps({provenance!r}).encode() if 'provenance.json' in args[1] else {binary!r}
 if os.environ.get('TAMPER') and 'provenance.json' not in args[1]: data+=b'tampered'
 with tarfile.open(fileobj=sys.stdout.buffer,mode='w|') as archive:
  entry=tarfile.TarInfo('payload'); entry.size=len(data); archive.addfile(entry,io.BytesIO(data))
elif args[0]=='tag': pathlib.Path({str(self.root / "tagged")!r}).touch()
elif args[0]!='rm': sys.exit(98)
''')
        fake.chmod(0o755)
        self.environment["PATH"] = str(bindir) + os.pathsep + self.environment["PATH"]
        command = ["bash", str(self.docker / "build.sh"), "--platform", "linux/amd64", "--tag", "fixture"]
        accepted = subprocess.run(command, env=self.environment, capture_output=True, text=True)
        self.assertEqual(0, accepted.returncode, accepted.stderr)
        self.assertTrue((self.root / "tagged").exists())
        (self.root / "tagged").unlink()
        rejected = subprocess.run(command, env={**self.environment, "TAMPER": "1"}, capture_output=True, text=True)
        self.assertNotEqual(0, rejected.returncode)
        self.assertIn("not the binary", rejected.stderr)
        self.assertFalse((self.root / "tagged").exists())
        installed = self.root / "installed-build-image.sh"
        shutil.copyfile(self.docker / "build.sh", installed)
        (self.docker / "build.sh").write_text("#!/bin/sh\nexit 99\n")
        external = subprocess.run(["bash", str(installed), *command[2:]], env=self.environment,
                                  capture_output=True, text=True)
        self.assertEqual(0, external.returncode, external.stderr)

    def test_partial_or_invalid_image_metadata_fails_before_build(self):
        for field, value in [("NEO_SDK_SOURCE_REVISION", "short"), ("NEO_SDK_SOURCE_DIRTY", "yes"),
                             ("NEO_SDK_SOURCE_ROOT", "/"), ("NEO_SDK_SOURCE_DIRTY", "")]:
            result = subprocess.run(["bash", str(self.docker / "build.sh")],
                                    env={**self.environment, field: value}, capture_output=True, text=True)
            self.assertNotEqual(0, result.returncode)

    def test_spec_metadata_works_without_git_and_rejects_partial_fields(self):
        module = AGENT / "crates/neo-sdk/neo-playwright-tests/node/source-metadata.mjs"
        command = [shutil.which("node"), "--input-type=module", "-e",
                   f"import {{sourceMetadata}} from {json.dumps(module.as_uri())}; "
                   "console.log(JSON.stringify(sourceMetadata(process.argv[1])))", str(self.agent)]
        environment = {**self.environment, "PATH": "/nonexistent"}
        result = subprocess.run(command, env=environment, capture_output=True, text=True)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual({"revision": "a" * 40, "dirty": True}, json.loads(result.stdout))
        environment.pop("NEO_SDK_SOURCE_DIRTY")
        self.assertNotEqual(0, subprocess.run(command, env=environment, capture_output=True).returncode)

    def test_legacy_spec_preload_only_supplies_the_uploaded_checkouts_head(self):
        public = self.root / "public-checkout"
        public.mkdir(); (public / "fixture").write_text("public corpus fixture")
        subprocess.run(["git", "init", "--quiet", str(public)], check=True)
        subprocess.run(["git", "-C", str(public), "add", "."], check=True)
        subprocess.run(["git", "-C", str(public), "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                        "commit", "--quiet", "-m", "fixture"], check=True)
        expected = subprocess.check_output(["git", "-C", str(public), "rev-parse", "HEAD"], text=True).strip()
        preload = HERE / "source-metadata-compat.cjs"
        code = '''import {execFileSync} from 'node:child_process';
const git = cwd => execFileSync('git', ['rev-parse','HEAD'], {cwd,encoding:'utf8'}).trim();
console.log(JSON.stringify([git(process.argv[1]),git(process.argv[2]),execFileSync('pwd',{encoding:'utf8'}).trim()]));'''
        result = subprocess.run([shutil.which("node"), "--require", str(preload), "--input-type=module", "-e", code,
                                 str(self.agent), str(public)], env=self.environment, capture_output=True, text=True)
        self.assertEqual(0, result.returncode, result.stderr)
        actual = json.loads(result.stdout)
        self.assertEqual(["a" * 40, expected], actual[:2])
        self.assertEqual(str(Path.cwd().resolve()), actual[2])


if __name__ == "__main__":
    unittest.main()
