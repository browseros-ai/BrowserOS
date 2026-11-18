"""The GitHub runner path: pool host lists, the dispatcher, the job hook and the ref check.

These run without GitHub, SSH, systemd or a pool machine. GitHub and the dispatcher's
SSH transport are fakes; the pool is a real pool.py state file in a temporary directory.
"""
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import pool
import runners
sys.path.pop(0)
guest_spec = importlib.util.spec_from_file_location("runner_guest", HERE / "guest.py")
guest = importlib.util.module_from_spec(guest_spec)
guest_spec.loader.exec_module(guest)

REPOSITORY = "org/repo"
WORKFLOW = ".github/workflows/pool-run.yml"
ALLOWED = f"{REPOSITORY}/{WORKFLOW}@refs/heads/main"


def submission(worker: str, kind: str = "run") -> dict:
    return {"worker": worker, "label": "worker-" + worker[:4], "kind": kind,
            "clientPid": 1, "clientMachine": "fixture", "protocol": 2}


class PoolFixture(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name) / "pool.json"
        self.state.write_text(json.dumps({"hosts": [
            {"name": f"vm-{index}", "host": f"192.0.2.{index}", "job": None} for index in range(1, 5)]}))

    def acquire(self, job: str, worker: str, kind: str = "run", hosts=None):
        message = {"operation": "acquire", "job": job, "submission": submission(worker, kind)}
        if hosts is not None:
            message["hosts"] = hosts
        return pool.request(self.state, message)["host"]

    def occupy(self, *names: str) -> None:
        state = json.loads(self.state.read_text())
        for host in state["hosts"]:
            if host["name"] in names:
                host["job"] = {"id": host["name"][-1] * 32, "state": "reserved", "started": time.time()}
        self.state.write_text(json.dumps(state))

    def holder(self, name: str):
        hosts = pool.request(self.state, {"operation": "status"})["hosts"]
        return next(host for host in hosts if host["name"] == name).get("job")


class HostListTests(PoolFixture):
    def test_a_restricted_waiter_takes_only_its_hosts(self):
        self.occupy("vm-4")
        self.assertIsNone(self.acquire("a" * 32, "1" * 32, hosts=["vm-4"]))
        waiting = pool.request(self.state, {"operation": "queue"})["waiting"]
        self.assertEqual([["vm-4"]], [row["hosts"] for row in waiting])
        pool.request(self.state, {"operation": "release", "job": "4" * 32})
        self.assertEqual("vm-4", self.acquire("a" * 32, "1" * 32, hosts=["vm-4"])["name"])

    def test_a_restricted_waiter_never_blocks_the_developers_behind_it(self):
        self.occupy("vm-1", "vm-2", "vm-3", "vm-4")
        self.assertIsNone(self.acquire("a" * 32, "1" * 32, hosts=["vm-4"]))
        self.assertIsNone(self.acquire("b" * 32, "2" * 32))
        pool.request(self.state, {"operation": "release", "job": "1" * 32})
        self.assertIsNone(self.acquire("a" * 32, "1" * 32, hosts=["vm-4"]))
        self.assertEqual("vm-1", self.acquire("b" * 32, "2" * 32)["name"])

    def test_a_higher_priority_developer_still_goes_first_on_a_shared_host(self):
        self.occupy("vm-1", "vm-2", "vm-3", "vm-4")
        self.assertIsNone(self.acquire("b" * 32, "2" * 32, kind="gate"))
        self.assertIsNone(self.acquire("a" * 32, "1" * 32, hosts=["vm-4"]))
        pool.request(self.state, {"operation": "release", "job": "4" * 32})
        self.assertIsNone(self.acquire("a" * 32, "1" * 32, hosts=["vm-4"]))
        self.assertEqual("vm-4", self.acquire("b" * 32, "2" * 32, kind="gate")["name"])

    def test_requests_without_a_host_list_keep_todays_rows_and_order(self):
        self.occupy("vm-1", "vm-2", "vm-3", "vm-4")
        self.assertIsNone(self.acquire("b" * 32, "2" * 32))
        row = pool.request(self.state, {"operation": "queue"})["waiting"][0]
        self.assertNotIn("hosts", row)
        pool.request(self.state, {"operation": "release", "job": "3" * 32})
        pool.request(self.state, {"operation": "release", "job": "2" * 32})
        self.assertEqual("vm-2", self.acquire("b" * 32, "2" * 32)["name"])

    def test_malformed_host_lists_are_refused(self):
        for hosts in ("vm-1", [], [1], ["x" * 65]):
            with self.assertRaises(ValueError):
                self.acquire("a" * 32, "1" * 32, hosts=hosts)


def run_row(identifier, event="workflow_dispatch", branch="main", head=REPOSITORY, path=WORKFLOW):
    return {"id": identifier, "event": event, "head_branch": branch, "path": path,
            "repository": {"full_name": REPOSITORY}, "head_repository": {"full_name": head}}


def job_row(status="queued", labels=("self-hosted", "browseros-pool")):
    return {"status": status, "labels": list(labels)}


POLICY = runners.Policy(REPOSITORY, WORKFLOW, ("main",), "browseros-pool")


class PolicyTests(unittest.TestCase):
    def test_only_queued_pool_jobs_of_dispatched_main_runs_count(self):
        runs = [run_row(1), run_row(2, event="pull_request", head="fork/repo"),
                run_row(3, event="pull_request_target"), run_row(4, branch="feature"),
                run_row(5, path=".github/workflows/other.yml"), run_row(6, head="fork/repo")]
        jobs = {1: [job_row(), job_row("completed", labels=("ubuntu-24.04",)), job_row("in_progress")]}
        counted = runners.pool_jobs_waiting(runs, lambda run: jobs.get(run, [job_row()]), POLICY)
        self.assertEqual(1, counted)

    def test_a_pool_job_counts_only_after_the_request_check_finished(self):
        checking = {1: [job_row("in_progress", labels=("ubuntu-24.04",)), job_row()]}
        self.assertEqual(0, runners.pool_jobs_waiting([run_row(1)], checking.get, POLICY))

    def test_only_a_group_restricted_to_this_repository_and_workflow_passes(self):
        good = {"visibility": "selected", "restricted_to_workflows": True, "selected_workflows": [ALLOWED]}
        self.assertIsNone(runners.runner_group_problem(good, [REPOSITORY], POLICY))
        problems = [({**good, "visibility": "all"}, [REPOSITORY]), (good, [REPOSITORY, "org/other"]),
                    ({**good, "restricted_to_workflows": False}, [REPOSITORY]),
                    ({**good, "selected_workflows": [ALLOWED, f"{REPOSITORY}/{WORKFLOW}@refs/heads/dev"]}, [REPOSITORY]),
                    ({**good, "selected_workflows": []}, [REPOSITORY])]
        for group, repositories in problems:
            self.assertIsNotNone(runners.runner_group_problem(group, repositories, POLICY), group)

    def test_runners_receive_the_allowed_workflow_refs(self):
        self.assertEqual([ALLOWED], POLICY.workflow_refs())

    def test_new_runners_cover_unmet_demand_within_the_pool_cap(self):
        cases = {(3, 0, 0): 2, (1, 1, 1): 0, (2, 0, 2): 0, (2, 1, 1): 1, (0, 0, 0): 0, (1, 0, 1): 1}
        for (demand, supply, held), expected in cases.items():
            self.assertEqual(expected, runners.runners_to_start(demand, supply, held), (demand, supply, held))


class FakeProcess:
    def __init__(self):
        self.done = threading.Event()
        self.returncode = None

    def poll(self):
        if self.done.is_set():
            self.returncode = 0
        return self.returncode


class FakeGitHub:
    def __init__(self, queued: int = 1):
        self.calls = []
        self.runners = {}
        self.queued = queued
        self.lock = threading.Lock()
        self.group = {"visibility": "selected", "restricted_to_workflows": True, "selected_workflows": [ALLOWED]}
        self.status_errors = 0

    def call(self, method, path, body=None):
        with self.lock:
            self.calls.append((method, path))
            if path.endswith("/generate-jitconfig"):
                self.minted = body
                runner = 100 + len(self.calls)
                self.runners[runner] = {"busy": False}
                return {"runner": {"id": runner}, "encoded_jit_config": "one-job-registration"}
            if "/workflows/" in path:
                return {"workflow_runs": [run_row(7)] if "status=queued" in path else []}
            if path.endswith("per_page=100") and "/runs/" in path:
                return {"jobs": [job_row() for _ in range(self.queued)]}
            if path.endswith("/runner-groups/3"):
                return self.group
            if "/runner-groups/3/repositories" in path:
                return {"repositories": [{"full_name": REPOSITORY}]}
            if "/runner-groups/" in path:
                return {"runners": []}
            if method == "GET" and self.status_errors:
                self.status_errors -= 1
                raise RuntimeError("GitHub GET: HTTP 502")
            runner = int(path.rsplit("/", 1)[1])
            if runner not in self.runners:
                raise runners.NotFound(path)
            if method == "DELETE":
                if self.runners[runner]["busy"]:
                    raise runners.Conflict(path)
                del self.runners[runner]
                return {}
            return self.runners[runner]


class FakeTransport:
    def __init__(self, github: FakeGitHub, takes_job: bool):
        self.github, self.takes_job = github, takes_job
        self.calls, self.started = [], threading.Event()
        self.process = FakeProcess()
        self.request = None

    def reachable(self, host):
        return True

    def start(self, host, job, request, errors):
        self.request = request
        self.started.set()
        if self.takes_job:
            runner = next(iter(self.github.runners))
            self.github.runners[runner]["busy"] = True
        return self.process

    def call(self, host, command):
        operation, job = command.split()
        self.calls.append(operation)
        if operation == "cancel":
            self.process.done.set()
        if operation in {"result", "cancel"}:
            return {"job": job, "safeToRelease": True, "exitCode": 0}
        return {"cleaned": True}


class DispatcherTests(PoolFixture):
    def setUp(self):
        super().setUp()
        for name, value in {"POLL_SECONDS": 0, "ACQUIRE_SECONDS": 0.01, "WATCH_SECONDS": 0.01}.items():
            patcher = patch.object(runners, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        self.settings = {"org": "org", "repository": REPOSITORY, "workflow": WORKFLOW, "branches": ["main"],
                         "label": "browseros-pool", "runnerGroupId": 3, "hosts": ["vm-4"]}

    def dispatcher(self, takes_job: bool):
        github = FakeGitHub()
        transport = FakeTransport(github, takes_job)
        return runners.Dispatcher(self.settings, github, transport, self.state), github, transport

    def test_a_runner_job_holds_its_slot_like_any_job_and_releases_it(self):
        dispatcher, github, transport = self.dispatcher(takes_job=True)
        slot = runners.Slot(dispatcher)
        slot.start()
        self.assertTrue(transport.started.wait(5))
        held = self.holder("vm-4")
        self.assertEqual((slot.job, "github-actions", "run"), (held["id"], held["label"], held["kind"]))
        # A developer's job submitted meanwhile never lands on the runner's machine.
        self.assertEqual("vm-1", self.acquire("d" * 32, "9" * 32)["name"])
        self.assertEqual({"jitconfig": "one-job-registration", "repository": REPOSITORY,
                          "workflows": [ALLOWED]}, transport.request)
        # pool-run.yml asks for [self-hosted, browseros-pool]; a JIT runner gets no default labels.
        self.assertEqual(["self-hosted", "browseros-pool"], github.minted["labels"])
        time.sleep(0.1)
        self.assertEqual("busy", slot.state)
        transport.process.done.set()
        slot.join(5)
        self.assertEqual(["result", "cleanup"], transport.calls)
        self.assertIsNone(self.holder("vm-4"))

    def test_an_unused_runner_is_deleted_before_its_job_is_cancelled(self):
        dispatcher, github, transport = self.dispatcher(takes_job=False)
        with patch.object(runners, "IDLE_SECONDS", 0.05):
            slot = runners.Slot(dispatcher)
            slot.start()
            slot.join(5)
        deleted = next(index for index, call in enumerate(github.calls) if call[0] == "DELETE")
        self.assertEqual({}, github.runners)
        self.assertEqual(["cancel", "result", "cleanup"], transport.calls)
        self.assertGreater(deleted, 0)
        self.assertIsNone(self.holder("vm-4"))

    def test_github_errors_while_a_job_runs_never_end_its_supervision(self):
        dispatcher, github, transport = self.dispatcher(takes_job=False)
        github.status_errors = 3
        with patch.object(runners, "IDLE_SECONDS", 60):
            slot = runners.Slot(dispatcher)
            slot.start()
            time.sleep(0.2)
            self.assertTrue(slot.is_alive())
            self.assertEqual("idle", slot.state)
            transport.process.done.set()
            slot.join(5)
        self.assertEqual(["result", "cleanup"], transport.calls)
        self.assertIsNone(self.holder("vm-4"))

    def test_no_runner_is_minted_unless_the_group_is_restricted(self):
        dispatcher, github, transport = self.dispatcher(takes_job=True)
        github.group = {**github.group, "restricted_to_workflows": False}
        slot = runners.Slot(dispatcher)
        slot.start()
        slot.join(5)
        self.assertFalse(any(path.endswith("generate-jitconfig") for _, path in github.calls))
        self.assertIsNone(self.holder("vm-4"))
        with patch.object(runners.Slot, "start", lambda slot: None):
            dispatcher.group = (0.0, None)
            dispatcher.tick()
        self.assertEqual([], [slot for slot in dispatcher.slots if slot.state != "done"])

    def test_a_coordinator_that_ignores_host_lists_gets_its_host_back_and_halts_the_dispatcher(self):
        dispatcher, github, transport = self.dispatcher(takes_job=True)
        legacy = lambda state, message: pool.request(state, {key: value for key, value in message.items() if key != "hosts"})
        with patch.object(runners, "pool_request", legacy):
            slot = runners.Slot(dispatcher)
            slot.start()
            slot.join(5)
        self.assertIn("vm-1", dispatcher.halted)
        self.assertIsNone(self.holder("vm-1"))
        self.assertFalse(transport.started.is_set())

    def test_a_waiting_slot_withdraws_when_its_job_disappears(self):
        self.occupy("vm-4")
        dispatcher, github, transport = self.dispatcher(takes_job=True)
        slot = runners.Slot(dispatcher)
        slot.start()
        time.sleep(0.1)
        self.assertEqual(1, len(pool.request(self.state, {"operation": "queue"})["waiting"]))
        slot.wanted = False
        slot.join(5)
        self.assertEqual([], pool.request(self.state, {"operation": "queue"})["waiting"])
        self.assertFalse(any(path.endswith("generate-jitconfig") for _, path in github.calls))

    def test_ticks_open_at_most_two_slots_and_stop_surplus_waiters(self):
        dispatcher, github, transport = self.dispatcher(takes_job=True)
        github.queued = 3
        with patch.object(runners.Slot, "start", lambda slot: None):
            dispatcher.tick()
            dispatcher.tick()
            self.assertEqual(2, len(dispatcher.slots))
            github.queued = 1
            dispatcher.tick()
        self.assertEqual([True, False], [slot.wanted for slot in dispatcher.slots])

    def test_no_runner_is_minted_for_a_fork_pull_request(self):
        dispatcher, github, transport = self.dispatcher(takes_job=True)
        answer = github.call
        github.call = lambda method, path, body=None: (
            {"workflow_runs": [run_row(8, event="pull_request", head="fork/repo")]} if "/workflows/" in path
            else answer(method, path, body))
        with patch.object(runners.Slot, "start", lambda slot: None):
            dispatcher.tick()
            self.assertEqual([], dispatcher.slots)
            github.call = answer
            dispatcher.tick()
        self.assertEqual(1, len(dispatcher.slots))  # The same dispatcher does serve main's run.


class HookTests(unittest.TestCase):
    def hook(self, **values):
        environment = {"PATH": os.environ["PATH"], "POOL_RUNNER_REPOSITORY": REPOSITORY,
                       "POOL_RUNNER_WORKFLOWS": ALLOWED, "GITHUB_REPOSITORY": REPOSITORY,
                       "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_WORKFLOW_REF": ALLOWED}
        environment.update(values)
        environment = {key: value for key, value in environment.items() if value is not None}
        return subprocess.run(["bash", str(HERE / "runner-hook.sh")], env=environment,
                              capture_output=True, text=True).returncode

    def test_only_mains_dispatched_workflow_may_run_a_step(self):
        self.assertEqual(0, self.hook())
        refused = [
            {"GITHUB_EVENT_NAME": "pull_request", "GITHUB_WORKFLOW_REF": f"{REPOSITORY}/.github/workflows/x.yml@refs/pull/7/merge"},
            {"GITHUB_EVENT_NAME": "pull_request_target"},
            {"GITHUB_WORKFLOW_REF": f"{REPOSITORY}/{WORKFLOW}@refs/heads/feature"},
            {"GITHUB_WORKFLOW_REF": f"{REPOSITORY}/.github/workflows/other.yml@refs/heads/main"},
            {"GITHUB_REPOSITORY": "fork/repo"},
            {"GITHUB_WORKFLOW_REF": None},
            {"POOL_RUNNER_WORKFLOWS": None},
        ]
        for values in refused:
            self.assertEqual(1, self.hook(**values), values)


class RefTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory()
        cls.remote = Path(cls.directory.name) / "repo"
        git = ["git", "-C", str(cls.remote), "-c", "user.name=t", "-c", "user.email=t@example.com"]
        subprocess.run(["git", "init", "-q", "-b", "main", str(cls.remote)], check=True)

        def commit(message):
            subprocess.run([*git, "commit", "-q", "--allow-empty", "-m", message], check=True)
            return subprocess.run([*git, "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
        cls.main = commit("main")
        cls.feature = commit("feature")
        subprocess.run([*git, "branch", "feature"], check=True)
        subprocess.run([*git, "tag", "-a", "-m", "annotated", "v2"], check=True)
        cls.decoy = commit("decoy")
        subprocess.run([*git, "branch", "x/refs/heads/feature"], check=True)
        subprocess.run([*git, "update-ref", "refs/pull/1/head", cls.decoy], check=True)
        subprocess.run([*git, "branch", "dup"], check=True)
        subprocess.run([*git, "tag", "dup"], check=True)

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def resolve(self, ref, preset="command", command="true"):
        with tempfile.NamedTemporaryFile("r") as output:
            environment = {"PATH": os.environ["PATH"], "POOL_REF": ref, "POOL_PRESET": preset,
                           "POOL_COMMAND": command, "POOL_REMOTE": str(self.remote), "GITHUB_OUTPUT": output.name}
            result = subprocess.run(["bash", str(HERE / "pool-ref.sh")], env=environment, capture_output=True, text=True)
            return result.returncode, output.read().strip()

    def test_branches_and_tags_resolve_to_their_exact_commit(self):
        self.assertEqual((0, "sha=" + self.feature), self.resolve("feature"))
        self.assertEqual((0, "sha=" + self.feature), self.resolve("v2"))

    def test_pull_request_refs_commit_ids_and_ambiguous_or_unsafe_names_are_refused(self):
        for ref in ("refs/pull/1/head", "pull/1/head", self.decoy, "dup", "missing", "-x", "a b", ""):
            self.assertEqual(1, self.resolve(ref)[0], ref)

    def test_presets_and_commands_must_agree(self):
        self.assertEqual(1, self.resolve("feature", command="")[0])
        self.assertEqual(1, self.resolve("feature", preset="sdk-gate")[0])
        self.assertEqual(0, self.resolve("feature", preset="sdk-gate", command="")[0])
        self.assertEqual(1, self.resolve("feature", preset="other")[0])


class RunnerCommandTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        patcher = patch.object(guest, "ROOT", Path(directory.name))
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_the_dispatcher_key_reaches_only_runner_operations(self):
        for command in ("bash -c id", "runner-start", "runner-start not-a-job", "lease-cancel " + "a" * 32,
                        "probe " + "0" * 32, "result " + "a" * 32 + " extra"):
            with self.assertRaises(ValueError, msg=command):
                guest.runner_rpc(command)

    def test_a_start_request_carries_exactly_the_registration_and_policy(self):
        valid = {"jitconfig": "registration", "repository": REPOSITORY, "workflows": [ALLOWED]}
        invalid = [{**valid, "extra": 1}, {**valid, "jitconfig": ""}, {**valid, "repository": "x y/z"},
                   {**valid, "workflows": ALLOWED}, {**valid, "workflows": ["a b"]},
                   {key: value for key, value in valid.items() if key != "workflows"}]
        for request in invalid:
            with patch.object(guest.sys, "stdin", io.StringIO(json.dumps(request))), self.assertRaises(ValueError):
                guest.runner_request("a" * 32)
        with patch.object(guest.sys, "stdin", io.StringIO(json.dumps(valid))):
            self.assertEqual(valid, guest.runner_request("a" * 32))
        (guest.ROOT / ("a" * 32 + ".runner.json")).write_text("{}")
        with patch.object(guest.sys, "stdin", io.StringIO(json.dumps(valid))), self.assertRaises(ValueError):
            guest.runner_request("a" * 32)

    def test_only_a_runner_job_is_swept_and_a_failed_sweep_blocks_release(self):
        calls = []
        with patch.object(guest.subprocess, "run", lambda argv, **options: calls.append(argv)):
            guest.sweep_runner("b" * 32)
        self.assertEqual([], calls)
        (guest.ROOT / ("a" * 32 + ".runner")).write_text("{}")
        with patch.object(guest.subprocess, "run", lambda argv, **options: calls.append(argv)):
            guest.sweep_runner("a" * 32)
        self.assertEqual([["bash", guest.RUNNER_LAUNCHER, "--sweep"]], calls)

        def fails(argv, **options):
            raise subprocess.CalledProcessError(1, argv)
        with patch.object(guest.subprocess, "run", fails), self.assertRaises(subprocess.CalledProcessError):
            guest.sweep_runner("a" * 32)

    def test_a_runner_start_leaves_the_sweep_marker_and_runs_the_fixed_launcher(self):
        valid = {"jitconfig": "registration", "repository": REPOSITORY, "workflows": [ALLOWED]}
        started = []
        with patch.object(guest.sys, "stdin", io.StringIO(json.dumps(valid))), \
                patch.object(guest, "run", lambda arguments: started.append(arguments) or 0), \
                patch.object(guest.Path, "mkdir", lambda *args, **kwargs: None):
            self.assertEqual(0, guest.runner_rpc("runner-start " + "a" * 32))
        self.assertTrue((guest.ROOT / ("a" * 32 + ".runner")).exists())
        self.assertEqual(["bash", guest.RUNNER_LAUNCHER], started[0].command)
        self.assertEqual(valid, json.loads((guest.ROOT / ("a" * 32 + ".runner.json")).read_text()))

    def test_runner_scripts_parse(self):
        for script in ("runner.sh", "runner-hook.sh", "prepare-runner.sh", "pool-ref.sh"):
            subprocess.run(["bash", "-n", str(HERE / script)], check=True)


if __name__ == "__main__":
    unittest.main()
