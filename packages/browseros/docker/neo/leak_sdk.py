#!/usr/bin/env python3
"""End-to-end leak test of a claw-sdk Neo image: does a sensitive value ever leave Neo?

It starts a fresh container from the image, plants known values in every sensitive kind
the rule table covers, in every kind of frame, through the page's own script and through
the owner typing on the takeover display. It tries every way out (MCP `run`, the host
API, live pixels, the display, the clipboard, the trace store on disk, the logs), records
every byte that comes back, then judges the record into a table: channel x field kind ->
masked / leaked / n/a, with the exact leaking request for each leak. Leaks are reported,
never fixed here.

  python3 leak_sdk.py --image neo-leak:0.52.1.2-ec1ded9 [--out DIR] [--keep] [--scenario NAME]
  python3 leak_sdk.py --rejudge DIR          # judge saved evidence again, no Docker

Exit status is a bit set: 0 nothing left Neo; +1 a new leak; +4 a known residual (leaked
or held); +8 a verdict could not be proven; +16 the harness failed (the table is
incomplete). 2 is a usage error.

This file holds the scenario order (read it top to bottom) and the Session, which owns
every byte to and from the container. What to plant and how to probe is leak_plan.py;
what the record means is leak_judge.py; the protocol clients are leak_wire.py;
leak_inside.py and leak_fixture/ run inside the container. Standard library only.
"""

from __future__ import annotations

import argparse
import contextlib
import dataclasses
import io
import json
import secrets
import subprocess
import tarfile
import tempfile
import threading
import time
import traceback
import urllib.parse
from collections.abc import Callable, Collection, Iterator, Sequence
from pathlib import Path

from leak_judge import (
    Capture,
    Evidence,
    Facts,
    Proof,
    Request,
    Step,
    Stream,
    encode,
    instance_to_json,
    judge,
    live,
    render_json,
    render_markdown,
)
from leak_plan import (
    ACT_EVENT,
    ACT_EXPOSURE,
    AFTER_RELEASE_READS,
    ECHO_READS,
    FIELD_READS,
    GESTURE_ACTS,
    NAV_FRAMES,
    PAGE_ECHO_PROBES,
    PIXEL_READS,
    PLACES,
    RACE_KINDS,
    REGISTRABLE_BUDGET,
    SCRIPT_READS,
    SPECIAL_TAIL,
    SPECS,
    Act,
    ChannelId,
    Exposure,
    Instance,
    Kind,
    Layout,
    Location,
    Origin,
    Phase,
    PhaseGuard,
    Plant,
    Region,
    assert_clean,
    make_instance,
    plan_runs,
    registrable,
)
from leak_wire import (
    Agent,
    Display,
    Docker,
    Host,
    SseParser,
    StreamReader,
    parse_run_reply,
)

HERE = Path(__file__).resolve().parent
FIELD = frozenset({Exposure.FIELD, Exposure.PASTED})
ECHO = frozenset({Exposure.ECHO, Exposure.SERVER_ECHO})
RUN_TIMEOUT_MS = 60_000  # under smoke.McpClient's 120 s urllib timeout
SCREEN = (1600, 1000)  # NEO_SCREEN: a takeover fit to it never resizes the display
FX_F_SUBJECT = "every sealed-world read is masked with its frame's whole site"

# ------------------------------------------------------------------------------------------
# The scenarios, in the order they run. Each opens its own page instances and closes them.
# ------------------------------------------------------------------------------------------


def echoes(s: Session) -> None:
    """Every text read meets every echo cold, then warm; then a page that plants after
    Neo looked.

    A read registers what it sweeps for the rest of the process, so a cold result is real
    only on values nothing has swept. Each read therefore gets a page instance of its own
    and is its first touch; the host sweep then registers every frame's fields and the
    same read runs again warm. This holds whatever a read sweeps (its frame or its site).
    """
    for channel in ECHO_READS:
        page = s.open(Layout.ECHOES)
        s.read(page, channel, Phase.COLD, exposures=ECHO)
        s.sweep(page)
        s.read(page, channel, Phase.WARM, exposures=ECHO)
        s.close(page)
    # The page's own input events reach the agent-path journal only in frames Neo has
    # sealed, so the first read touches every frame while the late fields are empty.
    page = s.open(Layout.ECHOES, late=True)
    s.read(page, ChannelId.INPUT_VALUE)
    s.act(page, Act.LATE_PLANT)
    s.read(page, ChannelId.TEXT_CONTENT, Phase.JOURNALED, exposures=ECHO)
    s.close(page)


def fields(s: Session) -> None:
    """Every read of a field in every location; the show-password flip after Neo's seal
    watched it; and every way to run script or a built-in, which must refuse."""
    page = s.open(Layout.FIELDS)
    for channel in FIELD_READS:
        s.read(page, channel, exposures=FIELD)
    s.act(page, Act.SHOW_PASSWORD)
    for channel in FIELD_READS:
        s.read(page, channel, exposures=FIELD, kinds={Kind.SHOW_PASSWORD})
    for channel in SCRIPT_READS:
        s.read(page, channel)
    s.close(page)


def page_echoes(s: Session) -> None:
    """Values the page moves where only L2 can mask them: the title, the URL, an alert, a
    download's name, a popup's title. Each probe gets a fresh page, read cold, then warm."""
    for act, channel in PAGE_ECHO_PROBES:
        page = s.open(Layout.FACTS)
        exposures = {ACT_EXPOSURE[act]}
        for phase in (Phase.COLD, Phase.WARM):
            if act in ACT_EVENT:
                s.read(page, channel, phase, exposures=exposures, trigger=act)
            else:
                s.act(page, act)
                s.read(page, channel, phase, exposures=exposures)
            if phase is Phase.COLD:
                s.sweep(page)
        s.close(page)


def pixels(s: Session) -> None:
    """Pixels matter once a tab is private (L3). A page that is not private proves every
    pixel channel is live; then the host makes it private and every channel must stop."""
    page = s.open(Layout.PIXELS)
    with s.watch(page) as watch:
        watch.live()
        for channel in PIXEL_READS:
            s.read(page, channel)
        # X-Url is the only text a pixel stream carries: a value Neo has registered, put
        # in the URL by the page, must reach the screencast's part headers masked.
        s.sweep(page)
        s.act(page, Act.ECHO_URL)
        watch.live()
        s.make_private(page)
        watch.settle()
        watch.retry()
        for channel in PIXEL_READS:
            s.read(page, channel)
    s.close(page)


def takeover(s: Session) -> None:
    """The owner types every kind into every location over the takeover's control
    display, also into a frame the page adds during control and into two frames that
    navigate away before release, and copies a card number into a plain box. Then the
    agent reads after release."""
    page = s.open(Layout.OWNER)
    s.read(page, ChannelId.ARIA_AI)  # Neo's first touch seals every frame present now
    with s.watch(page) as watch:
        watch.live()
        with s.takeover(page) as owner:  # private, lock, views held, journal armed
            watch.settle()
            watch.retry()
            s.act(page, Act.ADD_LATE_FRAME)
            for plant in page.instance.owner_plants():
                owner.type(plant)
            owner.copy_paste(
                page.instance.plant(Kind.CC_NUMBER, Location.MAIN, origin=Origin.OWNER)
            )
            s.read(page, ChannelId.TEXT_CONTENT, Phase.DURING_CONTROL, exposures=ECHO)
            for plant in page.instance.owner_plants(navigating=True):
                if plant.location in NAV_FRAMES:
                    owner.type(plant, then_enter=True)
    for channel in AFTER_RELEASE_READS:
        s.read(page, channel, Phase.AFTER_RELEASE)
    s.close(page)


def control(s: Session) -> None:
    """A raw control display: no takeover and no lock, so the agent reads while the owner
    types, at once and after the journal settles. Its first input makes every open tab
    private and closes views; closing it sweeps every tab. A popup no agent op touched is
    typed into and navigates away."""
    page = s.open(Layout.OWNER)
    s.read(page, ChannelId.ARIA_AI)
    with s.watch(page) as watch:
        watch.live()
        with s.control(page) as owner:
            for plant in page.instance.owner_plants(kinds=RACE_KINDS):
                owner.type(plant)
                s.read(page, ChannelId.TEXT_CONTENT, Phase.CONTROL_RACE, exposures=ECHO)
            owner.settle()
            s.read(page, ChannelId.TEXT_CONTENT, Phase.CONTROL_SETTLED, exposures=ECHO)
            owner.open_popup()
            owner.type(
                page.instance.plant(Kind.PASSWORD, Location.POPUP), then_enter=True
            )
        watch.settle()
        watch.retry()
    for channel in AFTER_RELEASE_READS:
        s.read(page, channel, Phase.AFTER_RELEASE)
    s.close(page)


def host_secret(s: Session) -> None:
    """A value only the host knows, shown by the page with no field behind it. Cold it
    reads raw by design (L1 has nothing to sweep); once the host registers it, every exit
    masks it, and no host route ever hands it back."""
    page = s.open(Layout.HOST)
    s.read(page, ChannelId.TEXT_CONTENT, Phase.COLD)
    s.register_secret(page, page.instance.plant(origin=Origin.HOST))
    s.read(page, ChannelId.TEXT_CONTENT, Phase.WARM)
    s.read(page, ChannelId.CONTENT, Phase.WARM)
    s.close(page)


# The restart scenario is deferred: it checks R3 (the SecretSet is memory only), which
# is by design, and a restart is the least predictable step.
SCENARIOS: tuple[tuple[str, Callable[[Session], None]], ...] = (
    ("echoes", echoes),
    ("fields", fields),
    ("page_echoes", page_echoes),
    ("pixels", pixels),
    # control runs before takeover: after a takeover in which the owner typed, a raw control
    # display's keys never reach the page (measured on ec1ded9; a functional bug, reported).
    ("control", control),  # its first input makes every open tab private
    ("takeover", takeover),
    ("host_secret", host_secret),
)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="End-to-end leak test of a claw-sdk Neo image."
    )
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--image", help="image tag, e.g. neo-leak:0.52.1.2-ec1ded9")
    group.add_argument("--rejudge", type=Path, help="an evidence directory from a run")
    parser.add_argument(
        "--out", type=Path, help="evidence directory (default: a temp dir)"
    )
    parser.add_argument("--seed", help="value seed (default: the run's start time)")
    parser.add_argument(
        "--keep", action="store_true", help="leave the container running"
    )
    parser.add_argument(
        "--scenario", action="append", choices=[n for n, _ in SCENARIOS]
    )
    args = parser.parse_args(argv)
    if args.rejudge:
        out = args.rejudge
    else:
        stamp = time.strftime("%Y%m%d-%H%M%S")
        out = args.out or Path(tempfile.gettempdir()) / "leak-sdk" / stamp
        session = Session(args.image, out, seed=args.seed or stamp, keep=args.keep)
        try:
            session.start()
            for name, scenario in SCENARIOS:
                if not args.scenario or name in args.scenario:
                    session.scenario(name, scenario)
        except Exception:  # noqa: BLE001 - preflight failed; the evidence still judges
            session.step("failed", error=traceback.format_exc(limit=4))
        finally:
            session.finish()
    evidence = Evidence.load(out)
    judgement = judge(evidence)
    (out / "report.md").write_text(render_markdown(judgement, evidence))
    (out / "verdicts.json").write_text(render_json(judgement, evidence))
    print(f"{out / 'report.md'}\nexit {judgement.exit_code}")
    return judgement.exit_code


# ------------------------------------------------------------------------------------------
# The Session: the one object scenarios drive
# ------------------------------------------------------------------------------------------


@dataclasses.dataclass(frozen=True)
class Page:
    """One fixture instance in one tab: what scenarios pass around."""

    instance: Instance
    tab: int


class HarnessError(RuntimeError):
    """The harness could not do what a scenario asked (the oracle, focus, an answer it
    cannot use). The scenario stops; earlier evidence stands."""


class Session:
    """A fresh claw-sdk Neo container and every way the test talks to it, each recorded.

    Scenarios say what to do to a page; the Session makes it sound:
    - every agent read is bracketed by oracle proofs (presence at that moment) and
      checked by the PhaseGuard (a cold read is the first touch of fresh values);
    - every request passes assert_clean before it leaves (no value, no anchor);
    - every answer's bytes land in the evidence directory before anything parses them;
    - owner input goes through the display only, and each typed value is proven.

    Threads: the events reader runs for the whole session, and each watch() or display
    adds readers. Each reader owns its blob; only the main thread writes log.jsonl.
    """

    def __init__(self, image: str, out: Path, *, seed: str, keep: bool) -> None:
        self.image, self.out, self.seed, self.keep = image, out, seed, keep
        for sub in ("blobs", "streams", "disk"):
            (out / sub).mkdir(parents=True, exist_ok=True)
        self.docker = Docker(f"leak-{time.strftime('%H%M%S')}-{secrets.token_hex(2)}")
        self.token = "leak-" + secrets.token_hex(24)
        self.token_file = out / "host-token"
        self.guard = PhaseGuard()
        self.instances: dict[str, Instance] = {}
        self.pages: list[Page] = []
        self.counter = 0
        self.scenario_name = "preflight"
        self.registrable = 0
        # Instances with a tab that went private: every screenshot of their operations
        # is fetched, and the judge holds each against the privacy start.
        self.private_instances: set[str] = set()
        # Instances whose page opened a popup. claw-sdk makes a task's last tab the run's
        # `page`, and a popup joins its opener's task, so from then on each read of the
        # instance first points `page` back at the instance's main tab.
        self.popped: set[str] = set()
        self.oracle: Oracle | None = None
        self.events: StreamReader | None = None
        self._write_manifest()

    # -- evidence ----------------------------------------------------------------------

    def _id(self, prefix: str) -> str:
        self.counter += 1
        return f"{prefix}{self.counter:05}"

    def record(self, item: Capture | Proof | Step | Stream | Facts) -> None:
        with (self.out / "log.jsonl").open("a") as log:
            log.write(encode(item) + "\n")

    def blob(self, data: bytes, ident: str) -> str:
        path = f"blobs/{ident}.bin"
        (self.out / path).write_bytes(data)
        return path

    def step(self, action: str, page: Page | None = None, **detail: object) -> None:
        self.record(
            Step(
                now(),
                self.scenario_name,
                page.instance.name if page else None,
                action,
                detail,
            )
        )

    def _write_manifest(self) -> None:
        manifest = {
            "seed": self.seed,
            "image": self.image,
            "instances": [instance_to_json(i) for i in self.instances.values()],
        }
        (self.out / "manifest.json").write_text(json.dumps(manifest))

    # -- the container -----------------------------------------------------------------

    def start(self) -> None:
        """Run the container, start leak_inside, open the events stream, and preflight.
        CLAW_LOG stays unset, so the logs are judged as shipped."""
        self.token_file.write_text(self.token)
        self.token_file.chmod(0o644)
        self.docker.run(self.image, self.token_file)
        self.docker.wait_healthy()
        self.docker.exec("mkdir", "-p", "/tmp/leakfx")
        self.docker.cp_in(HERE / "leak_inside.py", "/tmp/leakfx/leak_inside.py")
        self.docker.cp_in(HERE / "leak_fixture", "/tmp/leakfx/leak_fixture")
        self.oracle = Oracle(
            self.docker.exec_pipe("python3", "-u", "/tmp/leakfx/leak_inside.py")
        )
        self.agent = Agent(f"http://127.0.0.1:{self.docker.port(9010)}/mcp")
        self.host = Host(self.docker.port(9300), self.token)
        self.events = self.host.stream("/v1/events", self.out / "streams/events.bin")
        self.preflight()

    def preflight(self) -> None:
        """Every check the table depends on, recorded in Facts; a failed one raises."""
        checks: dict[str, str] = {}
        tools = [t["name"] for t in self.agent.client.tools()]
        checks["MCP lists exactly run"] = (
            f"{'ok' if tools == ['run'] else 'FAIL'} {tools}"
        )
        ok, bad = (
            self.host.call("GET", "/v1/runs?limit=1", token=t).status
            for t in (True, False)
        )
        checks["host API 200 with the token, 401 without"] = (
            f"{'ok' if (ok, bad) == (200, 401) else 'FAIL'} {ok}/{bad}"
        )
        display = subprocess.run(
            ["docker", "exec", "-i", self.docker.name, "python3", "-", "claw-sdk"],
            input=(HERE / "check_display.py").read_bytes(),
            capture_output=True,
        )
        clipboard = display.stdout.decode().strip().replace("\n", "; ")
        checks["check_display.py claw-sdk"] = (
            f"{'ok' if display.returncode == 0 else 'FAIL'}"
        )
        claw_log = (
            self.docker.exec("printenv", "CLAW_LOG", check=False).decode().strip()
        )
        checks["CLAW_LOG"] = f"{'ok unset' if not claw_log else 'FAIL ' + claw_log}"
        # The oracle is a second DevTools client: an agent read must answer the same
        # before and after it first attaches, or its presence would perturb what it proves.
        page = self.open(Layout.FIELDS, prove=False)
        first = self._raw_read(page, ChannelId.INPUT_VALUE)
        assert self.oracle is not None
        self.wait_present(page)
        second = self._raw_read(page, ChannelId.INPUT_VALUE)
        checks[
            "an agent read answers the same before and after the oracle attaches"
        ] = f"{'ok' if first == second else 'FAIL'}"
        targets = self.oracle.call("targets")["targets"]
        oopif = {
            str(doc): any(
                t["type"] == "iframe" and t["url"].startswith(page.instance.url(doc))
                for t in targets
            )
            for doc in page.instance.documents()
            if doc is not Location.MAIN
        }
        for doc, seen in oopif.items():
            if seen is not PLACES[Location(doc)].expect_oopif:
                checks[f"{doc} is {'an OOPIF' if seen else 'in process'}"] = (
                    "FAIL: label lies"
                )
        self.close(page)
        t0 = now()
        neo = int(self.oracle.call("clock")["ms"])
        offset = neo - (t0 + now()) // 2
        revision = self.docker.label("org.opencontainers.image.revision")
        self.record(
            Facts(
                image=self.image,
                revision=revision,
                fx_f_in_revision=_git_has(revision, FX_F_SUBJECT),
                oopif=oopif,
                checks=checks,
                clipboard_flags=clipboard,
                clock_offset_ms=offset,
                tools={t: _version(t) for t in ("tesseract", "ffmpeg")},
            )
        )
        failed = [k for k, v in checks.items() if "FAIL" in v]
        if failed:
            raise HarnessError(f"preflight failed: {failed}")

    def scenario(self, name: str, run: Callable[[Session], None]) -> None:
        """Run one scenario. A raise becomes a "failed" Step and closes the scenario's
        tabs, so the next scenario starts clean and the judge knows the table is partial."""
        self.scenario_name = name
        self.step("scenario")
        try:
            run(self)
        except Exception:  # noqa: BLE001 - recorded; the next scenario runs
            self.step("failed", error=traceback.format_exc(limit=6))
            for page in list(self.pages):
                with contextlib.suppress(Exception):
                    self.close(page)

    def finish(self) -> None:
        """Collect every host record, the logs and two copies of the state dir, then
        remove the container (unless --keep). Runs even after a failed start."""
        self.scenario_name = "finish"
        if self.events is not None:
            self._collect("host records and the events replay", self.collect_records)
            self.events.stop()
            self.record(
                self._stream(
                    self.events, ChannelId.EVENTS, None, None, "GET /v1/events"
                )
            )
        if self.oracle is not None:
            self._collect("profile scan", self._profile_scan)
            self.oracle.close()
        self._collect("disk copy (live)", lambda: self._disk("live"))
        self._collect(
            "docker logs",
            lambda: (self.out / "docker.log").write_bytes(self.docker.logs()),
        )
        if not self.keep:
            # The stopped copy is the graceful stop's checkpoint: one collection.
            self._collect("disk copy (stopped)", self._stop_and_copy)
            with contextlib.suppress(Exception):
                self.docker.remove()
            self.token_file.unlink(missing_ok=True)
        self._write_manifest()

    def _collect(self, name: str, collect: Callable[[], object]) -> None:
        """One evidence collection. A raise is a failed Step: the judge sets +16, since
        the table lacks that channel's bytes, and finish() goes on to the next
        collection and the cleanup rather than losing them too."""
        try:
            collect()
        except Exception:  # noqa: BLE001 - recorded as the channel missing
            self.step("failed", error=f"{name}: {traceback.format_exc(limit=4)}")

    def _disk(self, label: str) -> None:
        """A live tar of /data/browserclaw: values in WAL frames and the -shm file."""
        data = self.docker.exec("tar", "-C", "/data/browserclaw", "-cf", "-", ".")
        with tarfile.open(fileobj=io.BytesIO(data)) as tar:
            tar.extractall(self.out / "disk" / label, filter="data")

    def _stop_and_copy(self) -> None:
        """A copy after a graceful stop: values checkpointed into trace.db's pages."""
        self.docker.stop()
        self.docker.cp_out("/data/browserclaw", self.out / "disk" / "stopped")

    def _profile_scan(self) -> None:
        assert self.oracle is not None
        # Long values only: short ones would match any digit run in a binary file.
        plants = [
            p
            for i in self.instances.values()
            for p in i.plants
            if p.kind is not Kind.OTP_SPLIT and len(p.secret) >= 8
        ]
        needles = [p.secret.reveal().removesuffix(SPECIAL_TAIL) for p in plants]
        hits = self.oracle.call("grep", root="/data/profile", needles=needles)["hits"]
        self.step(
            "profile_scan",
            hits=[{"file": h["file"], "plant": plants[h["needle"]].id} for h in hits],
        )

    # -- pages -------------------------------------------------------------------------

    def open(self, layout: Layout, *, late: bool = False, prove: bool = True) -> Page:
        """A fresh instance in a new tab, every page-planted region proven present.

        goto reads nothing, so the instance stays cold."""
        assert self.oracle is not None
        index = len(self.instances) + 1
        instance = make_instance(layout, seed=self.seed, index=index, late=late)
        self.registrable += sum(
            registrable(p, typed=p.origin is Origin.OWNER) for p in instance.plants
        )
        if self.registrable > REGISTRABLE_BUDGET:
            raise HarnessError(
                f"{self.registrable} registrable values: the SecretSet could evict"
            )
        self.instances[instance.name] = instance
        self._write_manifest()
        self.oracle.call(
            "put",
            instance=instance.name,
            slices={str(d): instance.slice(d) for d in instance.documents()},
            echoes=instance.server_echoes(),
        )
        capture = self._run(
            instance,
            ChannelId.HARNESS,
            Phase.ANY,
            f"await page.goto({json.dumps(instance.url())}); return 1;",
        )
        tab = self._tab_of(capture.run_id)
        page = Page(instance, tab)
        self.pages.append(page)
        self.step("open", page, tab=tab, layout=str(layout))
        if prove:
            self.wait_present(page)
        return page

    def wait_present(self, page: Page, timeout: float = 15) -> str:
        """Proofs until every region the page plants itself shows its value."""
        want = [
            r
            for r in page.instance.regions
            if r.plant.origin in (Origin.PAGE, Origin.HOST)
            and not r.plant.late
            and r.exposure in (Exposure.FIELD, Exposure.ECHO)
        ]
        deadline = time.monotonic() + timeout
        while True:
            proof = self.proof(page)
            missing = [r.id for r in want if not live(proof, r)]
            if not missing:
                return proof.id
            if time.monotonic() > deadline:
                raise HarnessError(
                    f"{page.instance.name}: never present: {missing[:6]}"
                )
            time.sleep(0.3)

    def close(self, page: Page) -> None:
        """Every tab of the instance (popups too). Idempotent."""
        mine = json.dumps(f"/fx/{page.instance.name}/")
        code = (
            f"for (const p of context.pages()) if (p.url().includes({mine})) await p.close();\n"
            "return 1;"
        )
        with contextlib.suppress(Exception):
            self._run(page.instance, ChannelId.HARNESS, Phase.ANY, code, task=False)
        self.step("close", page)
        if page in self.pages:
            self.pages.remove(page)

    def act(self, page: Page, act: Act) -> None:
        """The page's own script does `act` now, with a user gesture when it needs one."""
        assert self.oracle is not None
        self.oracle.call(
            "act",
            instance=page.instance.name,
            name=str(act),
            gesture=act in GESTURE_ACTS,
        )
        self.step("act", page, act=str(act))
        if act is Act.POPUP:
            self.popped.add(page.instance.name)
        if act is Act.LATE_PLANT:
            self.guard.note_late_plant(page.instance)

    def proof(self, page: Page) -> Proof:
        assert self.oracle is not None
        state = self.oracle.call("state", instance=page.instance.name)
        proof = Proof(
            self._id("p"),
            page.instance.name,
            int(state["at"]),
            {k: tuple(v) if v else None for k, v in state["regions"].items()},
            tuple(state["documents"]),
        )
        self.record(proof)
        return proof

    # -- the agent ---------------------------------------------------------------------

    def read(
        self,
        page: Page,
        channel: ChannelId,
        phase: Phase = Phase.ANY,
        *,
        exposures: Collection[Exposure] | None = None,
        kinds: Collection[Kind] | None = None,
        trigger: Act | None = None,
    ) -> None:
        """Exercise one agent channel on the page and record what came back: per planned
        run, a proof, the run, a proof. A COLD read is followed at once by a repeat of its
        requestId, whose stored reply must be what the first call got."""
        runs = plan_runs(
            SPECS[channel], page.instance, exposures=exposures, kinds=kinds
        )
        self.guard.claim(page.instance, phase, len(runs))
        for planned in runs:
            request_id = f"{page.instance.name}.{self._id('q')}"
            before = self.proof(page)
            capture = self._run(
                page.instance,
                channel,
                phase,
                planned.code,
                trigger=trigger,
                page=page,
                request_id=request_id,
            )
            after = self.proof(page)
            capture = dataclasses.replace(
                capture,
                proofs=(before.id, after.id),
                regions=tuple(r.id for r in planned.regions),
            )
            self.record(capture)
            if phase is Phase.COLD:
                again = self._run(
                    page.instance,
                    ChannelId.REPLAY,
                    phase,
                    planned.code,
                    request_id=request_id,
                    replay_of=capture.id,
                )
                self.record(again)
        self.guard.note_touched(page.instance)

    def _raw_read(self, page: Page, channel: ChannelId) -> object:
        planned = plan_runs(SPECS[channel], page.instance, exposures=FIELD)[0]
        capture = self._run(page.instance, channel, Phase.ANY, planned.code)
        self.record(
            dataclasses.replace(capture, regions=tuple(r.id for r in planned.regions))
        )
        return parse_run_reply((self.out / capture.blob).read_bytes()).value

    def _run(
        self,
        instance: Instance,
        channel: ChannelId,
        phase: Phase,
        code: str,
        *,
        trigger: Act | None = None,
        page: Page | None = None,
        task: bool = True,
        request_id: str | None = None,
        replay_of: str | None = None,
    ) -> Capture:
        """One MCP run, its every byte kept. Harness runs are recorded here; reads are
        recorded by read() once their closing proof exists."""
        ident = self._id("c")
        if instance.name in self.popped and channel is not ChannelId.HARNESS:
            main = json.dumps(f"/fx/{instance.name}/main")
            code = (
                f"page = context.pages().find((p) => p.url().includes({main})) ?? page;\n"
                + code
            )
        meta = {"requestId": request_id or f"{instance.name}.{ident}"}
        if task:
            meta["taskId"] = instance.name
        assert_clean(json.dumps({"code": code, "meta": meta}), self.instances.values())
        started = now()
        if trigger is None:
            body, raw = self.agent.run(code, meta, RUN_TIMEOUT_MS)
        else:
            body, raw = self._triggered(instance, page, trigger, code, meta)
        reply_meta = _run_meta(raw)
        capture = Capture(
            ident,
            self.scenario_name,
            instance.name,
            channel,
            phase,
            Request("mcp", "tools/call run", "/mcp", body),
            self.blob(raw, ident),
            started,
            now(),
            run_id=reply_meta.get("id"),
            trigger=str(trigger) if trigger else None,
            replay_of=replay_of,
        )
        if channel is ChannelId.HARNESS:
            self.record(capture)
        return capture

    def _triggered(
        self, instance: Instance, page: Page | None, trigger: Act, code: str, meta: dict
    ) -> tuple[str, bytes]:
        """Start the run, wait until its events.wait operation is listening (seen on
        /v1/events), then let the page act; the run returns with the event."""
        assert self.events is not None and page is not None
        mark = len(self.events.data())
        result: list[tuple[str, bytes]] = []
        worker = threading.Thread(
            target=lambda: result.append(self.agent.run(code, meta, RUN_TIMEOUT_MS))
        )
        worker.start()
        needle = f'"taskId":"{instance.name}"'

        def listening(data: bytes) -> bool:
            for event in SseParser().feed(data[mark:]):
                if event.type == "operation.started" and needle in event.data:
                    if '"name":"events.wait"' in event.data:
                        return True
            return False

        if not self.events.wait_bytes(listening, timeout=15):
            self.step("trigger_unseen", page, act=str(trigger))
        self.act(page, trigger)
        worker.join(timeout=RUN_TIMEOUT_MS / 1000 + 15)
        if not result:
            raise HarnessError(f"{instance.name}: the {trigger} run never returned")
        return result[0]

    def _tab_of(self, run_id: str | None) -> int:
        answer = self._host(
            ChannelId.RUN, "GET", f"/v1/runs/{run_id}?wait=10", run_id=run_id
        )
        operations = json.loads(answer.body).get("operations", [])
        tabs = [op["tabId"] for op in operations if op.get("tabId")]
        if not tabs:
            raise HarnessError(f"run {run_id} names no tab")
        return int(tabs[-1])

    # -- host parts --------------------------------------------------------------------

    def _host(
        self,
        channel: ChannelId,
        method: str,
        path: str,
        body: dict | None = None,
        *,
        page: Page | None = None,
        run_id: str | None = None,
        inject: Plant | None = None,
        task: str | None = None,
    ):
        """One host request, recorded as a capture of `channel`. Its instance is the
        page's, the given task's, or the task the answer names."""
        text = json.dumps(body) if body is not None else None
        assert_clean(f"{path} {text or ''}", self.instances.values(), inject=inject)
        ident = self._id("h")
        started = now()
        answer = self.host.call(method, path, body)
        self.record(
            Capture(
                ident,
                self.scenario_name,
                page.instance.name if page else task or _task_of(answer.body),
                channel,
                Phase.ANY,
                Request("http", method, path, text, inject.id if inject else None),
                self.blob(answer.body, ident),
                started,
                now(),
                status=answer.status,
                run_id=run_id,
            )
        )
        return answer

    def sweep(self, page: Page) -> None:
        """POST /v1/tabs/{tab}/sweep: every frame's fields into the SecretSet. The answer
        must be a count only."""
        answer = self._host(
            ChannelId.HOST_REPLIES, "POST", f"/v1/tabs/{page.tab}/sweep", page=page
        )
        self.step(
            "sweep", page, status=answer.status, answer=answer.body.decode()[:200]
        )
        self.guard.note_registered(page.instance)

    def make_private(self, page: Page) -> None:
        answer = self._host(
            ChannelId.HOST_REPLIES,
            "POST",
            f"/v1/tabs/{page.tab}/private",
            {},
            page=page,
        )
        self.private_instances.add(page.instance.name)
        self.step("private", page, tab=page.tab, status=answer.status)

    def register_secret(self, page: Page, plant: Plant) -> None:
        """POST /v1/secrets: a designed injection (Request.inject)."""
        body = {"value": plant.secret.reveal(), "ttlSeconds": 3600}
        answer = self._host(
            ChannelId.HOST_REPLIES, "POST", "/v1/secrets", body, page=page, inject=plant
        )
        self.step("secret", page, plant=plant.id, status=answer.status)
        self.guard.note_registered(page.instance)

    @contextlib.contextmanager
    def watch(self, page: Page) -> Iterator[Watch]:
        """A view display that follows the tab (which also keeps it in front, so it
        paints), the screencast and the video, each read by its own thread."""
        watch = Watch(self, page)
        try:
            watch.open()
            yield watch
        finally:
            watch.close()

    @contextlib.contextmanager
    def takeover(self, page: Page) -> Iterator[Owner]:
        body = {
            "tabId": page.tab,
            "viewer": {"width": SCREEN[0], "height": SCREEN[1]},
            "reads": "block",
            "ttlSeconds": 300,
        }
        answer = self._host(
            ChannelId.HOST_REPLIES, "POST", "/v1/takeovers", body, page=page
        )
        if answer.status != 200:
            # Whole, not cut: the report redacts every failure, and a cut could leave
            # less of a value than the redactor recognises.
            body = answer.body.decode(errors="replace")
            raise HarnessError(f"takeover refused: {answer.status} {body}")
        takeover = json.loads(answer.body)
        self.private_instances.add(page.instance.name)
        self.step("takeover", page, id=takeover["id"], tab=page.tab)
        self.guard.note_control("takeover", page.instance)
        owner = Owner(self, page, takeover["display"], "control")
        try:
            yield owner
        finally:
            self._host(
                ChannelId.HOST_REPLIES,
                "POST",
                f"/v1/takeovers/{takeover['id']}/release",
                {"reason": "done"},
                page=page,
            )
            self.step("release", page, id=takeover["id"])
            self.guard.note_control(None)
            owner.close()

    @contextlib.contextmanager
    def control(self, page: Page) -> Iterator[Owner]:
        """/v1/display?mode=control with no takeover. Its first input makes every open tab
        private, so no tab of another instance may be open."""
        assert self.oracle is not None
        others = [
            t["url"]
            for t in self.oracle.call("targets")["targets"]
            if t["type"] == "page"
            and "/fx/" in t["url"]
            and f"/fx/{page.instance.name}/" not in t["url"]
        ]
        if others:
            raise HarnessError(f"other instances still open: {others}")
        self.guard.note_control("display", page.instance)
        owner = Owner(self, page, "/v1/display?mode=control", "control")
        self.step("control_open", page)
        try:
            yield owner
        finally:
            owner.close()
            self.step("control_close", page)
            self.guard.note_control(None)
            time.sleep(2)  # closing the display sweeps every tab; let it finish

    def collect_records(self) -> None:
        """Every host record, read after every run is complete. Records are masked once,
        when written, and events frozen at publish, so reading late sees what a host
        reading at once would have seen."""
        for _ in range(30):
            running = json.loads(self.host.call("GET", "/v1/runs?status=running").body)
            if not running.get("items"):
                break
            time.sleep(1)
        runs: list[dict] = []
        cursor = ""
        while True:
            answer = self._host(ChannelId.RUNS, "GET", f"/v1/runs?limit=100{cursor}")
            page = json.loads(answer.body)
            runs += page.get("items", [])
            if not page.get("nextCursor"):
                break
            cursor = f"&cursor={urllib.parse.quote(page['nextCursor'])}"
        for run in runs:
            answer = self._host(
                ChannelId.RUN,
                "GET",
                f"/v1/runs/{run['id']}?include=screenshots",
                run_id=run["id"],
            )
            for op in json.loads(answer.body).get("operations", []):
                self._host(
                    ChannelId.OPERATION,
                    "GET",
                    f"/v1/operations/{op['id']}",
                    run_id=run["id"],
                )
                if run.get("taskId") in self.private_instances:
                    self._host(
                        ChannelId.OP_SCREENSHOT,
                        "GET",
                        f"/v1/operations/{op['id']}/screenshot",
                        run_id=op["id"],
                        task=run.get("taskId"),
                    )
        for name in self.instances:
            self._host(
                ChannelId.TRACE,
                "GET",
                f"/v1/trace?taskId={name}&tail=1000&screenshots=all",
            )
        self._host(ChannelId.TRACE, "GET", "/v1/trace?tail=1000")
        # Neo keeps the last 1024 events; an id older than that answers stream.reset. So
        # the replay starts inside the ring, from the live stream's last id.
        assert self.events is not None
        last = [e.id for e in SseParser().feed(self.events.data()) if e.id][-1]
        boot, _, seq = last.rpartition("-")
        replay = self.host.stream(
            "/v1/events",
            self.out / "streams/events-replay.bin",
            headers={"Last-Event-ID": f"{boot}-{max(0, int(seq) - 1000)}"},
        )
        size = -1
        while size != len(replay.data()):
            size = len(replay.data())
            time.sleep(2)
        replay.stop()
        self.record(
            self._stream(
                replay,
                ChannelId.EVENTS_REPLAY,
                None,
                None,
                "GET /v1/events (Last-Event-ID)",
            )
        )

    def _stream(
        self,
        reader: StreamReader | Display,
        channel: ChannelId,
        page: Page | None,
        tab: int | None,
        target: str,
        role: str = "",
    ) -> Stream:
        geometry = None
        if isinstance(reader, Display) and reader.status == 101:
            geometry = (reader.width, reader.height, reader.pixel_format.hex())
        return Stream(
            self._id("s"),
            self.scenario_name,
            channel,
            page.instance.name if page else None,
            tab,
            Request("ws" if isinstance(reader, Display) else "http", "GET", target),
            str(reader.blob.relative_to(self.out)),
            reader.opened_ms,
            tuple(reader.chunks),
            reader.closed,
            reader.status,
            role,
            geometry,
        )


class Oracle:
    """leak_inside.py over `docker exec -i`: JSON lines, one request at a time. A failed
    or malformed reply raises HarnessError, so a probe is never recorded as proven."""

    def __init__(self, process: subprocess.Popen[bytes]) -> None:
        self.process = process
        self.next = 0
        ready = self._line()
        if not ready.get("ready"):
            raise HarnessError(f"leak_inside did not start: {ready}")

    def _line(self) -> dict:
        assert self.process.stdout is not None
        line = self.process.stdout.readline()
        if not line:
            raise HarnessError("leak_inside exited")
        return json.loads(line)

    def call(self, op: str, **params: object) -> dict:
        assert self.process.stdin is not None
        self.next += 1
        self.process.stdin.write(
            json.dumps({"id": self.next, "op": op, "params": params}).encode() + b"\n"
        )
        self.process.stdin.flush()
        reply = self._line()
        if reply.get("id") != self.next or not reply.get("ok"):
            raise HarnessError(f"oracle {op}: {reply.get('error', reply)}")
        return reply

    def close(self) -> None:
        with contextlib.suppress(Exception):
            assert self.process.stdin is not None
            self.process.stdin.close()
            self.process.wait(timeout=10)


class Watch:
    """The live pixel channels of one tab during a scenario."""

    def __init__(self, session: Session, page: Page) -> None:
        self.s, self.page = session, page
        self.readers: list[tuple[StreamReader | Display, ChannelId, str, str]] = []

    def _paths(self, name: str) -> Path:
        return (
            self.s.out
            / "streams"
            / f"{self.page.instance.name}-{name}-{len(self.readers)}.bin"
        )

    def _start(self, role: str) -> None:
        tab, host, s = self.page.tab, self.s.host, self.s
        for channel, path in (
            (ChannelId.SCREENCAST, f"/v1/tabs/{tab}/screencast"),
            (ChannelId.VIDEO, f"/v1/tabs/{tab}/video"),
        ):
            reader = host.stream(path, self._paths(channel.name.lower()))
            self.readers.append((reader, channel, path, role))
        view = f"/v1/display?mode=view&followTab={tab}"
        display = Display(
            s.docker.port(9300), view, s.token, self._paths("view"), watch=True
        )
        self.readers.append((display, ChannelId.DISPLAY_VIEW, view, role))

    def open(self) -> None:
        self._start("view")

    def live(self, timeout: float = 15) -> None:
        """Every open stream delivered a frame since now: the positive control that lets
        "no frame after private" mean masked, and proof a frame saw the page as is."""
        marks = [self._frames(r) for r, *_ in self.readers]
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            fresh = [
                self._frames(r) > m
                for (r, *_), m in zip(self.readers, marks, strict=True)
            ]
            if all(fresh):
                break
            time.sleep(0.2)
        self.s.step(
            "live",
            self.page,
            fresh=[
                str(c) for (r, c, *_), f in zip(self.readers, fresh, strict=True) if f
            ],
        )

    @staticmethod
    def _frames(reader: StreamReader | Display) -> int:
        if isinstance(reader, Display):
            return sum(1 for m in reader.messages() if m.type == 0)
        data = reader.data()
        return data.count(b"--frame") + data.count(b"moof")

    def settle(self, timeout: float = 6) -> None:
        """Wait until every stream ended or the bound passed; record which."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and any(
            r.closed is None for r, *_ in self.readers
        ):
            time.sleep(0.2)
        self.s.step(
            "settled",
            self.page,
            open=[str(c) for r, c, *_ in self.readers if r.closed is None],
        )

    def retry(self) -> None:
        """Ask for a new screencast, video and view display: each must be refused."""
        self._start("retry")
        time.sleep(3)

    def close(self) -> None:
        for reader, channel, path, role in self.readers:
            reader.stop()
            self.s.record(
                self.s._stream(reader, channel, self.page, self.page.tab, path, role)
            )
        self.readers.clear()


class Owner:
    """The owner at a control display: keystrokes over RFB, each result proven by the
    oracle. The oracle focuses a field the way the owner's click would; the keys go over
    the display only, so Neo sees exactly what an owner's typing produces."""

    def __init__(self, session: Session, page: Page, path: str, role: str) -> None:
        self.s, self.page, self.path, self.role = session, page, path, role
        blob = session.out / "streams" / f"{page.instance.name}-control.bin"
        self.display = Display(
            session.docker.port(9300), path, session.token, blob, watch=False
        )
        if self.display.status != 101:
            raise HarnessError(f"control display refused: {self.display.status}")
        self.first_key = True

    def _keys(self, text: str) -> None:
        if self.first_key and "release=" not in self.path:
            self.s.private_instances.update(p.instance.name for p in self.s.pages)
            self.s.step("control_input", self.page)
        self.first_key = False
        self.display.type_text(text)

    def _focus(self, region: Region) -> None:
        assert self.s.oracle is not None
        tab = "popup" if region.location is Location.POPUP else "main"
        reply = self.s.oracle.call(
            "focus",
            instance=self.page.instance.name,
            region=region.id,
            path=list(PLACES[region.location].frame_path),
            tab=tab,
        )
        if not reply.get("focused"):
            raise HarnessError(f"focus missed {region.id}: {reply}")
        time.sleep(0.1)

    def type(self, plant: Plant, *, then_enter: bool = False) -> None:
        """Each box of the plant: focus, the keys, a proof that the field holds them.
        then_enter: Return, then wait until the next document shows the server echo."""
        first = now()
        boxes = self.page.instance.field_regions(plant)
        for region in boxes:
            self._focus(region)
            self._keys(region.expected())
        proven = self._wait(boxes)
        if then_enter:
            self.display.press(0xFF0D)
            echo = [
                r
                for r in self.page.instance.regions_of(plant)
                if r.exposure is Exposure.SERVER_ECHO
            ]
            proven = self._wait(echo)
        self.s.step(
            "typed",
            self.page,
            plant=plant.id,
            first_ms=first,
            last_ms=now(),
            proven=proven,
        )
        if not proven:
            raise HarnessError(f"{plant.id}: the typed value never showed")

    def _wait(self, regions: Sequence[Region], timeout: float = 6) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            proof = self.s.proof(self.page)
            if all(live(proof, r) for r in regions):
                return True
            time.sleep(0.3)
        return False

    def copy_paste(self, plant: Plant) -> None:
        """Select all and copy in the plant's field, paste into the plain box; the paste
        box holding the value is the clipboard's positive control."""
        (field,) = self.page.instance.field_regions(plant)
        (box,) = [
            r
            for r in self.page.instance.regions_of(plant)
            if r.exposure is Exposure.PASTED
        ]
        control, keys = 0xFFE3, (0x61, 0x63, 0x76)  # Control_L; a, c, v
        self._focus(field)
        self.display.chord(control, keys[0])
        self.display.chord(control, keys[1])
        self.s.step("copy", self.page, region=field.id)
        self._focus(box)
        self.display.chord(control, keys[2])
        self.s.step("paste", self.page, region=box.id, proven=self._wait([box]))

    def open_popup(self) -> None:
        """The owner presses Enter on the page's button, which opens the popup tab. On
        Neo's display only the owner's own input brings a new tab to the front (CDP
        activation and tab-switch keys are ignored), and the keys go to the front tab."""
        assert self.s.oracle is not None
        name = self.page.instance.name
        self.s.oracle.call(
            "focus", instance=name, region="fx-open-popup", path=[], tab="main"
        )
        self.display.press(0xFF0D)
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            targets = self.s.oracle.call("targets")["targets"]
            if any(f"/fx/{name}/popup" in t["url"] for t in targets):
                break
            time.sleep(0.2)
        else:
            raise HarnessError(f"{name}: the popup never opened")
        self.s.popped.add(name)
        self.s.step("popup_opened", self.page)

    def settle(self) -> None:
        """Wait out the takeover journal's 1 s debounce."""
        time.sleep(1.5)
        self.s.step("settled", self.page)

    def close(self) -> None:
        self.display.stop()
        self.s.record(
            self.s._stream(
                self.display,
                ChannelId.DISPLAY_VIEW,
                self.page,
                self.page.tab,
                self.path,
                self.role,
            )
        )


def now() -> int:
    return int(time.time() * 1000)


def _run_meta(raw: bytes) -> dict:
    text = raw.decode("utf-8", "replace")
    for chunk in [
        text,
        *(line[5:] for line in text.splitlines() if line.startswith("data:")),
    ]:
        with contextlib.suppress(ValueError, AttributeError):
            return json.loads(chunk)["result"]["_meta"]["com.browseros.neo/run"]
    return {}


def _task_of(body: bytes) -> str | None:
    with contextlib.suppress(ValueError, AttributeError, TypeError):
        return json.loads(body).get("taskId")
    return None


def _git_has(revision: str | None, subject: str) -> bool | None:
    """Whether the image's revision holds a commit with this subject (matched by subject:
    the fix exists under more than one sha), when the revision is in the local repo."""
    if not revision:
        return None
    found = subprocess.run(
        ["git", "-C", str(HERE), "log", "--format=%s", "-n", "400", revision],
        capture_output=True,
        text=True,
    )
    return subject in found.stdout if found.returncode == 0 else None


def _version(tool: str) -> str | None:
    with contextlib.suppress(OSError):
        out = subprocess.run(
            [tool, "-version" if tool == "ffmpeg" else "--version"],
            capture_output=True,
            text=True,
        )
        return (out.stdout or out.stderr).splitlines()[0][:60]
    return None


if __name__ == "__main__":
    raise SystemExit(main())
